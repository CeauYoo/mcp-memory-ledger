use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{QueryBuilder, Row, Sqlite, sqlite::SqlitePool};

use crate::{
    domain::{
        claim::{ClaimDraft, ClaimReference},
        commitment::Commitment,
        event::{Event, EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        identity_core::IdentityCore,
        operation_log::{
            ActorKind, OperationLogEntry, OperationLogKind, OperationLogStatus, redact_secrets,
        },
        reflection::ReflectionIdentityUpdate,
        self_revision::TriggerType,
        snapshot::SnapshotTimeWindow,
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::{
        ClaimReadRecord, ClaimRecordQuery, ClaimReflectionHistoryPage, ClaimReflectionHistoryQuery,
        ClaimReflectionHistoryRecord, ClaimRevisionLinks, ClaimStatus, ClaimStore, CommitmentStore,
        EpisodeReadRecord, EpisodeRecordQuery, EpisodeStore, EventReadRecord, EventRecordQuery,
        EventStore, EvidenceQuery, IdentityStore, IngestTransaction, IngestTransactionRunner,
        MAX_EVENT_RECORD_QUERY_LIMIT, MemoryReadStore, OperationLogQuery, OperationLogStore,
        ReflectionProvenanceLinks, ReflectionReadRecord, ReflectionRecordQuery, ReflectionStore,
        ReflectionTransaction, ReflectionTransactionRunner, ScopedEventIdQuery,
        SelfModelHistoryKind, SelfModelHistoryPage, SelfModelHistoryQuery, SelfModelHistoryRecord,
        StoredClaim, StoredEvent, StoredReflection, StoredTriggerLedgerEntry, StoredWriteReceipt,
        TriggerLedgerStatus, TriggerLedgerStore, WriteReceiptRequest,
    },
};

use super::schema::{
    OWNER_NAMESPACE_SCOPE_CONSTRAINT_NAME, claims_table_sql, events_table_sql,
    legacy_namespace_backfill_expression,
};

#[derive(Clone)]
pub struct SqliteStore {
    pub(super) pool: SqlitePool,
}

impl SqliteStore {
    pub async fn bootstrap(database_url: &str) -> Result<Self, AppError> {
        super::lifecycle::bootstrap_database(database_url).await
    }
}

fn utc_timestamp_sort_key(timestamp: &DateTime<Utc>) -> String {
    // Match the persisted fixed-width seconds/nanoseconds pair, including
    // Chrono's nanosecond >= 1e9 representation of an RFC3339 leap second.
    format!(
        "{:020}:{:010}",
        timestamp.timestamp() + 10_000_000_000_000_i64,
        timestamp.timestamp_subsec_nanos()
    )
}

#[async_trait]
impl EventStore for SqliteStore {
    async fn append_event(&self, event: StoredEvent) -> Result<(), AppError> {
        insert_event(&self.pool, &event).await
    }

    async fn list_event_references(&self) -> Result<Vec<String>, AppError> {
        let rows = map_sqlite(
            sqlx::query("SELECT event_id FROM events ORDER BY rowid")
                .fetch_all(&self.pool)
                .await,
        )?;

        Ok(rows
            .into_iter()
            .map(|row| format!("event:{}", row.get::<String, _>("event_id")))
            .collect())
    }

    async fn list_event_references_in_scope(
        &self,
        scope: &MemoryScope,
        evidence_manifest: Option<&[EventReference]>,
    ) -> Result<Vec<String>, AppError> {
        if scope.is_legacy_unscoped() && evidence_manifest.is_none() {
            return self.list_event_references().await;
        }
        self.list_event_references_for_snapshot(
            scope,
            evidence_manifest,
            &SnapshotTimeWindow::unbounded(),
        )
        .await
    }

    async fn list_event_references_for_snapshot(
        &self,
        scope: &MemoryScope,
        evidence_manifest: Option<&[EventReference]>,
        time_window: &SnapshotTimeWindow,
    ) -> Result<Vec<String>, AppError> {
        time_window.validate().map_err(|_| {
            AppError::InvalidParams(
                "recorded_after must be less than or equal to recorded_before".to_string(),
            )
        })?;
        if evidence_manifest.is_some_and(|manifest| manifest.len() > MAX_EVIDENCE_MANIFEST_ITEMS) {
            return Err(AppError::InvalidParams(format!(
                "evidence_manifest must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        if evidence_manifest.is_some() && !scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "evidence_manifest requires an explicit namespace".to_string(),
            ));
        }
        if !time_window.is_unbounded() && !scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "snapshot time window requires an explicit namespace".to_string(),
            ));
        }
        let Some(manifest) = evidence_manifest else {
            return self
                .query_evidence_event_ids_unbounded(EvidenceQuery {
                    namespace: scope.namespace().cloned(),
                    owner: scope.owner(),
                    kind: None,
                    limit: None,
                    recorded_after: time_window.recorded_after,
                    recorded_before: time_window.recorded_before,
                    event_id_prefix: None,
                })
                .await
                .map(|event_ids| {
                    event_ids
                        .into_iter()
                        .map(|event_id| EventReference::from_event_id(event_id).canonical())
                        .collect()
                });
        };
        if manifest.is_empty() {
            return Ok(Vec::new());
        }

        let (Some(owner), Some(namespace)) = (scope.owner(), scope.namespace()) else {
            return Err(AppError::InvalidParams(
                "evidence_manifest requires an explicit namespace".to_string(),
            ));
        };
        let mut query = QueryBuilder::<Sqlite>::new("SELECT event_id FROM events WHERE ");
        let recorded_at_sort_key = "recorded_at_sort_key";
        query
            .push("owner = ")
            .push_bind(owner_as_str(owner))
            .push(" AND namespace = ")
            .push_bind(namespace.as_str());
        if let Some(recorded_after) = time_window.recorded_after {
            query
                .push(" AND ")
                .push(recorded_at_sort_key)
                .push(" >= ")
                .push_bind(utc_timestamp_sort_key(&recorded_after));
        }
        if let Some(recorded_before) = time_window.recorded_before {
            query
                .push(" AND ")
                .push(recorded_at_sort_key)
                .push(" <= ")
                .push_bind(utc_timestamp_sort_key(&recorded_before));
        }
        query.push(" AND event_id IN (");
        let mut separated = query.separated(", ");
        for reference in manifest {
            separated.push_bind(reference.event_id());
        }
        separated
            .push_unseparated(") ORDER BY ")
            .push_unseparated(recorded_at_sort_key)
            .push_unseparated(" DESC, rowid DESC");

        let rows = map_sqlite(query.build().fetch_all(&self.pool).await)?;
        Ok(rows
            .into_iter()
            .map(|row| EventReference::from_event_id(row.get::<String, _>("event_id")).canonical())
            .collect())
    }

    async fn list_recorded_at_for_snapshot_manifest(
        &self,
        scope: &MemoryScope,
        evidence_manifest: &[EventReference],
    ) -> Result<Vec<DateTime<Utc>>, AppError> {
        if evidence_manifest.is_empty() {
            return Ok(Vec::new());
        }
        if evidence_manifest.len() > MAX_EVIDENCE_MANIFEST_ITEMS {
            return Err(AppError::InvalidParams(format!(
                "evidence_manifest must contain at most {MAX_EVIDENCE_MANIFEST_ITEMS} entries"
            )));
        }
        let (Some(owner), Some(namespace)) = (scope.owner(), scope.namespace()) else {
            return Err(AppError::InvalidParams(
                "evidence_manifest requires an explicit namespace".to_string(),
            ));
        };
        let mut query =
            QueryBuilder::<Sqlite>::new("SELECT recorded_at FROM events WHERE owner = ");
        query
            .push_bind(owner_as_str(owner))
            .push(" AND namespace = ")
            .push_bind(namespace.as_str())
            .push(" AND event_id IN (");
        let mut separated = query.separated(", ");
        for reference in evidence_manifest {
            separated.push_bind(reference.event_id());
        }
        separated.push_unseparated(")");
        map_sqlite(query.build().fetch_all(&self.pool).await)?
            .into_iter()
            .map(|row| parse_timestamp(&row.get::<String, _>("recorded_at")))
            .collect()
    }

    async fn query_evidence_event_ids(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        query_evidence_event_ids_with_limit(&self.pool, query, Some(10)).await
    }

    async fn query_evidence_event_ids_unbounded(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        query_evidence_event_ids_with_limit(&self.pool, query, None).await
    }

    async fn has_event(&self, event_id: &str) -> Result<bool, AppError> {
        let count = map_sqlite(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE event_id = ?")
                .bind(event_id)
                .fetch_one(&self.pool)
                .await,
        )?;

        Ok(count > 0)
    }
}

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
            super::reflection_scope::evidence_ids_sql("r"),
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
            record.scope =
                super::reflection_scope::load_metadata(&self.pool, &record.reflection_id).await?;
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
            super::reflection_scope::evidence_ids_sql("r"),
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
            record.scope =
                super::reflection_scope::load_metadata(&self.pool, &record.reflection_id).await?;
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
            super::reflection_scope::evidence_ids_sql("r"),
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
            record.scope =
                super::reflection_scope::load_metadata(&self.pool, &record.reflection_id).await?;
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
    let predicate = super::reflection_scope::targetless_visibility_sql("r");
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

