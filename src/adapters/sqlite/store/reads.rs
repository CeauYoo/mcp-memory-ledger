//! Scoped record reads, deterministic ordering, and provenance loading.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{QueryBuilder, Row, Sqlite, sqlite::SqlitePool};

use crate::{
    domain::{
        claim::ClaimReference,
        commitment::Commitment,
        event::{EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        identity_core::IdentityCore,
        reflection::ReflectionIdentityUpdate,
        types::{Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimReadRecord, ClaimRecordQuery, ClaimReflectionHistoryPage, ClaimReflectionHistoryQuery,
        ClaimReflectionHistoryRecord, ClaimRevisionLinks, EpisodeReadRecord, EpisodeRecordQuery,
        EventReadRecord, EventRecordQuery, EvidenceQuery, MAX_EVENT_RECORD_QUERY_LIMIT,
        MemoryReadStore, ReflectionProvenanceLinks, ReflectionReadRecord, ReflectionRecordQuery,
        ScopedEventIdQuery, SelfModelHistoryKind, SelfModelHistoryPage, SelfModelHistoryQuery,
        SelfModelHistoryRecord,
    },
};

use super::rows::{
    deserialize_json, event_kind_as_str, map_sqlite, mode_as_str, owner_as_str, parse_owner,
    parse_timestamp, stored_claim_from_row, stored_event_from_row, utc_timestamp_sort_key,
};

use super::SqliteStore;

// A union applies its final timestamp/type/id order before every per-type
// LIMIT. Standalone browse methods retain their established ordering.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ReadOrdering {
    Standalone,
    Union,
}

impl SqliteStore {
    async fn query_event_records_ordered(
        &self,
        query: EventRecordQuery,
        ordering: ReadOrdering,
    ) -> Result<Vec<EventReadRecord>, AppError> {
        if !query.scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "event record query requires an explicit namespace".to_string(),
            ));
        }
        if query.limit == 0 {
            return Err(AppError::InvalidParams(
                "event record query limit must be at least 1".to_string(),
            ));
        }
        if query.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "event record query limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }
        if query
            .recorded_after
            .zip(query.recorded_before)
            .is_some_and(|(after, before)| after > before)
        {
            return Err(AppError::InvalidParams(
                "recorded_after must be less than or equal to recorded_before".to_string(),
            ));
        }

        let owner = query
            .scope
            .owner()
            .expect("explicit memory scope must have an owner");
        let namespace = query
            .scope
            .namespace()
            .expect("explicit memory scope must have a namespace");
        let recorded_at_sort_key = "recorded_at_sort_key";
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT event_id, recorded_at, observed_at, owner, namespace, kind, summary, feedback_json FROM events WHERE owner = ",
        );
        builder
            .push_bind(owner_as_str(owner))
            .push(" AND namespace = ")
            .push_bind(namespace.as_str());
        if let Some(reference) = query.event_reference.as_ref() {
            builder
                .push(" AND event_id = ")
                .push_bind(reference.event_id());
        }
        if let Some(kind) = query.kind {
            builder
                .push(" AND kind = ")
                .push_bind(event_kind_as_str(kind));
        }
        if let Some(after) = query.recorded_after {
            builder
                .push(" AND ")
                .push(recorded_at_sort_key)
                .push(" >= ")
                .push_bind(utc_timestamp_sort_key(&after));
        }
        if let Some(before) = query.recorded_before {
            builder
                .push(" AND ")
                .push(recorded_at_sort_key)
                .push(" <= ")
                .push_bind(utc_timestamp_sort_key(&before));
        }
        builder
            .push(" ORDER BY ")
            .push(recorded_at_sort_key)
            .push(if ordering == ReadOrdering::Union {
                " DESC, event_id DESC LIMIT "
            } else {
                " DESC, rowid DESC LIMIT "
            })
            .push_bind(i64::try_from(query.limit).map_err(|_| {
                AppError::InvalidParams(
                    "event record query limit exceeds the supported maximum".to_string(),
                )
            })?);

        let stored_events = map_sqlite(builder.build().fetch_all(&self.pool).await)?
            .iter()
            .map(stored_event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        if stored_events.is_empty() {
            return Ok(Vec::new());
        }

        let event_ids = stored_events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>();
        let claim_ids = load_event_claim_ids(&self.pool, &event_ids, owner, namespace).await?;
        let episode_references = load_event_episode_references(&self.pool, &event_ids).await?;

        Ok(stored_events
            .into_iter()
            .map(|event| {
                let event_id = event.event_id.clone();
                EventReadRecord::new(
                    event,
                    claim_ids.get(&event_id).cloned().unwrap_or_default(),
                    episode_references
                        .get(&event_id)
                        .cloned()
                        .unwrap_or_default(),
                )
            })
            .collect())
    }
    async fn query_episode_records_ordered(
        &self,
        query: EpisodeRecordQuery,
        ordering: ReadOrdering,
    ) -> Result<Vec<EpisodeReadRecord>, AppError> {
        if !query.scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "episode record query requires an explicit namespace".to_string(),
            ));
        }
        if query.limit == 0 {
            return Err(AppError::InvalidParams(
                "episode record query limit must be at least 1".to_string(),
            ));
        }
        if query.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "episode record query limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }
        if let Some(reference) = query.episode_reference.as_deref()
            && (reference.is_empty() || reference.trim() != reference)
        {
            return Err(AppError::InvalidParams(
                "episode_reference must be non-empty and have no leading or trailing whitespace"
                    .to_string(),
            ));
        }

        let owner = query
            .scope
            .owner()
            .expect("explicit memory scope must have an owner");
        let namespace = query
            .scope
            .namespace()
            .expect("explicit memory scope must have a namespace");
        let recorded_at_sort_key = "e.recorded_at_sort_key";
        let mut builder = QueryBuilder::<Sqlite>::new(
            r#"
            WITH ranked_episode_events AS (
                SELECT
                    ee.episode_reference,
                    e.recorded_at,
            "#,
        );
        builder
            .push(recorded_at_sort_key)
            .push(
                r#" AS recorded_at_sort_key,
                    e.rowid AS event_rowid,
                    ROW_NUMBER() OVER (
                        PARTITION BY ee.episode_reference
                        ORDER BY "#,
            )
            .push(recorded_at_sort_key)
            .push(
                r#" DESC, e.rowid DESC
                    ) AS episode_rank
                FROM episode_events ee
                INNER JOIN events e ON e.event_id = ee.event_id
                WHERE e.owner = "#,
            )
            .push_bind(owner_as_str(owner))
            .push(" AND e.namespace = ")
            .push_bind(namespace.as_str());
        if let Some(reference) = query.episode_reference.as_deref() {
            builder
                .push(" AND ee.episode_reference = ")
                .push_bind(reference);
        }
        builder
            .push(
                r#"
            )
            SELECT episode_reference, recorded_at, recorded_at_sort_key, event_rowid
            FROM ranked_episode_events
            WHERE episode_rank = 1
            ORDER BY recorded_at_sort_key DESC, "#,
            )
            .push(if ordering == ReadOrdering::Union {
                "episode_reference DESC LIMIT "
            } else {
                "event_rowid DESC, episode_reference ASC LIMIT "
            })
            .push_bind(i64::try_from(query.limit).map_err(|_| {
                AppError::InvalidParams(
                    "episode record query limit exceeds the supported maximum".to_string(),
                )
            })?);

        let episode_rows = map_sqlite(builder.build().fetch_all(&self.pool).await)?
            .into_iter()
            .map(|row| {
                Ok((
                    row.get::<String, _>("episode_reference"),
                    parse_timestamp(&row.get::<String, _>("recorded_at"))?,
                ))
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        if episode_rows.is_empty() {
            return Ok(Vec::new());
        }

        let episode_references = episode_rows
            .iter()
            .map(|(reference, _)| reference.as_str())
            .collect::<Vec<_>>();
        let event_references =
            load_episode_event_references(&self.pool, &episode_references, owner, namespace)
                .await?;
        let claim_references =
            load_episode_claim_references(&self.pool, &episode_references, owner, namespace)
                .await?;

        Ok(episode_rows
            .into_iter()
            .map(|(episode_reference, recorded_at)| {
                EpisodeReadRecord::new(
                    episode_reference.clone(),
                    recorded_at,
                    owner,
                    namespace.clone(),
                    event_references
                        .get(&episode_reference)
                        .cloned()
                        .unwrap_or_default(),
                    claim_references
                        .get(&episode_reference)
                        .cloned()
                        .unwrap_or_default(),
                )
            })
            .collect())
    }
    async fn query_reflection_records_ordered(
        &self,
        query: ReflectionRecordQuery,
        ordering: ReadOrdering,
    ) -> Result<Vec<ReflectionReadRecord>, AppError> {
        if !query.scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "reflection record query requires an explicit namespace".to_string(),
            ));
        }
        if query.limit == 0 {
            return Err(AppError::InvalidParams(
                "reflection record query limit must be at least 1".to_string(),
            ));
        }
        if query.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "reflection record query limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }
        if let Some(reference) = query.reflection_reference.as_deref()
            && (reference.is_empty() || reference.trim() != reference)
        {
            return Err(AppError::InvalidParams(
                "reflection_reference must be non-empty and have no leading or trailing whitespace"
                    .to_string(),
            ));
        }

        let owner = query
            .scope
            .owner()
            .expect("explicit memory scope must have an owner");
        let namespace = query
            .scope
            .namespace()
            .expect("explicit memory scope must have a namespace");
        // Preserve both legacy endpoint checks. Targetless rows additionally need
        // verified origin, safe affected scopes and complete normalized evidence.
        let recorded_at_sort_key = "r.recorded_at_sort_key";
        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT r.reflection_id, r.recorded_at, r.summary, r.superseded_claim_id, \
             r.replacement_claim_id, {} AS supporting_evidence_event_ids \
             FROM reflections r \
             LEFT JOIN claims superseded ON superseded.claim_id = r.superseded_claim_id \
             LEFT JOIN claims replacement ON replacement.claim_id = r.replacement_claim_id \
             WHERE ",
            crate::adapters::sqlite::reflection_scope::evidence_ids_sql("r"),
        ));
        push_reflection_visibility(&mut builder, owner, namespace);
        if let Some(reference) = query.reflection_reference.as_deref() {
            builder.push(" AND r.reflection_id = ").push_bind(reference);
        }
        builder
            .push(" ORDER BY ")
            .push(recorded_at_sort_key)
            .push(if ordering == ReadOrdering::Union {
                " DESC, r.reflection_id DESC LIMIT "
            } else {
                " DESC, r.rowid DESC LIMIT "
            })
            .push_bind(i64::try_from(query.limit).map_err(|_| {
                AppError::InvalidParams(
                    "reflection record query limit exceeds the supported maximum".to_string(),
                )
            })?);

        let unfiltered = map_sqlite(builder.build().fetch_all(&self.pool).await)?
            .into_iter()
            .map(|row| {
                Ok(UnfilteredClaimReflectionHistoryRecord {
                    reflection_id: row.get("reflection_id"),
                    recorded_at: parse_timestamp(&row.get::<String, _>("recorded_at"))?,
                    summary: row.get("summary"),
                    superseded_claim_id: row.get("superseded_claim_id"),
                    replacement_claim_id: row.get("replacement_claim_id"),
                    supporting_evidence_event_ids: deserialize_json(
                        &row.get::<String, _>("supporting_evidence_event_ids"),
                    )?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        if unfiltered.is_empty() {
            return Ok(Vec::new());
        }
        let scoped_evidence_ids = load_scoped_event_id_set(
            &self.pool,
            unfiltered
                .iter()
                .flat_map(|record| record.supporting_evidence_event_ids.iter())
                .map(String::as_str),
            owner,
            namespace,
        )
        .await?;
        let mut records: Vec<ReflectionReadRecord> = unfiltered
            .into_iter()
            .map(|record| {
                let scoped = record.into_scoped_record(&scoped_evidence_ids);
                ReflectionReadRecord::new(
                    scoped.reflection_id,
                    scoped.recorded_at,
                    owner,
                    namespace.clone(),
                    scoped.summary,
                    ReflectionProvenanceLinks {
                        superseded_claim_reference: scoped.superseded_claim_reference,
                        replacement_claim_reference: scoped.replacement_claim_reference,
                        supporting_evidence_event_references: scoped
                            .supporting_evidence_event_references,
                    },
                )
            })
            .collect();
        for record in &mut records {
            record.scope = crate::adapters::sqlite::reflection_scope::load_metadata(
                &self.pool,
                &record.reflection_id,
            )
            .await?;
        }
        Ok(records)
    }
    async fn query_claim_records_ordered(
        &self,
        query: ClaimRecordQuery,
        ordering: ReadOrdering,
    ) -> Result<Vec<ClaimReadRecord>, AppError> {
        if !query.scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "claim record query requires an explicit namespace".to_string(),
            ));
        }
        if query.limit == 0 {
            return Err(AppError::InvalidParams(
                "claim record query limit must be at least 1".to_string(),
            ));
        }
        if query.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "claim record query limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }

        let owner = query
            .scope
            .owner()
            .expect("explicit memory scope must have an owner");
        let namespace = query
            .scope
            .namespace()
            .expect("explicit memory scope must have a namespace");
        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT claim_id, owner, namespace, subject, predicate, object, mode, status, recorded_at, observed_at FROM claims WHERE owner = ",
        );
        builder
            .push_bind(owner_as_str(owner))
            .push(" AND namespace = ")
            .push_bind(namespace.as_str());
        if let Some(reference) = query.claim_reference.as_ref() {
            builder
                .push(" AND claim_id = ")
                .push_bind(reference.claim_id());
        }
        if let Some(status) = query.status {
            builder.push(" AND status = ").push_bind(status.as_str());
        }
        if let Some(mode) = query.mode {
            builder.push(" AND mode = ").push_bind(mode_as_str(mode));
        }
        builder
            .push(if ordering == ReadOrdering::Union {
                " ORDER BY recorded_at_sort_key DESC, claim_id DESC LIMIT "
            } else {
                " ORDER BY claim_id ASC LIMIT "
            })
            .push_bind(i64::try_from(query.limit).map_err(|_| {
                AppError::InvalidParams(
                    "claim record query limit exceeds the supported maximum".to_string(),
                )
            })?);

        let stored_claims = map_sqlite(builder.build().fetch_all(&self.pool).await)?
            .iter()
            .map(stored_claim_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        if stored_claims.is_empty() {
            return Ok(Vec::new());
        }

        let claim_ids = stored_claims
            .iter()
            .map(|claim| claim.claim_id.as_str())
            .collect::<Vec<_>>();
        let evidence =
            load_claim_evidence_references(&self.pool, &claim_ids, owner, namespace).await?;
        let episodes =
            load_claim_episode_references(&self.pool, &claim_ids, owner, namespace).await?;
        let revisions = load_claim_revision_links(&self.pool, &claim_ids, owner, namespace).await?;

        Ok(stored_claims
            .into_iter()
            .map(|claim| {
                let claim_id = claim.claim_id.clone();
                ClaimReadRecord::new(
                    claim,
                    evidence.get(&claim_id).cloned().unwrap_or_default(),
                    episodes.get(&claim_id).cloned().unwrap_or_default(),
                    revisions.get(&claim_id).cloned().unwrap_or_default(),
                )
            })
            .collect())
    }
}

