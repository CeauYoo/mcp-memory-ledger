//! Atomic ingest/reflection transactions and ledger row mutations.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{Row, Sqlite};

use crate::{
    domain::{commitment::Commitment, identity_core::IdentityCore, types::Namespace},
    error::AppError,
    ports::{
        ClaimStatus, IngestTransaction, IngestTransactionRunner, ReflectionTransaction,
        ReflectionTransactionRunner, StoredClaim, StoredEvent, StoredReflection,
        StoredTriggerLedgerEntry, StoredWriteReceipt, WriteReceiptRequest,
    },
};

use super::reads::{load_commitment_rows, load_identity_rows};

use super::rows::{
    deserialize_json, event_kind_as_str, map_sqlite, mode_as_str, owner_as_str,
    serialize_episode_watermark, serialize_json, serialize_optional_json, stored_claim_from_row,
    stored_event_from_row, trigger_type_as_str,
};

use super::SqliteStore;

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
        let result = crate::adapters::sqlite::feedback_candidate::load(
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
        let result = crate::adapters::sqlite::feedback_candidate::list_for_target(
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
        let result = crate::adapters::sqlite::feedback_candidate::insert(
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
        let result = crate::adapters::sqlite::feedback_candidate::update(
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

pub(super) async fn insert_event<'e, E>(executor: E, event: &StoredEvent) -> Result<(), AppError>
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

pub(super) async fn upsert_claim_row<'e, E>(
    executor: E,
    claim: &StoredClaim,
) -> Result<(), AppError>
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

pub(super) async fn insert_evidence_link<'e, E>(
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

pub(super) async fn insert_episode_event<'e, E>(
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

pub(super) async fn insert_reflection(
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

    crate::adapters::sqlite::reflection_scope::persist_relations(connection, reflection).await
}

pub(super) async fn insert_trigger_ledger_entry<'e, E>(
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

pub(super) async fn update_claim_status_row<'e, E>(
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

pub(super) async fn replace_identity_rows(
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