async fn query_evidence_event_ids_with_limit(
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

#[async_trait]
impl ClaimStore for SqliteStore {
    async fn upsert_claim(&self, claim: StoredClaim) -> Result<(), AppError> {
        upsert_claim_row(&self.pool, &claim).await
    }

    async fn link_evidence(&self, claim_id: String, event_id: String) -> Result<(), AppError> {
        insert_evidence_link(&self.pool, &claim_id, &event_id).await
    }

    async fn list_active_claims(&self) -> Result<Vec<StoredClaim>, AppError> {
        let rows = map_sqlite(
            sqlx::query(
                r#"
                SELECT claim_id, owner, subject, predicate, object, mode, status
                , namespace, recorded_at, observed_at
                FROM claims
                WHERE status = ?
                ORDER BY rowid
                "#,
            )
            .bind(ClaimStatus::Active.as_str())
            .fetch_all(&self.pool)
            .await,
        )?;

        rows.into_iter()
            .map(|row| stored_claim_from_row(&row))
            .collect()
    }

    async fn list_active_claims_in_scope(
        &self,
        scope: &MemoryScope,
    ) -> Result<Vec<StoredClaim>, AppError> {
        let (Some(owner), Some(namespace)) = (scope.owner(), scope.namespace()) else {
            return self.list_active_claims().await;
        };
        let rows = map_sqlite(
            sqlx::query(
                r#"
                SELECT claim_id, owner, subject, predicate, object, mode, status, namespace, recorded_at, observed_at
                FROM claims
                WHERE status = ? AND owner = ? AND namespace = ?
                ORDER BY rowid
                "#,
            )
            .bind(ClaimStatus::Active.as_str())
            .bind(owner_as_str(owner))
            .bind(namespace.as_str())
            .fetch_all(&self.pool)
            .await,
        )?;

        rows.into_iter()
            .map(|row| stored_claim_from_row(&row))
            .collect()
    }

    async fn update_claim_status(
        &self,
        claim_id: &str,
        status: ClaimStatus,
    ) -> Result<(), AppError> {
        update_claim_status_row(&self.pool, claim_id, status).await
    }
}

#[async_trait]
impl EpisodeStore for SqliteStore {
    async fn record_event_in_episode(
        &self,
        episode_reference: String,
        event_id: String,
    ) -> Result<(), AppError> {
        insert_episode_event(&self.pool, &episode_reference, &event_id).await
    }

    async fn list_episode_references(&self) -> Result<Vec<String>, AppError> {
        let rows = map_sqlite(
            sqlx::query(
                r#"
                SELECT episode_reference
                FROM episode_events
                GROUP BY episode_reference
                ORDER BY MIN(rowid)
                "#,
            )
            .fetch_all(&self.pool)
            .await,
        )?;

        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("episode_reference"))
            .collect())
    }

    async fn list_episode_references_supporting_claims(
        &self,
        scope: &MemoryScope,
        claim_ids: &[String],
    ) -> Result<Vec<String>, AppError> {
        let (Some(owner), Some(namespace)) = (scope.owner(), scope.namespace()) else {
            return Err(AppError::InvalidParams(
                "claim-to-evidence-to-episode lookup requires an explicit namespace".to_string(),
            ));
        };
        if claim_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            SELECT
                episode_events.episode_reference,
                MIN(episode_events.rowid) AS first_episode_event_rowid
            FROM evidence_links
            INNER JOIN claims
                ON claims.claim_id = evidence_links.claim_id
            INNER JOIN events
                ON events.event_id = evidence_links.event_id
            INNER JOIN episode_events
                ON episode_events.event_id = evidence_links.event_id
            WHERE claims.owner =
            "#,
        );
        query
            .push_bind(owner_as_str(owner))
            .push(" AND claims.namespace = ")
            .push_bind(namespace.as_str())
            .push(" AND events.owner = ")
            .push_bind(owner_as_str(owner))
            .push(" AND events.namespace = ")
            .push_bind(namespace.as_str())
            .push(" AND evidence_links.claim_id IN (");
        let mut separated = query.separated(", ");
        for claim_id in claim_ids {
            separated.push_bind(claim_id);
        }
        separated.push_unseparated(")");
        query.push(
            r#"
            GROUP BY episode_events.episode_reference
            ORDER BY first_episode_event_rowid ASC, episode_events.episode_reference ASC
            "#,
        );

        let rows = map_sqlite(query.build().fetch_all(&self.pool).await)?;
        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("episode_reference"))
            .collect())
    }

    async fn list_episode_references_in_scope(
        &self,
        scope: &MemoryScope,
    ) -> Result<Vec<String>, AppError> {
        if scope.is_legacy_unscoped() {
            return self.list_episode_references().await;
        }
        self.list_episode_references_for_snapshot(scope, &SnapshotTimeWindow::unbounded())
            .await
    }

    async fn list_episode_references_for_snapshot(
        &self,
        scope: &MemoryScope,
        time_window: &SnapshotTimeWindow,
    ) -> Result<Vec<String>, AppError> {
        time_window.validate().map_err(|_| {
            AppError::InvalidParams(
                "recorded_after must be less than or equal to recorded_before".to_string(),
            )
        })?;
        if !time_window.is_unbounded() && !scope.is_explicitly_scoped() {
            return Err(AppError::InvalidParams(
                "snapshot time window requires an explicit namespace".to_string(),
            ));
        }
        let recorded_at_sort_key = "events.recorded_at_sort_key";
        let mut query = QueryBuilder::<Sqlite>::new(
            r#"
            WITH ranked_episode_events AS (
                SELECT
                    episode_events.episode_reference,
            "#,
        );
        query
            .push(recorded_at_sort_key)
            .push(
                r#" AS recorded_at_sort_key,
                    events.rowid AS event_rowid,
                    ROW_NUMBER() OVER (
                        PARTITION BY episode_events.episode_reference
                        ORDER BY "#,
            )
            .push(recorded_at_sort_key)
            .push(
                r#" DESC, events.rowid DESC, episode_events.rowid DESC
                    ) AS episode_rank
                FROM episode_events
                INNER JOIN events ON events.event_id = episode_events.event_id
                WHERE 1 = 1
            "#,
            );
        if let (Some(owner), Some(namespace)) = (scope.owner(), scope.namespace()) {
            query
                .push(" AND events.owner = ")
                .push_bind(owner_as_str(owner))
                .push(" AND events.namespace = ")
                .push_bind(namespace.as_str());
        }
        if let Some(recorded_after) = time_window.recorded_after {
            query
                .push(" AND ")
                .push(recorded_at_sort_key)
                .push(" >= ")
                .push_bind(utc_timestamp_sort_key(&recorded_after));
        }
        if let Some(recorded_before) = time_window.recorded_before {
            query
                .push(" AND ")
                .push(recorded_at_sort_key)
                .push(" <= ")
                .push_bind(utc_timestamp_sort_key(&recorded_before));
        }
        query.push(
            r#"
            )
            SELECT episode_reference
            FROM ranked_episode_events
            WHERE episode_rank = 1
            ORDER BY recorded_at_sort_key DESC, event_rowid DESC, episode_reference ASC
            "#,
        );
        let rows = map_sqlite(query.build().fetch_all(&self.pool).await)?;

        Ok(rows
            .into_iter()
            .map(|row| row.get::<String, _>("episode_reference"))
            .collect())
    }
}