#[async_trait]
impl MemoryReadStore for SqliteStore {
    async fn query_event_records(
        &self,
        query: EventRecordQuery,
    ) -> Result<Vec<EventReadRecord>, AppError> {
        self.query_event_records_ordered(query, ReadOrdering::Standalone)
            .await
    }

    async fn query_event_records_for_union(
        &self,
        query: EventRecordQuery,
    ) -> Result<Vec<EventReadRecord>, AppError> {
        self.query_event_records_ordered(query, ReadOrdering::Union)
            .await
    }

    async fn query_episode_records(
        &self,
        query: EpisodeRecordQuery,
    ) -> Result<Vec<EpisodeReadRecord>, AppError> {
        self.query_episode_records_ordered(query, ReadOrdering::Standalone)
            .await
    }

    async fn query_episode_records_for_union(
        &self,
        query: EpisodeRecordQuery,
    ) -> Result<Vec<EpisodeReadRecord>, AppError> {
        self.query_episode_records_ordered(query, ReadOrdering::Union)
            .await
    }

    async fn query_reflection_records(
        &self,
        query: ReflectionRecordQuery,
    ) -> Result<Vec<ReflectionReadRecord>, AppError> {
        self.query_reflection_records_ordered(query, ReadOrdering::Standalone)
            .await
    }

