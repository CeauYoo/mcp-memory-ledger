//! Adapter port implementations; mutations delegate to shared transaction write helpers.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{QueryBuilder, Row, Sqlite};

use crate::{
    domain::{
        commitment::Commitment,
        event::{EventReference, MAX_EVIDENCE_MANIFEST_ITEMS},
        identity_core::IdentityCore,
        snapshot::SnapshotTimeWindow,
        types::MemoryScope,
    },
    error::AppError,
    ports::{
        ClaimStatus, ClaimStore, CommitmentStore, EpisodeStore, EventStore, EvidenceQuery,
        IdentityStore, ReflectionStore, StoredClaim, StoredEvent, StoredReflection,
        StoredTriggerLedgerEntry, TriggerLedgerStatus, TriggerLedgerStore,
    },
};

use super::reads::{load_commitment_rows, load_identity_rows, query_evidence_event_ids_with_limit};

use super::transactions::{
    insert_episode_event, insert_event, insert_evidence_link, insert_reflection,
    insert_trigger_ledger_entry, replace_identity_rows, update_claim_status_row, upsert_claim_row,
};

use super::rows::{
    map_sqlite, owner_as_str, parse_timestamp, stored_claim_from_row,
    stored_trigger_ledger_entry_from_row, utc_timestamp_sort_key,
};

use super::SqliteStore;

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