#[async_trait]
impl ReflectionStore for SqliteStore {
    async fn append_reflection(&self, reflection: StoredReflection) -> Result<(), AppError> {
        let mut transaction = map_sqlite(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        insert_reflection(transaction.as_mut(), &reflection).await?;
        map_sqlite(transaction.commit().await)?;
        Ok(())
    }
}

#[async_trait]
impl TriggerLedgerStore for SqliteStore {
    async fn record_trigger_attempt(
        &self,
        entry: StoredTriggerLedgerEntry,
    ) -> Result<(), AppError> {
        insert_trigger_ledger_entry(&self.pool, &entry).await
    }

    async fn latest_trigger_entry(
        &self,
        trigger_key: &str,
    ) -> Result<Option<StoredTriggerLedgerEntry>, AppError> {
        // Task 2 defines "latest" as the last recorded attempt for the
        // canonical trigger key, so append order wins over business timestamps.
        let row = map_sqlite(
            sqlx::query(
                r#"
                SELECT
                    ledger_id,
                    trigger_type,
                    namespace,
                    trigger_key,
                    status,
                    evidence_window,
                    handled_at,
                    cooldown_until,
                    episode_watermark,
                    reflection_id
                FROM reflection_trigger_ledger
                WHERE trigger_key = ?
                ORDER BY rowid DESC
                LIMIT 1
                "#,
            )
            .bind(trigger_key)
            .fetch_optional(&self.pool)
            .await,
        )?;

        row.as_ref()
            .map(stored_trigger_ledger_entry_from_row)
            .transpose()
    }

    async fn latest_handled_trigger_entry(
        &self,
        trigger_key: &str,
    ) -> Result<Option<StoredTriggerLedgerEntry>, AppError> {
        let row = map_sqlite(
            sqlx::query(
                r#"
                SELECT
                    ledger_id,
                    trigger_type,
                    namespace,
                    trigger_key,
                    status,
                    evidence_window,
                    handled_at,
                    cooldown_until,
                    episode_watermark,
                    reflection_id
                FROM reflection_trigger_ledger
                WHERE trigger_key = ? AND status = ?
                ORDER BY rowid DESC
                LIMIT 1
                "#,
            )
            .bind(trigger_key)
            .bind(TriggerLedgerStatus::Handled.as_str())
            .fetch_optional(&self.pool)
            .await,
        )?;

        row.as_ref()
            .map(stored_trigger_ledger_entry_from_row)
            .transpose()
    }
}

#[async_trait]
impl IdentityStore for SqliteStore {
    async fn load_identity(&self) -> Result<IdentityCore, AppError> {
        load_identity_rows(&self.pool).await
    }

    async fn save_identity(&self, identity: IdentityCore) -> Result<(), AppError> {
        let mut tx = map_sqlite(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        replace_identity_rows(tx.as_mut(), &identity).await?;
        map_sqlite(tx.commit().await)?;
        Ok(())
    }
}

#[async_trait]
impl CommitmentStore for SqliteStore {
    async fn list_commitments(&self) -> Result<Vec<Commitment>, AppError> {
        load_commitment_rows(&self.pool).await
    }
}

#[async_trait]
impl IngestTransactionRunner for SqliteStore {
    async fn begin_ingest_transaction(
        &self,
    ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError> {
        let transaction = map_sqlite(self.pool.begin_with("BEGIN IMMEDIATE").await)?;

        Ok(Box::new(SqliteIngestTransaction {
            transaction: Some(transaction),
            poisoned: false,
        }))
    }
}

#[async_trait]
impl ReflectionTransactionRunner for SqliteStore {
    async fn begin_reflection_transaction(
        &self,
    ) -> Result<Box<dyn ReflectionTransaction + Send + '_>, AppError> {
        let transaction = map_sqlite(self.pool.begin_with("BEGIN IMMEDIATE").await)?;

        Ok(Box::new(SqliteReflectionTransaction {
            transaction: Some(transaction),
            poisoned: false,
        }))
    }
}

struct SqliteIngestTransaction<'a> {
    transaction: Option<sqlx::Transaction<'a, Sqlite>>,
    poisoned: bool,
}

#[async_trait]
impl IngestTransaction for SqliteIngestTransaction<'_> {
    async fn load_event_for_ingest(
        &mut self,
        event_id: &str,
    ) -> Result<Option<StoredEvent>, AppError> {
        self.ensure_writable()?;
        let result = async {
            let transaction = self.transaction.as_mut().ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            let row = map_sqlite(sqlx::query("SELECT event_id, recorded_at, observed_at, owner, namespace, kind, summary, feedback_json FROM events WHERE event_id = ?")
                .bind(event_id).fetch_optional(transaction.as_mut()).await)?;
            row.as_ref().map(stored_event_from_row).transpose()
        }.await;
        self.note_result(result)
    }