    async fn query_reflection_records_for_union(
        &self,
        query: ReflectionRecordQuery,
    ) -> Result<Vec<ReflectionReadRecord>, AppError> {
        self.query_reflection_records_ordered(query, ReadOrdering::Union)
            .await
    }

    async fn query_scoped_event_ids(
        &self,
        query: ScopedEventIdQuery,
    ) -> Result<BTreeSet<String>, AppError> {
        if !query.scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "scoped event-id query requires an explicit namespace".to_string(),
            ));
        }
        if query.event_ids.len() > MAX_EVIDENCE_MANIFEST_ITEMS {
            return Err(AppError::InvalidParams(format!(
                "scoped event-id query must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        if query.event_ids.is_empty() {
            return Ok(BTreeSet::new());
        }
        let owner = query
            .scope
            .owner()
            .expect("explicit memory scope must have an owner");
        let namespace = query
            .scope
            .namespace()
            .expect("explicit memory scope must have a namespace");
        load_scoped_event_id_set(
            &self.pool,
            query.event_ids.iter().map(String::as_str),
            owner,
            namespace,
        )
        .await
    }

    async fn query_claim_records(
        &self,
        query: ClaimRecordQuery,
    ) -> Result<Vec<ClaimReadRecord>, AppError> {
        self.query_claim_records_ordered(query, ReadOrdering::Standalone)
            .await
    }

    async fn query_claim_records_for_union(
        &self,
        query: ClaimRecordQuery,
    ) -> Result<Vec<ClaimReadRecord>, AppError> {
        self.query_claim_records_ordered(query, ReadOrdering::Union)
            .await
    }

    async fn query_claim_reflection_history(
        &self,
        query: ClaimReflectionHistoryQuery,
    ) -> Result<ClaimReflectionHistoryPage, AppError> {
        if !query.scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "claim reflection history query requires an explicit namespace".to_string(),
            ));
        }
        if query.limit == 0 {
            return Err(AppError::InvalidParams(
                "claim reflection history query limit must be at least 1".to_string(),
            ));
        }
        if query.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "claim reflection history query limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }

        let owner = query
            .scope
            .owner()
            .expect("explicit memory scope must have an owner");
        let namespace = query
            .scope
            .namespace()
            .expect("explicit memory scope must have a namespace");
        let recorded_at_sort_key = "edge.recorded_at_sort_key";
        let fetch_limit = query.limit + 1;
        // This claim graph still requires a scoped superseded Claim and excludes
        // the entire edge when a replacement Claim exists
        // outside that same scope; returning a redacted edge would still leak its audit text.
        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "WITH RECURSIVE scoped_edges AS (\
             SELECT r.rowid AS reflection_rowid, r.reflection_id, r.recorded_at, r.recorded_at_sort_key, r.summary, \
                    r.superseded_claim_id, r.replacement_claim_id, \
                    {} AS supporting_evidence_event_ids \
             FROM reflections r \
             JOIN claims superseded ON superseded.claim_id = r.superseded_claim_id \
             LEFT JOIN claims replacement ON replacement.claim_id = r.replacement_claim_id \
             WHERE superseded.owner = ",
            crate::adapters::sqlite::reflection_scope::evidence_ids_sql("r"),
        ));
        builder
            .push_bind(owner_as_str(owner))
            .push(" AND superseded.namespace = ")
            .push_bind(namespace.as_str())
            .push(" AND (r.replacement_claim_id IS NULL OR (replacement.owner = ")
            .push_bind(owner_as_str(owner))
            .push(" AND replacement.namespace = ")
            .push_bind(namespace.as_str())
            .push(
                "))), reachable_claims(claim_id) AS (\
                 SELECT claim_id FROM claims \
                 WHERE claim_id = ",
            )
            .push_bind(query.claim_reference.claim_id())
            .push(" AND owner = ")
            .push_bind(owner_as_str(owner))
            .push(" AND namespace = ")
            .push_bind(namespace.as_str())
            .push(
                " UNION \
                 SELECT edge.replacement_claim_id \
                 FROM scoped_edges edge \
                 JOIN reachable_claims reachable \
                   ON edge.superseded_claim_id = reachable.claim_id \
                 WHERE edge.replacement_claim_id IS NOT NULL \
                 UNION \
                 SELECT edge.superseded_claim_id \
                 FROM scoped_edges edge \
                 JOIN reachable_claims reachable \
                   ON edge.replacement_claim_id = reachable.claim_id\
             ) \
             SELECT edge.reflection_rowid, edge.reflection_id, edge.recorded_at, edge.summary, \
                    edge.superseded_claim_id, edge.replacement_claim_id, \
                    edge.supporting_evidence_event_ids \
             FROM scoped_edges edge \
             WHERE edge.superseded_claim_id IN (SELECT claim_id FROM reachable_claims) \
                OR edge.replacement_claim_id IN (SELECT claim_id FROM reachable_claims) \
             ORDER BY ",
            )
            .push(recorded_at_sort_key)
            .push(" DESC, edge.reflection_rowid DESC LIMIT ")
            .push_bind(i64::try_from(fetch_limit).map_err(|_| {
                AppError::InvalidParams(
                    "claim reflection history query limit exceeds the supported maximum"
                        .to_string(),
                )
            })?);

        let mut unfiltered = map_sqlite(builder.build().fetch_all(&self.pool).await)?
            .into_iter()
            .map(|row| {
                Ok(UnfilteredClaimReflectionHistoryRecord {
                    reflection_id: row.get("reflection_id"),
                    recorded_at: parse_timestamp(&row.get::<String, _>("recorded_at"))?,
                    summary: row.get("summary"),
                    superseded_claim_id: row.get("superseded_claim_id"),
                    replacement_claim_id: row.get("replacement_claim_id"),
                    supporting_evidence_event_ids: deserialize_json(
                        &row.get::<String, _>("supporting_evidence_event_ids"),
                    )?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let has_more = unfiltered.len() > query.limit;
        unfiltered.truncate(query.limit);

        let scoped_evidence_ids = load_scoped_event_id_set(
            &self.pool,
            unfiltered
                .iter()
                .flat_map(|record| record.supporting_evidence_event_ids.iter())
                .map(String::as_str),
            owner,
            namespace,
        )
        .await?;
        let mut records: Vec<ClaimReflectionHistoryRecord> = unfiltered
            .into_iter()
            .map(|record| record.into_scoped_record(&scoped_evidence_ids))
            .collect();

        for record in &mut records {
            record.scope = crate::adapters::sqlite::reflection_scope::load_metadata(
                &self.pool,
                &record.reflection_id,
            )
            .await?;
        }
        Ok(ClaimReflectionHistoryPage { records, has_more })
    }

    async fn query_self_model_history(
        &self,
        query: SelfModelHistoryQuery,
    ) -> Result<SelfModelHistoryPage, AppError> {
        if !query.scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "self-model history query requires an explicit namespace".to_string(),
            ));
        }
        if query.limit == 0 {
            return Err(AppError::InvalidParams(
                "self-model history query limit must be at least 1".to_string(),
            ));
        }
        if query.limit > MAX_EVENT_RECORD_QUERY_LIMIT {
            return Err(AppError::InvalidParams(format!(
                "self-model history query limit must be at most {MAX_EVENT_RECORD_QUERY_LIMIT}"
            )));
        }

        let owner = query
            .scope
            .owner()
            .expect("explicit memory scope must have an owner");
        let namespace = query
            .scope
            .namespace()
            .expect("explicit memory scope must have a namespace");
        let recorded_at_sort_key = "r.recorded_at_sort_key";
        let fetch_limit = query.limit + 1;
        let audit_column = match query.history_kind {
            SelfModelHistoryKind::Identity => "r.requested_identity_update",
            SelfModelHistoryKind::Commitment => "r.requested_commitment_updates",
        };
        let mut builder = QueryBuilder::<Sqlite>::new(format!(
            "SELECT r.reflection_id, r.recorded_at, r.summary, r.superseded_claim_id, \
             r.replacement_claim_id, {} AS supporting_evidence_event_ids, \
             r.requested_identity_update, r.requested_commitment_updates \
             FROM reflections r \
             LEFT JOIN claims superseded ON superseded.claim_id = r.superseded_claim_id \
             LEFT JOIN claims replacement ON replacement.claim_id = r.replacement_claim_id \
             WHERE ",
            crate::adapters::sqlite::reflection_scope::evidence_ids_sql("r"),
        ));
        push_reflection_visibility(&mut builder, owner, namespace);
        builder
            .push(" AND ")
            .push(audit_column)
            .push(" IS NOT NULL ORDER BY ")
            .push(recorded_at_sort_key)
            .push(" DESC, r.rowid DESC LIMIT ")
            .push_bind(i64::try_from(fetch_limit).map_err(|_| {
                AppError::InvalidParams(
                    "self-model history query limit exceeds the supported maximum".to_string(),
                )
            })?);

        let mut unfiltered = map_sqlite(builder.build().fetch_all(&self.pool).await)?
            .into_iter()
            .map(|row| {
                Ok(UnfilteredSelfModelHistoryRecord {
                    base: UnfilteredClaimReflectionHistoryRecord {
                        reflection_id: row.get("reflection_id"),
                        recorded_at: parse_timestamp(&row.get::<String, _>("recorded_at"))?,
                        summary: row.get("summary"),
                        superseded_claim_id: row.get("superseded_claim_id"),
                        replacement_claim_id: row.get("replacement_claim_id"),
                        supporting_evidence_event_ids: deserialize_json(
                            &row.get::<String, _>("supporting_evidence_event_ids"),
                        )?,
                    },
                    identity_update: row
                        .get::<Option<String>, _>("requested_identity_update")
                        .as_deref()
                        .map(deserialize_json)
                        .transpose()?,
                    commitment_updates: row
                        .get::<Option<String>, _>("requested_commitment_updates")
                        .as_deref()
                        .map(deserialize_json)
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        let has_more = unfiltered.len() > query.limit;
        unfiltered.truncate(query.limit);
        if unfiltered.is_empty() {
            return Ok(SelfModelHistoryPage {
                records: Vec::new(),
                has_more,
            });
        }
        let scoped_evidence_ids = load_scoped_event_id_set(
            &self.pool,
            unfiltered
                .iter()
                .flat_map(|record| record.base.supporting_evidence_event_ids.iter())
                .map(String::as_str),
            owner,
            namespace,
        )
        .await?;
        let mut records: Vec<SelfModelHistoryRecord> = unfiltered
            .into_iter()
            .map(|record| record.into_scoped_record(query.history_kind, &scoped_evidence_ids))
            .collect();
        for record in &mut records {
            record.scope = crate::adapters::sqlite::reflection_scope::load_metadata(
                &self.pool,
                &record.reflection_id,
            )
            .await?;
        }
        Ok(SelfModelHistoryPage { records, has_more })
    }
}

/// Existing endpoint attribution is retained independently of new scope metadata.
/// A targetless row must satisfy all six scoped predicates in the shared helper.
fn push_reflection_visibility<'a>(
    builder: &mut QueryBuilder<'a, Sqlite>,
    owner: Owner,
    namespace: &'a Namespace,
) {
    builder
        .push("((superseded.owner = ")
        .push_bind(owner_as_str(owner))
        .push(" AND superseded.namespace = ")
        .push_bind(namespace.as_str())
        .push(" AND (r.replacement_claim_id IS NULL OR (replacement.owner = ")
        .push_bind(owner_as_str(owner))
        .push(" AND replacement.namespace = ")
        .push_bind(namespace.as_str())
        .push("))) OR ");
    let predicate = crate::adapters::sqlite::reflection_scope::targetless_visibility_sql("r");
    let parts: Vec<_> = predicate.split('?').collect();
    debug_assert_eq!(parts.len(), 7);
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            builder.push_bind(if index % 2 == 1 {
                owner_as_str(owner)
            } else {
                namespace.as_str()
            });
        }
        builder.push(part);
    }
    builder.push(")");
}

struct UnfilteredClaimReflectionHistoryRecord {
    reflection_id: String,
    recorded_at: DateTime<Utc>,
    summary: String,
    superseded_claim_id: Option<String>,
    replacement_claim_id: Option<String>,
    supporting_evidence_event_ids: Vec<String>,
}

struct UnfilteredSelfModelHistoryRecord {
    base: UnfilteredClaimReflectionHistoryRecord,
    identity_update: Option<ReflectionIdentityUpdate>,
    commitment_updates: Option<Vec<Commitment>>,
}

impl UnfilteredSelfModelHistoryRecord {
    fn into_scoped_record(
        self,
        history_kind: SelfModelHistoryKind,
        scoped_evidence_ids: &BTreeSet<String>,
    ) -> SelfModelHistoryRecord {
        let scoped = self.base.into_scoped_record(scoped_evidence_ids);
        SelfModelHistoryRecord {
            scope: scoped.scope,
            reflection_id: scoped.reflection_id,
            recorded_at: scoped.recorded_at,
            summary: scoped.summary,
            superseded_claim_reference: scoped.superseded_claim_reference,
            replacement_claim_reference: scoped.replacement_claim_reference,
            supporting_evidence_event_references: scoped.supporting_evidence_event_references,
            identity_update: matches!(history_kind, SelfModelHistoryKind::Identity)
                .then_some(self.identity_update)
                .flatten(),
            commitment_updates: matches!(history_kind, SelfModelHistoryKind::Commitment)
                .then_some(self.commitment_updates)
                .flatten(),
        }
    }
}