    async fn load_write_receipt(
        &mut self,
        operation_id: &str,
    ) -> Result<Option<StoredWriteReceipt>, AppError> {
        self.ensure_writable()?;
        let result = async {
            let transaction = self.transaction.as_mut().ok_or_else(|| AppError::Message("transaction already closed".into()))?;
            let row = map_sqlite(sqlx::query("SELECT request_summary_json, response_summary_json FROM operation_log WHERE operation_id = ? AND actor_id = 'durable_write_receipt_v1' AND status = 'ok'")
                .bind(operation_id).fetch_optional(transaction.as_mut()).await)?;
            row.map(|row| {
                let request_json: String = row.get("request_summary_json");
                let metadata: serde_json::Value = deserialize_json(&request_json)?;
                let request_hash = metadata.get("request_hash").and_then(|v| v.as_str()).ok_or_else(|| AppError::Message("invalid durable receipt request hash".into()))?.to_string();
                Ok(StoredWriteReceipt { request_hash, result_json: row.get("response_summary_json") })
            }).transpose()
        }.await;
        self.note_result(result)
    }
    async fn append_write_receipt(
        &mut self,
        request: &WriteReceiptRequest,
        receipt: StoredWriteReceipt,
        recorded_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;
        let result = async {
            let transaction = self.transaction.as_mut().ok_or_else(|| AppError::Message("transaction already closed".into()))?;
            map_sqlite(sqlx::query("INSERT INTO operation_log (operation_id, occurred_at, namespace, actor_kind, actor_id, entrypoint, operation_kind, status, request_summary_json, response_summary_json, redaction_version) VALUES (?, ?, ?, 'system', 'durable_write_receipt_v1', ?, 'tool', 'ok', ?, ?, 1)")
                .bind(&request.operation_id).bind(recorded_at.to_rfc3339()).bind(&request.namespace).bind(&request.operation)
                .bind(serde_json::json!({"request_hash": receipt.request_hash}).to_string()).bind(&receipt.result_json)
                .execute(transaction.as_mut()).await)?;
            Ok(())
        }.await;
        self.note_result(result)
    }
    async fn append_event(&mut self, event: StoredEvent) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            insert_event(transaction.as_mut(), &event).await
        };
        self.note_result(result)
    }

    async fn record_event_in_episode(
        &mut self,
        episode_reference: String,
        event_id: String,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            insert_episode_event(transaction.as_mut(), &episode_reference, &event_id).await
        };
        self.note_result(result)
    }

    async fn upsert_claim(&mut self, claim: StoredClaim) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            upsert_claim_row(transaction.as_mut(), &claim).await
        };
        self.note_result(result)
    }

    async fn link_evidence(&mut self, claim_id: String, event_id: String) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            insert_evidence_link(transaction.as_mut(), &claim_id, &event_id).await
        };
        self.note_result(result)
    }

    async fn commit(mut self: Box<Self>) -> Result<(), AppError> {
        if self.poisoned {
            return Err(AppError::Message(
                "transaction is poisoned and cannot be committed".to_string(),
            ));
        }

        let transaction = self
            .transaction
            .take()
            .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
        map_sqlite(transaction.commit().await)?;
        Ok(())
    }
}

impl SqliteIngestTransaction<'_> {
    fn ensure_writable(&self) -> Result<(), AppError> {
        if self.poisoned {
            return Err(AppError::Message(
                "transaction is poisoned and cannot accept more writes".to_string(),
            ));
        }

        Ok(())
    }

    fn note_result<T>(&mut self, result: Result<T, AppError>) -> Result<T, AppError> {
        if result.is_err() {
            self.poisoned = true;
        }

        result
    }
}

struct SqliteReflectionTransaction<'a> {
    transaction: Option<sqlx::Transaction<'a, Sqlite>>,
    poisoned: bool,
}

#[async_trait]
impl ReflectionTransaction for SqliteReflectionTransaction<'_> {
    async fn load_feedback_candidate(
        &mut self,
        namespace: &Namespace,
        candidate_id: &str,
    ) -> Result<Option<crate::domain::feedback_candidate::FeedbackCandidate>, AppError> {
        self.ensure_writable()?;
        let result = super::feedback_candidate::load(
            self.transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".into()))?
                .as_mut(),
            namespace,
            candidate_id,
        )
        .await;
        self.note_result(result)
    }
    async fn list_feedback_candidates_for_target(
        &mut self,
        namespace: &Namespace,
        target: &str,
        version: &str,
    ) -> Result<Vec<crate::domain::feedback_candidate::FeedbackCandidate>, AppError> {
        self.ensure_writable()?;
        let result = super::feedback_candidate::list_for_target(
            self.transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".into()))?
                .as_mut(),
            namespace,
            target,
            version,
        )
        .await;
        self.note_result(result)
    }
    async fn insert_feedback_candidate(
        &mut self,
        candidate: &crate::domain::feedback_candidate::FeedbackCandidate,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;
        let result = super::feedback_candidate::insert(
            self.transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".into()))?
                .as_mut(),
            candidate,
        )
        .await;
        self.note_result(result)
    }
    async fn update_feedback_candidate(
        &mut self,
        candidate: &crate::domain::feedback_candidate::FeedbackCandidate,
        expected_revision: u64,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;
        let result = super::feedback_candidate::update(
            self.transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".into()))?
                .as_mut(),
            candidate,
            expected_revision,
        )
        .await;
        self.note_result(result)
    }
    async fn load_write_receipt(
        &mut self,
        operation_id: &str,
    ) -> Result<Option<StoredWriteReceipt>, AppError> {
        self.ensure_writable()?;
        let result = async {
            let transaction = self.transaction.as_mut().ok_or_else(|| AppError::Message("transaction already closed".into()))?;
            let row = map_sqlite(sqlx::query("SELECT request_summary_json, response_summary_json FROM operation_log WHERE operation_id = ? AND actor_id = 'durable_write_receipt_v1' AND status = 'ok'")
                .bind(operation_id).fetch_optional(transaction.as_mut()).await)?;
            row.map(|row| {
                let request_json: String = row.get("request_summary_json");
                let metadata: serde_json::Value = deserialize_json(&request_json)?;
                let request_hash = metadata.get("request_hash").and_then(|v| v.as_str()).ok_or_else(|| AppError::Message("invalid durable receipt request hash".into()))?.to_string();
                Ok(StoredWriteReceipt { request_hash, result_json: row.get("response_summary_json") })
            }).transpose()
        }.await;
        self.note_result(result)
    }
    async fn append_write_receipt(
        &mut self,
        request: &WriteReceiptRequest,
        receipt: StoredWriteReceipt,
        recorded_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;
        let result = async {
            let transaction = self.transaction.as_mut().ok_or_else(|| AppError::Message("transaction already closed".into()))?;
            map_sqlite(sqlx::query("INSERT INTO operation_log (operation_id, occurred_at, namespace, actor_kind, actor_id, entrypoint, operation_kind, status, request_summary_json, response_summary_json, redaction_version) VALUES (?, ?, ?, 'system', 'durable_write_receipt_v1', ?, 'tool', 'ok', ?, ?, 1)")
                .bind(&request.operation_id).bind(recorded_at.to_rfc3339()).bind(&request.namespace).bind(&request.operation)
                .bind(serde_json::json!({"request_hash": receipt.request_hash}).to_string()).bind(&receipt.result_json)
                .execute(transaction.as_mut()).await)?;
            Ok(())
        }.await;
        self.note_result(result)
    }
    async fn load_claim_for_reflection(
        &mut self,
        claim_id: &str,
    ) -> Result<Option<StoredClaim>, AppError> {
        self.ensure_writable()?;
        let result = async {
            let transaction = self.transaction.as_mut().ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            let row = map_sqlite(sqlx::query("SELECT claim_id, owner, namespace, subject, predicate, object, mode, status, recorded_at, observed_at FROM claims WHERE claim_id = ?")
                .bind(claim_id).fetch_optional(transaction.as_mut()).await)?;
            row.as_ref().map(stored_claim_from_row).transpose()
        }.await;
        self.note_result(result)
    }

    async fn load_event_for_reflection(
        &mut self,
        event_id: &str,
    ) -> Result<Option<StoredEvent>, AppError> {
        self.ensure_writable()?;
        let result = async {
            let transaction = self.transaction.as_mut().ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            let row = map_sqlite(sqlx::query("SELECT event_id, recorded_at, observed_at, owner, namespace, kind, summary, feedback_json FROM events WHERE event_id = ?")
                .bind(event_id).fetch_optional(transaction.as_mut()).await)?;
            row.as_ref().map(stored_event_from_row).transpose()
        }.await;
        self.note_result(result)
    }

    async fn compare_and_set_claim_status(
        &mut self,
        claim_id: &str,
        expected: ClaimStatus,
        status: ClaimStatus,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;
        let result = async {
            if expected == ClaimStatus::Superseded || status == ClaimStatus::Active {
                return Err(AppError::InvalidParams(
                    "invalid reflection claim status transition".to_string(),
                ));
            }
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            let updated = map_sqlite(
                sqlx::query("UPDATE claims SET status = ? WHERE claim_id = ? AND status = ?")
                    .bind(status.as_str())
                    .bind(claim_id)
                    .bind(expected.as_str())
                    .execute(transaction.as_mut())
                    .await,
            )?;
            if updated.rows_affected() != 1 {
                return Err(AppError::InvalidParams(
                    "reflection target claim changed or no longer exists".to_string(),
                ));
            }
            Ok(())
        }
        .await;
        self.note_result(result)
    }

    async fn upsert_claim(&mut self, claim: StoredClaim) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            upsert_claim_row(transaction.as_mut(), &claim).await
        };
        self.note_result(result)
    }

    async fn link_evidence(&mut self, claim_id: String, event_id: String) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            insert_evidence_link(transaction.as_mut(), &claim_id, &event_id).await
        };
        self.note_result(result)
    }

    async fn append_reflection(&mut self, reflection: StoredReflection) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            insert_reflection(transaction.as_mut(), &reflection).await
        };
        self.note_result(result)
    }

    async fn append_trigger_ledger(
        &mut self,
        entry: StoredTriggerLedgerEntry,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            insert_trigger_ledger_entry(transaction.as_mut(), &entry).await
        };
        self.note_result(result)
    }

    async fn load_identity(&mut self) -> Result<IdentityCore, AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            load_identity_rows(transaction.as_mut()).await
        };
        self.note_result(result)
    }

    async fn replace_identity(&mut self, identity: IdentityCore) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            replace_identity_rows(transaction.as_mut(), &identity).await
        };
        self.note_result(result)
    }

    async fn load_commitments(&mut self) -> Result<Vec<Commitment>, AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            load_commitment_rows(transaction.as_mut()).await
        };
        self.note_result(result)
    }

    async fn replace_commitments(&mut self, commitments: Vec<Commitment>) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            replace_commitment_rows(transaction.as_mut(), &commitments).await
        };
        self.note_result(result)
    }

    async fn update_claim_status(
        &mut self,
        claim_id: &str,
        status: ClaimStatus,
    ) -> Result<(), AppError> {
        self.ensure_writable()?;

        let result = {
            let transaction = self
                .transaction
                .as_mut()
                .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
            update_claim_status_row(transaction.as_mut(), claim_id, status).await
        };
        self.note_result(result)
    }

    async fn commit(mut self: Box<Self>) -> Result<(), AppError> {
        if self.poisoned {
            return Err(AppError::Message(
                "transaction is poisoned and cannot be committed".to_string(),
            ));
        }

        let transaction = self
            .transaction
            .take()
            .ok_or_else(|| AppError::Message("transaction already closed".to_string()))?;
        map_sqlite(transaction.commit().await)?;
        Ok(())
    }
}