impl UnfilteredClaimReflectionHistoryRecord {
    fn into_scoped_record(
        self,
        scoped_evidence_ids: &BTreeSet<String>,
    ) -> ClaimReflectionHistoryRecord {
        let mut seen = BTreeSet::new();
        ClaimReflectionHistoryRecord {
            scope: Default::default(),
            reflection_id: self.reflection_id,
            recorded_at: self.recorded_at,
            summary: self.summary,
            superseded_claim_reference: self.superseded_claim_id.map(ClaimReference::from_claim_id),
            replacement_claim_reference: self
                .replacement_claim_id
                .map(ClaimReference::from_claim_id),
            supporting_evidence_event_references: self
                .supporting_evidence_event_ids
                .into_iter()
                .filter(|event_id| {
                    scoped_evidence_ids.contains(event_id) && seen.insert(event_id.clone())
                })
                .map(EventReference::from_event_id)
                .collect(),
        }
    }
}

async fn load_scoped_event_id_set<'a>(
    pool: &SqlitePool,
    event_ids: impl Iterator<Item = &'a str>,
    owner: Owner,
    namespace: &Namespace,
) -> Result<BTreeSet<String>, AppError> {
    let unique = event_ids.map(str::to_string).collect::<BTreeSet<_>>();
    let mut scoped = BTreeSet::new();
    for chunk in unique.iter().collect::<Vec<_>>().chunks(500) {
        let mut builder = QueryBuilder::<Sqlite>::new("SELECT event_id FROM events WHERE owner = ");
        builder
            .push_bind(owner_as_str(owner))
            .push(" AND namespace = ")
            .push_bind(namespace.as_str())
            .push(" AND event_id IN (");
        let mut separated = builder.separated(", ");
        for event_id in chunk {
            separated.push_bind(event_id.as_str());
        }
        separated.push_unseparated(")");
        for row in map_sqlite(builder.build().fetch_all(pool).await)? {
            scoped.insert(row.get("event_id"));
        }
    }
    Ok(scoped)
}