impl SqliteReflectionTransaction<'_> {
    fn ensure_writable(&self) -> Result<(), AppError> {
        if self.poisoned {
            return Err(AppError::Message(
                "transaction is poisoned and cannot accept more writes".to_string(),
            ));
        }

        Ok(())
    }

    fn note_result<T>(&mut self, result: Result<T, AppError>) -> Result<T, AppError> {
        if result.is_err() {
            self.poisoned = true;
        }

        result
    }
}

async fn insert_event<'e, E>(executor: E, event: &StoredEvent) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    if let Some(feedback) = event.event.feedback() {
        feedback.validate()?;
    }
    let feedback_json = event.event.feedback().map(serialize_json).transpose()?;
    map_sqlite(
        sqlx::query(
            r#"
            INSERT INTO events (event_id, recorded_at, observed_at, owner, namespace, kind, summary, feedback_json)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&event.event_id)
        .bind(event.recorded_at.to_rfc3339())
        .bind(&event.observed_at)
        .bind(owner_as_str(event.event.owner()))
        .bind(event.event.namespace().as_str())
        .bind(event_kind_as_str(event.event.kind()))
        .bind(event.event.summary())
        .bind(feedback_json)
        .execute(executor)
        .await,
    )?;

    Ok(())
}

async fn upsert_claim_row<'e, E>(executor: E, claim: &StoredClaim) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    claim.claim.validate_namespace_owner().map_err(|error| {
        AppError::Message(format!("invalid claim namespace mapping: {error:?}"))
    })?;

    // Conflict updates preserve ledger creation provenance, including legacy NULL.
    // Supplying a new clock value must not silently date an existing unknown row.
    map_sqlite(
        sqlx::query(
            r#"
            INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status, recorded_at, observed_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(claim_id) DO UPDATE SET
                owner = excluded.owner,
                namespace = excluded.namespace,
                subject = excluded.subject,
                predicate = excluded.predicate,
                object = excluded.object,
                mode = excluded.mode,
                status = excluded.status
            "#,
        )
        .bind(&claim.claim_id)
        .bind(owner_as_str(claim.claim.owner()))
        .bind(claim.claim.namespace().as_str())
        .bind(claim.claim.subject())
        .bind(claim.claim.predicate())
        .bind(claim.claim.object())
        .bind(mode_as_str(claim.claim.mode()))
        .bind(claim.status.as_str())
        .bind(claim.recorded_at.map(|value| value.to_rfc3339()))
        .bind(&claim.observed_at)
        .execute(executor)
        .await,
    )?;

    Ok(())
}

async fn insert_evidence_link<'e, E>(
    executor: E,
    claim_id: &str,
    event_id: &str,
) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    map_sqlite(
        sqlx::query(
            r#"
            INSERT INTO evidence_links (claim_id, event_id)
            VALUES (?, ?)
            "#,
        )
        .bind(claim_id)
        .bind(event_id)
        .execute(executor)
        .await,
    )?;

    Ok(())
}

async fn insert_episode_event<'e, E>(
    executor: E,
    episode_reference: &str,
    event_id: &str,
) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    map_sqlite(
        sqlx::query(
            r#"
            INSERT INTO episode_events (episode_reference, event_id)
            VALUES (?, ?)
            "#,
        )
        .bind(episode_reference)
        .bind(event_id)
        .execute(executor)
        .await,
    )?;

    Ok(())
}

async fn insert_reflection(
    connection: &mut sqlx::SqliteConnection,
    reflection: &StoredReflection,
) -> Result<(), AppError> {
    let supporting_evidence_event_ids = serialize_json(&reflection.supporting_evidence_event_ids)?;
    let requested_identity_update = serialize_optional_json(&reflection.requested_identity_update)?;
    let requested_commitment_updates =
        serialize_optional_json(&reflection.requested_commitment_updates)?;

    map_sqlite(
        sqlx::query(
            r#"
            INSERT INTO reflections (
                reflection_id,
                recorded_at,
                summary,
                superseded_claim_id,
                replacement_claim_id,
                supporting_evidence_event_ids,
                requested_identity_update,
                requested_commitment_updates
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&reflection.reflection_id)
        .bind(reflection.recorded_at.to_rfc3339())
        .bind(reflection.reflection.summary())
        .bind(&reflection.superseded_claim_id)
        .bind(&reflection.replacement_claim_id)
        .bind(supporting_evidence_event_ids)
        .bind(requested_identity_update)
        .bind(requested_commitment_updates)
        .execute(&mut *connection)
        .await,
    )?;

    super::reflection_scope::persist_relations(connection, reflection).await
}

async fn insert_trigger_ledger_entry<'e, E>(
    executor: E,
    entry: &StoredTriggerLedgerEntry,
) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    map_sqlite(
        sqlx::query(
            r#"
            INSERT INTO reflection_trigger_ledger (
                ledger_id, trigger_type, namespace, trigger_key, status,
                evidence_window, handled_at, cooldown_until, episode_watermark, reflection_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&entry.ledger_id)
        .bind(trigger_type_as_str(entry.trigger_type))
        .bind(entry.namespace.as_str())
        .bind(&entry.trigger_key)
        .bind(entry.status.as_str())
        .bind(serialize_json(&entry.evidence_window)?)
        .bind(entry.handled_at.map(|value| value.to_rfc3339()))
        .bind(entry.cooldown_until.map(|value| value.to_rfc3339()))
        .bind(serialize_episode_watermark(entry.episode_watermark)?)
        .bind(&entry.reflection_id)
        .execute(executor)
        .await,
    )?;

    Ok(())
}