async fn load_event_claim_ids(
    pool: &SqlitePool,
    event_ids: &[&str],
    owner: Owner,
    namespace: &Namespace,
) -> Result<BTreeMap<String, Vec<String>>, AppError> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT el.event_id, el.claim_id FROM evidence_links el JOIN claims c ON c.claim_id = el.claim_id WHERE el.event_id IN (",
    );
    let mut separated = builder.separated(", ");
    for event_id in event_ids {
        separated.push_bind(*event_id);
    }
    separated.push_unseparated(") AND c.owner = ");
    builder
        .push_bind(owner_as_str(owner))
        .push(" AND c.namespace = ")
        .push_bind(namespace.as_str())
        .push(" ORDER BY el.event_id, el.claim_id");
    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for row in map_sqlite(builder.build().fetch_all(pool).await)? {
        grouped
            .entry(row.get("event_id"))
            .or_default()
            .push(row.get("claim_id"));
    }
    Ok(grouped)
}

async fn load_event_episode_references(
    pool: &SqlitePool,
    event_ids: &[&str],
) -> Result<BTreeMap<String, Vec<String>>, AppError> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT event_id, episode_reference FROM episode_events WHERE event_id IN (",
    );
    let mut separated = builder.separated(", ");
    for event_id in event_ids {
        separated.push_bind(*event_id);
    }
    separated.push_unseparated(") ORDER BY event_id, episode_reference");
    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for row in map_sqlite(builder.build().fetch_all(pool).await)? {
        grouped
            .entry(row.get("event_id"))
            .or_default()
            .push(row.get("episode_reference"));
    }
    Ok(grouped)
}

async fn load_episode_event_references(
    pool: &SqlitePool,
    episode_references: &[&str],
    owner: Owner,
    namespace: &Namespace,
) -> Result<BTreeMap<String, Vec<EventReference>>, AppError> {
    let recorded_at_sort_key = "e.recorded_at_sort_key";
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT ee.episode_reference, e.event_id FROM episode_events ee INNER JOIN events e ON e.event_id = ee.event_id WHERE ee.episode_reference IN (",
    );
    let mut separated = builder.separated(", ");
    for episode_reference in episode_references {
        separated.push_bind(*episode_reference);
    }
    separated.push_unseparated(") AND e.owner = ");
    builder
        .push_bind(owner_as_str(owner))
        .push(" AND e.namespace = ")
        .push_bind(namespace.as_str())
        .push(" ORDER BY ee.episode_reference, ")
        .push(recorded_at_sort_key)
        .push(" DESC, e.rowid DESC");

    let mut grouped = BTreeMap::<String, Vec<EventReference>>::new();
    for row in map_sqlite(builder.build().fetch_all(pool).await)? {
        grouped
            .entry(row.get("episode_reference"))
            .or_default()
            .push(EventReference::from_event_id(
                row.get::<String, _>("event_id"),
            ));
    }
    Ok(grouped)
}