async fn update_claim_status_row<'e, E>(
    executor: E,
    claim_id: &str,
    status: ClaimStatus,
) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let result = map_sqlite(
        sqlx::query(
            r#"
            UPDATE claims
            SET status = ?
            WHERE claim_id = ?
            "#,
        )
        .bind(status.as_str())
        .bind(claim_id)
        .execute(executor)
        .await,
    )?;

    if result.rows_affected() == 0 {
        return Err(AppError::Message(format!(
            "cannot update missing claim: {claim_id}"
        )));
    }

    Ok(())
}

pub(super) async fn seed_baseline_commitments<'e, E>(executor: E) -> Result<(), AppError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    map_sqlite(
        sqlx::query(
            r#"
            INSERT OR IGNORE INTO commitments (description, owner)
            VALUES (?, ?)
            "#,
        )
        .bind("forbid:write_identity_core_directly")
        .bind("self")
        .execute(executor)
        .await,
    )?;

    Ok(())
}

pub(super) async fn ensure_claims_namespace_column(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), AppError> {
    let namespace_column_exists = map_sqlite(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pragma_table_info('claims') WHERE name = 'namespace'",
        )
        .fetch_one(&mut *connection)
        .await,
    )? > 0;
    let namespace_is_not_null = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT "notnull" FROM pragma_table_info('claims') WHERE name = 'namespace'"#,
            )
            .fetch_one(&mut *connection)
            .await,
        )? == 1
    } else {
        false
    };
    let claims_has_scope_check = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, String>(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'claims'",
            )
            .fetch_one(&mut *connection)
            .await,
        )?
        .contains(OWNER_NAMESPACE_SCOPE_CONSTRAINT_NAME)
    } else {
        false
    };

    if !namespace_column_exists || !namespace_is_not_null || !claims_has_scope_check {
        rebuild_claims_table_with_namespace(connection, namespace_column_exists).await?;
    }

    Ok(())
}

pub(super) async fn ensure_events_namespace_column(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), AppError> {
    let namespace_column_exists = map_sqlite(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM pragma_table_info('events') WHERE name = 'namespace'",
        )
        .fetch_one(&mut *connection)
        .await,
    )? > 0;
    let namespace_is_not_null = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, i64>(
                r#"SELECT "notnull" FROM pragma_table_info('events') WHERE name = 'namespace'"#,
            )
            .fetch_one(&mut *connection)
            .await,
        )? == 1
    } else {
        false
    };
    let events_has_scope_check = if namespace_column_exists {
        map_sqlite(
            sqlx::query_scalar::<_, String>(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'events'",
            )
            .fetch_one(&mut *connection)
            .await,
        )?
        .contains(OWNER_NAMESPACE_SCOPE_CONSTRAINT_NAME)
    } else {
        false
    };

    if !namespace_column_exists || !namespace_is_not_null || !events_has_scope_check {
        rebuild_events_table_with_namespace(connection, namespace_column_exists).await?;
    }

    Ok(())
}

pub(super) async fn ensure_reflection_audit_columns(
    connection: &mut sqlx::SqliteConnection,
) -> Result<(), AppError> {
    let columns = map_sqlite(
        sqlx::query("SELECT name FROM pragma_table_info('reflections')")
            .fetch_all(&mut *connection)
            .await,
    )?
    .into_iter()
    .map(|row| row.get::<String, _>("name"))
    .collect::<Vec<_>>();

    if !columns.contains(&"supporting_evidence_event_ids".to_string()) {
        map_sqlite(
            sqlx::query(
                "ALTER TABLE reflections ADD COLUMN supporting_evidence_event_ids TEXT NOT NULL DEFAULT '[]'",
            )
            .execute(&mut *connection)
            .await,
        )?;
    }

    if !columns.contains(&"requested_identity_update".to_string()) {
        map_sqlite(
            sqlx::query("ALTER TABLE reflections ADD COLUMN requested_identity_update TEXT")
                .execute(&mut *connection)
                .await,
        )?;
    }

    if !columns.contains(&"requested_commitment_updates".to_string()) {
        map_sqlite(
            sqlx::query("ALTER TABLE reflections ADD COLUMN requested_commitment_updates TEXT")
                .execute(&mut *connection)
                .await,
        )?;
    }

    Ok(())
}

async fn rebuild_claims_table_with_namespace(
    connection: &mut sqlx::SqliteConnection,
    legacy_table_has_namespace: bool,
) -> Result<(), AppError> {
    let create_claims_table_sql = claims_table_sql(false);
    let namespace_expression = legacy_namespace_backfill_expression(legacy_table_has_namespace);
    let copy_sql = format!(
        r#"
        INSERT INTO claims (claim_id, owner, namespace, subject, predicate, object, mode, status)
        SELECT claim_id, owner, {namespace_expression}, subject, predicate, object, mode, status
        FROM claims_legacy
        "#
    );

    map_sqlite(
        sqlx::query("ALTER TABLE claims RENAME TO claims_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(
        sqlx::query(&create_claims_table_sql)
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(sqlx::query(&copy_sql).execute(&mut *connection).await)?;
    map_sqlite(
        sqlx::query("DROP TABLE claims_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    Ok(())
}

async fn rebuild_events_table_with_namespace(
    connection: &mut sqlx::SqliteConnection,
    legacy_table_has_namespace: bool,
) -> Result<(), AppError> {
    let create_events_table_sql = events_table_sql(false);
    let namespace_expression = legacy_namespace_backfill_expression(legacy_table_has_namespace);
    let copy_sql = format!(
        r#"
        INSERT INTO events (event_id, recorded_at, owner, namespace, kind, summary)
        SELECT event_id, recorded_at, owner, {namespace_expression}, kind, summary
        FROM events_legacy
        "#
    );

    map_sqlite(
        sqlx::query("ALTER TABLE events RENAME TO events_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(
        sqlx::query(&create_events_table_sql)
            .execute(&mut *connection)
            .await,
    )?;
    map_sqlite(sqlx::query(&copy_sql).execute(&mut *connection).await)?;
    map_sqlite(
        sqlx::query("DROP TABLE events_legacy")
            .execute(&mut *connection)
            .await,
    )?;
    Ok(())
}

async fn load_identity_rows<'e, E>(executor: E) -> Result<IdentityCore, AppError>
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

async fn replace_identity_rows(
    connection: &mut sqlx::SqliteConnection,
    identity: &IdentityCore,
) -> Result<(), AppError> {
    map_sqlite(
        sqlx::query("DELETE FROM identity_claims")
            .execute(&mut *connection)
            .await,
    )?;

    for (position, claim) in identity.canonical_claims().iter().enumerate() {
        map_sqlite(
            sqlx::query("INSERT INTO identity_claims (position, claim) VALUES (?, ?)")
                .bind(position as i64)
                .bind(claim)
                .execute(&mut *connection)
                .await,
        )?;
    }

    Ok(())
}

async fn load_commitment_rows<'e, E>(executor: E) -> Result<Vec<Commitment>, AppError>
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

async fn replace_commitment_rows(
    connection: &mut sqlx::SqliteConnection,
    commitments: &[Commitment],
) -> Result<(), AppError> {
    map_sqlite(
        sqlx::query("DELETE FROM commitments")
            .execute(&mut *connection)
            .await,
    )?;

    for commitment in commitments {
        map_sqlite(
            sqlx::query(
                r#"
                INSERT INTO commitments (description, owner)
                VALUES (?, ?)
                "#,
            )
            .bind(commitment.description())
            .bind(owner_as_str(commitment.owner()))
            .execute(&mut *connection)
            .await,
        )?;
    }

    Ok(())
}

fn serialize_json<T>(value: &T) -> Result<String, AppError>
where
    T: serde::Serialize,
{
    serde_json::to_string(value).map_err(|error| AppError::Message(error.to_string()))
}

fn serialize_optional_json<T>(value: &Option<T>) -> Result<Option<String>, AppError>
where
    T: serde::Serialize,
{
    value.as_ref().map(serialize_json).transpose()
}

fn deserialize_json<T>(value: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|error| AppError::Message(error.to_string()))
}

fn serialize_episode_watermark(value: Option<u64>) -> Result<Option<i64>, AppError> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| {
                AppError::InvalidParams(format!(
                    "episode watermark {value} exceeds sqlite INTEGER range"
                ))
            })
        })
        .transpose()
}