async fn load_episode_claim_references(
    pool: &SqlitePool,
    episode_references: &[&str],
    owner: Owner,
    namespace: &Namespace,
) -> Result<BTreeMap<String, Vec<ClaimReference>>, AppError> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT ee.episode_reference, c.claim_id FROM episode_events ee INNER JOIN events e ON e.event_id = ee.event_id INNER JOIN evidence_links el ON el.event_id = e.event_id INNER JOIN claims c ON c.claim_id = el.claim_id WHERE ee.episode_reference IN (",
    );
    let mut separated = builder.separated(", ");
    for episode_reference in episode_references {
        separated.push_bind(*episode_reference);
    }
    separated.push_unseparated(") AND e.owner = ");
    builder
        .push_bind(owner_as_str(owner))
        .push(" AND e.namespace = ")
        .push_bind(namespace.as_str())
        .push(" AND c.owner = ")
        .push_bind(owner_as_str(owner))
        .push(" AND c.namespace = ")
        .push_bind(namespace.as_str())
        .push(" ORDER BY ee.episode_reference, c.claim_id");

    let mut grouped = BTreeMap::<String, Vec<ClaimReference>>::new();
    for row in map_sqlite(builder.build().fetch_all(pool).await)? {
        grouped
            .entry(row.get("episode_reference"))
            .or_default()
            .push(ClaimReference::from_claim_id(
                row.get::<String, _>("claim_id"),
            ));
    }
    Ok(grouped)
}

async fn load_claim_evidence_references(
    pool: &SqlitePool,
    claim_ids: &[&str],
    owner: Owner,
    namespace: &Namespace,
) -> Result<BTreeMap<String, Vec<EventReference>>, AppError> {
    let recorded_at_sort_key = "e.recorded_at_sort_key";
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT el.claim_id, e.event_id FROM evidence_links el JOIN events e ON e.event_id = el.event_id WHERE el.claim_id IN (",
    );
    let mut separated = builder.separated(", ");
    for claim_id in claim_ids {
        separated.push_bind(*claim_id);
    }
    separated.push_unseparated(") AND e.owner = ");
    builder
        .push_bind(owner_as_str(owner))
        .push(" AND e.namespace = ")
        .push_bind(namespace.as_str())
        .push(" ORDER BY el.claim_id, ")
        .push(recorded_at_sort_key)
        .push(" DESC, e.rowid DESC");

    let mut grouped = BTreeMap::<String, Vec<EventReference>>::new();
    for row in map_sqlite(builder.build().fetch_all(pool).await)? {
        grouped
            .entry(row.get("claim_id"))
            .or_default()
            .push(EventReference::parse(row.get::<String, _>("event_id")).map_err(AppError::from)?);
    }
    Ok(grouped)
}

async fn load_claim_episode_references(
    pool: &SqlitePool,
    claim_ids: &[&str],
    owner: Owner,
    namespace: &Namespace,
) -> Result<BTreeMap<String, Vec<String>>, AppError> {
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT el.claim_id, ee.episode_reference FROM evidence_links el JOIN events e ON e.event_id = el.event_id JOIN episode_events ee ON ee.event_id = e.event_id WHERE el.claim_id IN (",
    );
    let mut separated = builder.separated(", ");
    for claim_id in claim_ids {
        separated.push_bind(*claim_id);
    }
    separated.push_unseparated(") AND e.owner = ");
    builder
        .push_bind(owner_as_str(owner))
        .push(" AND e.namespace = ")
        .push_bind(namespace.as_str())
        .push(" ORDER BY el.claim_id, ee.episode_reference");

    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for row in map_sqlite(builder.build().fetch_all(pool).await)? {
        grouped
            .entry(row.get("claim_id"))
            .or_default()
            .push(row.get("episode_reference"));
    }
    Ok(grouped)
}