fn map_sqlite<T>(result: Result<T, sqlx::Error>) -> Result<T, AppError> {
    result.map_err(|error| AppError::Message(error.to_string()))
}

fn stored_claim_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<StoredClaim, AppError> {
    let claim = ClaimDraft::new(
        parse_owner(&row.get::<String, _>("owner"))?,
        row.get::<String, _>("subject"),
        row.get::<String, _>("predicate"),
        row.get::<String, _>("object"),
        parse_mode(&row.get::<String, _>("mode"))?,
    )
    .with_namespace(parse_namespace(&row.get::<String, _>("namespace"))?);
    claim.validate_namespace_owner().map_err(|error| {
        AppError::Message(format!("invalid stored claim namespace mapping: {error:?}"))
    })?;

    Ok(StoredClaim::new(
        row.get("claim_id"),
        claim,
        parse_claim_status(&row.get::<String, _>("status"))?,
    )
    .with_temporal_metadata(
        parse_optional_timestamp(row.get::<Option<String>, _>("recorded_at"))?,
        row.get("observed_at"),
    ))
}

#[allow(dead_code)]
fn stored_event_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<StoredEvent, AppError> {
    let mut event = Event::new_with_namespace(
        parse_owner(&row.get::<String, _>("owner"))?,
        parse_namespace(&row.get::<String, _>("namespace"))?,
        parse_event_kind(&row.get::<String, _>("kind"))?,
        row.get::<String, _>("summary"),
    )
    .map_err(|error| {
        AppError::Message(format!("invalid stored event namespace mapping: {error:?}"))
    })?;
    if let Some(json) = row.get::<Option<String>, _>("feedback_json") {
        event = event
            .with_feedback(deserialize_json(&json)?)
            .map_err(|error| {
                AppError::Message(format!("invalid stored feedback metadata: {error:?}"))
            })?;
    }
    Ok(StoredEvent::new(
        row.get("event_id"),
        parse_timestamp(&row.get::<String, _>("recorded_at"))?,
        event,
    )
    .with_observed_at(row.get("observed_at")))
}

fn stored_trigger_ledger_entry_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<StoredTriggerLedgerEntry, AppError> {
    let evidence_window = deserialize_json(&row.get::<String, _>("evidence_window"))?;
    let handled_at = parse_optional_timestamp(row.get::<Option<String>, _>("handled_at"))?;
    let cooldown_until = parse_optional_timestamp(row.get::<Option<String>, _>("cooldown_until"))?;
    let episode_watermark = row
        .get::<Option<i64>, _>("episode_watermark")
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                AppError::Message(format!("invalid negative episode watermark: {value}"))
            })
        })
        .transpose()?;

    Ok(StoredTriggerLedgerEntry {
        ledger_id: row.get("ledger_id"),
        trigger_type: parse_trigger_type(&row.get::<String, _>("trigger_type"))?,
        namespace: parse_namespace(&row.get::<String, _>("namespace"))?,
        trigger_key: row.get("trigger_key"),
        status: parse_trigger_ledger_status(&row.get::<String, _>("status"))?,
        evidence_window,
        handled_at,
        cooldown_until,
        episode_watermark,
        reflection_id: row.get("reflection_id"),
    })
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, AppError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| AppError::Message(error.to_string()))
}

fn parse_optional_timestamp(value: Option<String>) -> Result<Option<DateTime<Utc>>, AppError> {
    value.as_deref().map(parse_timestamp).transpose()
}

fn owner_as_str(owner: Owner) -> &'static str {
    match owner {
        Owner::Self_ => "self",
        Owner::User => "user",
        Owner::World => "world",
        Owner::Unknown => "unknown",
    }
}

fn parse_owner(value: &str) -> Result<Owner, AppError> {
    match value {
        "self" => Ok(Owner::Self_),
        "user" => Ok(Owner::User),
        "world" => Ok(Owner::World),
        "unknown" => Ok(Owner::Unknown),
        _ => Err(AppError::Message(format!("unknown owner: {value}"))),
    }
}

fn mode_as_str(mode: Mode) -> &'static str {
    match mode {
        Mode::Observed => "observed",
        Mode::Said => "said",
        Mode::Acted => "acted",
        Mode::Inferred => "inferred",
        Mode::Draft => "draft",
    }
}

fn parse_mode(value: &str) -> Result<Mode, AppError> {
    match value {
        "observed" => Ok(Mode::Observed),
        "said" => Ok(Mode::Said),
        "acted" => Ok(Mode::Acted),
        "inferred" => Ok(Mode::Inferred),
        "draft" => Ok(Mode::Draft),
        _ => Err(AppError::Message(format!("unknown mode: {value}"))),
    }
}

fn parse_namespace(value: &str) -> Result<Namespace, AppError> {
    Namespace::parse(value).map_err(|error| {
        AppError::Message(format!("invalid stored namespace `{value}`: {error:?}"))
    })
}

fn trigger_type_as_str(trigger_type: TriggerType) -> &'static str {
    match trigger_type {
        TriggerType::Conflict => "conflict",
        TriggerType::Failure => "failure",
        TriggerType::Periodic => "periodic",
    }
}

fn parse_trigger_type(value: &str) -> Result<TriggerType, AppError> {
    match value {
        "conflict" => Ok(TriggerType::Conflict),
        "failure" => Ok(TriggerType::Failure),
        "periodic" => Ok(TriggerType::Periodic),
        _ => Err(AppError::Message(format!("unknown trigger type: {value}"))),
    }
}

fn event_kind_as_str(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Observation => "observation",
        EventKind::Conversation => "conversation",
        EventKind::Action => "action",
        EventKind::Reflection => "reflection",
    }
}

fn parse_event_kind(value: &str) -> Result<EventKind, AppError> {
    match value {
        "observation" => Ok(EventKind::Observation),
        "conversation" => Ok(EventKind::Conversation),
        "action" => Ok(EventKind::Action),
        "reflection" => Ok(EventKind::Reflection),
        _ => Err(AppError::Message(format!("unknown event kind: {value}"))),
    }
}

fn parse_claim_status(value: &str) -> Result<ClaimStatus, AppError> {
    match value {
        "active" => Ok(ClaimStatus::Active),
        "disputed" => Ok(ClaimStatus::Disputed),
        "superseded" => Ok(ClaimStatus::Superseded),
        _ => Err(AppError::Message(format!("unknown claim status: {value}"))),
    }
}

fn parse_trigger_ledger_status(value: &str) -> Result<TriggerLedgerStatus, AppError> {
    match value {
        "pending" => Ok(TriggerLedgerStatus::Pending),
        "handled" => Ok(TriggerLedgerStatus::Handled),
        "rejected" => Ok(TriggerLedgerStatus::Rejected),
        "suppressed" => Ok(TriggerLedgerStatus::Suppressed),
        _ => Err(AppError::Message(format!(
            "unknown trigger ledger status: {value}"
        ))),
    }
}

#[async_trait]
impl OperationLogStore for SqliteStore {
    async fn append_operation(&self, entry: OperationLogEntry) -> Result<(), AppError> {
        map_sqlite(
            sqlx::query(
                "INSERT INTO operation_log (operation_id, occurred_at, namespace, actor_kind, actor_id, entrypoint, operation_kind, status, correlation_id, request_summary_json, response_summary_json, diagnostic_summary_json, redaction_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&entry.operation_id)
            .bind(entry.occurred_at.to_rfc3339())
            .bind(&entry.namespace)
            .bind(entry.actor_kind.as_str())
            .bind(&entry.actor_id)
            .bind(&entry.entrypoint)
            .bind(entry.operation_kind.as_str())
            .bind(entry.status.as_str())
            .bind(&entry.correlation_id)
            .bind(entry.request_summary_json.as_deref().map(redact_secrets))
            .bind(entry.response_summary_json.as_deref().map(redact_secrets))
            .bind(entry.diagnostic_summary_json.as_deref().map(redact_secrets))
            .bind(entry.redaction_version)
            .execute(&self.pool)
            .await,
        )?;
        Ok(())
    }

    async fn query_operations(
        &self,
        query: OperationLogQuery,
    ) -> Result<Vec<OperationLogEntry>, AppError> {
        let mut sql = String::from(
            "SELECT operation_id, occurred_at, namespace, actor_kind, actor_id, entrypoint, operation_kind, status, correlation_id, request_summary_json, response_summary_json, diagnostic_summary_json, redaction_version FROM operation_log",
        );
        let mut predicates = Vec::new();

        if query.operation_id.is_some() {
            predicates.push("operation_id = ?");
        }
        if query.namespace.is_some() {
            predicates.push("namespace = ?");
        }
        if query.operation_kind.is_some() {
            predicates.push("operation_kind = ?");
        }
        if query.status.is_some() {
            predicates.push("status = ?");
        }
        if query.correlation_id.is_some() {
            predicates.push("correlation_id = ?");
        }
        if query.after.is_some() {
            predicates.push("occurred_at > ?");
        }
        if query.before.is_some() {
            predicates.push("occurred_at < ?");
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        sql.push_str(" ORDER BY occurred_at DESC, operation_id DESC");
        if query.limit.is_some() {
            sql.push_str(" LIMIT ?");
        }

        let mut query_builder = sqlx::query(&sql);

        if let Some(ref operation_id) = query.operation_id {
            query_builder = query_builder.bind(operation_id.clone());
        }
        if let Some(ref ns) = query.namespace {
            query_builder = query_builder.bind(ns.clone());
        }
        if let Some(ref kind) = query.operation_kind {
            query_builder = query_builder.bind(kind.clone());
        }
        if let Some(ref status) = query.status {
            query_builder = query_builder.bind(status.clone());
        }
        if let Some(ref correlation_id) = query.correlation_id {
            query_builder = query_builder.bind(correlation_id.clone());
        }
        if let Some(after) = query.after {
            query_builder = query_builder.bind(after.to_rfc3339());
        }
        if let Some(before) = query.before {
            query_builder = query_builder.bind(before.to_rfc3339());
        }
        if let Some(limit) = query.limit {
            let limit = i64::try_from(limit).map_err(|_| {
                AppError::InvalidParams(
                    "operation log query limit exceeds the supported maximum".to_string(),
                )
            })?;
            query_builder = query_builder.bind(limit);
        }

        let rows = map_sqlite(query_builder.fetch_all(&self.pool).await)?;

        rows.iter()
            .map(|row| {
                let occurred_at_str: String = row.get("occurred_at");
                let occurred_at = DateTime::parse_from_rfc3339(&occurred_at_str)
                    .map_err(|e| AppError::Message(e.to_string()))?
                    .with_timezone(&Utc);
                let actor_kind_str: String = row.get("actor_kind");
                let operation_kind_str: String = row.get("operation_kind");
                let status_str: String = row.get("status");

                Ok(OperationLogEntry {
                    operation_id: row.get("operation_id"),
                    occurred_at,
                    namespace: row.get("namespace"),
                    actor_kind: parse_actor_kind(&actor_kind_str)?,
                    actor_id: row.get("actor_id"),
                    entrypoint: row.get("entrypoint"),
                    operation_kind: parse_operation_log_kind(&operation_kind_str)?,
                    status: parse_operation_log_status(&status_str)?,
                    correlation_id: row.get("correlation_id"),
                    request_summary_json: row.get("request_summary_json"),
                    response_summary_json: row.get("response_summary_json"),
                    diagnostic_summary_json: row.get("diagnostic_summary_json"),
                    redaction_version: row.get("redaction_version"),
                })
            })
            .collect()
    }
}

fn parse_actor_kind(value: &str) -> Result<ActorKind, AppError> {
    match value {
        "system" => Ok(ActorKind::System),
        "user" => Ok(ActorKind::User),
        "model" => Ok(ActorKind::Model),
        "hook" => Ok(ActorKind::Hook),
        _ => Err(AppError::Message(format!("unknown actor kind: {value}"))),
    }
}

fn parse_operation_log_kind(value: &str) -> Result<OperationLogKind, AppError> {
    match value {
        "startup" => Ok(OperationLogKind::Startup),
        "tool" => Ok(OperationLogKind::Tool),
        "trigger" => Ok(OperationLogKind::Trigger),
        "reflection" => Ok(OperationLogKind::Reflection),
        "decision" => Ok(OperationLogKind::Decision),
        "snapshot" => Ok(OperationLogKind::Snapshot),
        "doctor" => Ok(OperationLogKind::Doctor),
        "error" => Ok(OperationLogKind::Error),
        _ => Err(AppError::Message(format!(
            "unknown operation log kind: {value}"
        ))),
    }
}

fn parse_operation_log_status(value: &str) -> Result<OperationLogStatus, AppError> {
    match value {
        "started" => Ok(OperationLogStatus::Started),
        "ok" => Ok(OperationLogStatus::Ok),
        "handled" => Ok(OperationLogStatus::Handled),
        "suppressed" => Ok(OperationLogStatus::Suppressed),
        "rejected" => Ok(OperationLogStatus::Rejected),
        "failed" => Ok(OperationLogStatus::Failed),
        _ => Err(AppError::Message(format!(
            "unknown operation log status: {value}"
        ))),
    }
}