async fn load_claim_revision_links(
    pool: &SqlitePool,
    claim_ids: &[&str],
    owner: Owner,
    namespace: &Namespace,
) -> Result<BTreeMap<String, ClaimRevisionLinks>, AppError> {
    let recorded_at_sort_key = "r.recorded_at_sort_key";
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT r.reflection_id, r.superseded_claim_id, r.replacement_claim_id, \
         CASE WHEN superseded.owner = ",
    );
    builder
        .push_bind(owner_as_str(owner))
        .push(" AND superseded.namespace = ")
        .push_bind(namespace.as_str())
        .push(
            " THEN r.superseded_claim_id END AS scoped_superseded_claim_id, \
               CASE WHEN replacement.owner = ",
        )
        .push_bind(owner_as_str(owner))
        .push(" AND replacement.namespace = ")
        .push_bind(namespace.as_str())
        .push(
            " THEN r.replacement_claim_id END AS scoped_replacement_claim_id \
               FROM reflections r \
               LEFT JOIN claims superseded ON superseded.claim_id = r.superseded_claim_id \
               LEFT JOIN claims replacement ON replacement.claim_id = r.replacement_claim_id \
               WHERE r.superseded_claim_id IN (",
        );
    let mut superseded = builder.separated(", ");
    for claim_id in claim_ids {
        superseded.push_bind(*claim_id);
    }
    superseded.push_unseparated(") OR r.replacement_claim_id IN (");
    let mut replacement = builder.separated(", ");
    for claim_id in claim_ids {
        replacement.push_bind(*claim_id);
    }
    replacement
        .push_unseparated(") ORDER BY ")
        .push_unseparated(recorded_at_sort_key)
        .push_unseparated(" DESC, r.rowid DESC");

    let mut grouped = BTreeMap::<String, ClaimRevisionLinks>::new();
    for row in map_sqlite(builder.build().fetch_all(pool).await)? {
        let reflection_id = row.get::<String, _>("reflection_id");
        let superseded_claim_id = row.get::<Option<String>, _>("superseded_claim_id");
        let replacement_claim_id = row.get::<Option<String>, _>("replacement_claim_id");
        let scoped_superseded_claim_id = row.get::<Option<String>, _>("scoped_superseded_claim_id");
        let scoped_replacement_claim_id =
            row.get::<Option<String>, _>("scoped_replacement_claim_id");

        if let Some(claim_id) = replacement_claim_id.as_deref()
            && claim_ids.contains(&claim_id)
            && !revision_edge_leaves_requested_scope(
                superseded_claim_id.as_deref(),
                scoped_superseded_claim_id.as_deref(),
            )
        {
            let links = grouped.entry(claim_id.to_string()).or_default();
            links
                .source_reflection_id
                .get_or_insert(reflection_id.clone());
            if links.supersedes_claim_reference.is_none() {
                links.supersedes_claim_reference =
                    scoped_superseded_claim_id.map(ClaimReference::from_claim_id);
            }
        }
        if let Some(claim_id) = superseded_claim_id.as_deref()
            && claim_ids.contains(&claim_id)
            && !revision_edge_leaves_requested_scope(
                replacement_claim_id.as_deref(),
                scoped_replacement_claim_id.as_deref(),
            )
        {
            let links = grouped.entry(claim_id.to_string()).or_default();
            links
                .superseded_by_reflection_id
                .get_or_insert(reflection_id);
            if links.replacement_claim_reference.is_none() {
                links.replacement_claim_reference =
                    scoped_replacement_claim_id.map(ClaimReference::from_claim_id);
            }
        }
    }
    Ok(grouped)
}

fn revision_edge_leaves_requested_scope(
    endpoint_id: Option<&str>,
    scoped_endpoint_id: Option<&str>,
) -> bool {
    endpoint_id.is_some() && scoped_endpoint_id.is_none()
}

pub(super) async fn query_evidence_event_ids_with_limit(
    pool: &SqlitePool,
    query: EvidenceQuery,
    default_limit: Option<usize>,
) -> Result<Vec<String>, AppError> {
    if query.limit == Some(0) {
        return Err(AppError::InvalidParams(
            "evidence query limit must be at least 1".to_string(),
        ));
    }

    let mut sql = String::from("SELECT event_id, recorded_at, owner, kind, summary FROM events");
    let recorded_at_sort_key = "recorded_at_sort_key";
    let mut predicates = Vec::new();

    if query.namespace.is_some() {
        predicates.push("namespace = ?".to_string());
    }

    if query.owner.is_some() {
        predicates.push("owner = ?".to_string());
    }

    if query.kind.is_some() {
        predicates.push("kind = ?".to_string());
    }

    if query.recorded_after.is_some() {
        predicates.push(format!("{recorded_at_sort_key} >= ?"));
    }

    if query.recorded_before.is_some() {
        predicates.push(format!("{recorded_at_sort_key} <= ?"));
    }

    if query.event_id_prefix.is_some() {
        predicates.push("event_id LIKE ? || '%'".to_string());
    }

    if !predicates.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));
    }

    sql.push_str(" ORDER BY ");
    sql.push_str(recorded_at_sort_key);
    sql.push_str(" DESC, rowid DESC");
    if query.limit.is_some() || default_limit.is_some() {
        sql.push_str(" LIMIT ?");
    }

    let mut rows = {
        let mut query_builder = sqlx::query(&sql);

        if let Some(namespace) = query.namespace {
            query_builder = query_builder.bind(namespace.as_str().to_string());
        }

        if let Some(owner) = query.owner {
            query_builder = query_builder.bind(owner_as_str(owner));
        }

        if let Some(kind) = query.kind {
            query_builder = query_builder.bind(event_kind_as_str(kind));
        }

        if let Some(after) = query.recorded_after {
            query_builder = query_builder.bind(utc_timestamp_sort_key(&after));
        }

        if let Some(before) = query.recorded_before {
            query_builder = query_builder.bind(utc_timestamp_sort_key(&before));
        }

        if let Some(prefix) = query.event_id_prefix {
            query_builder = query_builder.bind(prefix);
        }

        if let Some(limit) = query.limit.or(default_limit) {
            let limit = i64::try_from(limit).map_err(|_| {
                AppError::InvalidParams(
                    "evidence query limit exceeds the supported maximum".to_string(),
                )
            })?;
            query_builder = query_builder.bind(limit);
        }

        map_sqlite(query_builder.fetch_all(pool).await)?
    };

    Ok(rows
        .drain(..)
        .map(|row| row.get::<String, _>("event_id"))
        .collect())
}

pub(super) async fn load_identity_rows<'e, E>(executor: E) -> Result<IdentityCore, AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let rows = map_sqlite(
        sqlx::query(
            r#"
            SELECT claim
            FROM identity_claims
            ORDER BY position
            "#,
        )
        .fetch_all(executor)
        .await,
    )?;

    if rows.is_empty() {
        return Err(AppError::Message("missing identity".to_string()));
    }

    Ok(IdentityCore::new(
        rows.into_iter().map(|row| row.get("claim")).collect(),
    ))
}

pub(super) async fn load_commitment_rows<'e, E>(executor: E) -> Result<Vec<Commitment>, AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let rows = map_sqlite(
        sqlx::query(
            r#"
            SELECT owner, description
            FROM commitments
            ORDER BY rowid
            "#,
        )
        .fetch_all(executor)
        .await,
    )?;

    rows.into_iter()
        .map(|row| {
            Ok(Commitment::new(
                parse_owner(&row.get::<String, _>("owner"))?,
                row.get::<String, _>("description"),
            ))
        })
        .collect()
}
