//! Immutable reservation and terminal receipts reuse operation_log; no schema change.
use super::{
    SqliteStore,
    experience::{audit, replay as receipt_replay},
    experience_use,
};
use crate::{
    domain::{
        event::EventReference, experience::ExperienceWriteResult, experience_draft::*,
        task::invalid,
    },
    error::AppError,
    ports::{WriteReceiptRequest, experience_draft_store::ExperienceDraftStore},
};
use async_trait::async_trait;
use sqlx::Row;
fn db<T>(r: Result<T, sqlx::Error>) -> Result<T, AppError> {
    r.map_err(|_| AppError::Message("experience draft database operation failed".into()))
}
fn receipt(
    r: &GenerateExperienceDraftRequest,
    terminal: bool,
) -> Result<WriteReceiptRequest, AppError> {
    WriteReceiptRequest::new(
        if terminal {
            "experience_draft_completion"
        } else {
            "generate_experience_draft"
        },
        &r.namespace,
        &r.request_id,
        r,
    )
}
/// Preflight sizes before loading JSON, including manually corrupted receipt rows.
async fn replay<T: serde::de::DeserializeOwned>(
    connection: &mut sqlx::SqliteConnection,
    request: &WriteReceiptRequest,
) -> Result<Option<ExperienceWriteResult<T>>, AppError> {
    let sizes: Option<(i64, i64)> = db(sqlx::query_as(
        "SELECT length(CAST(request_summary_json AS BLOB)), length(CAST(response_summary_json AS BLOB)) \
         FROM operation_log WHERE operation_id=? AND namespace=? \
         AND actor_id='durable_write_receipt_v1' AND status='ok'",
    )
    .bind(&request.operation_id)
    .bind(&request.namespace)
    .fetch_optional(&mut *connection)
    .await)?;
    if sizes.is_some_and(|(metadata, result)| {
        !(0..=1024).contains(&metadata) || !(0..=139264).contains(&result)
    }) {
        return invalid("persisted draft receipt exceeds bounds");
    }
    receipt_replay(connection, request).await
}
#[async_trait]
impl ExperienceDraftStore for SqliteStore {
    async fn reserve_experience_draft(
        &self,
        r: GenerateExperienceDraftRequest,
    ) -> Result<ExperienceWriteResult<ExperienceDraftInput>, AppError> {
        r.validate()?;
        let key = receipt(&r, false)?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        if let Some(result) = replay::<ExperienceDraftInput>(&mut tx, &key).await? {
            result.record.validate()?;
            if result.record.request != r {
                return invalid("draft reservation request mismatch");
            }
            db(tx.commit().await)?;
            return Ok(result);
        }
        let source_use = experience_use::load(&mut tx, &r.namespace, &r.use_id)
            .await?
            .ok_or_else(|| AppError::InvalidParams("source use not found in exact scope".into()))?;
        let refs: std::collections::BTreeSet<String> = source_use
            .observation
            .action_event_refs
            .iter()
            .chain(&source_use.observation.outcome_event_refs)
            .map(|r| EventReference::parse(r).map(|e| e.event_id().to_owned()))
            .collect::<Result<_, _>>()?;
        let mut evidence = Vec::new();
        let mut bytes = 0usize;
        for id in refs {
            // Check size before fetching potentially large stored text into the prompt.
            let size: Option<i64> = db(sqlx::query_scalar(
                "SELECT length(CAST(summary AS BLOB)) + coalesce(length(CAST(feedback_json AS BLOB)), 0) + length(CAST(kind AS BLOB)) + length(CAST(recorded_at AS BLOB)) + coalesce(length(CAST(observed_at AS BLOB)), 0) \
                 FROM events WHERE event_id=? AND namespace=? AND owner='world'",
            )
            .bind(&id)
            .bind(&r.namespace)
            .fetch_optional(&mut *tx)
            .await)?;
            let size = size.ok_or_else(|| {
                AppError::InvalidParams("draft evidence missing in exact scope".into())
            })?;
            bytes = bytes.saturating_add(usize::try_from(size).unwrap_or(usize::MAX));
            if bytes > MAX_DRAFT_INPUT_BYTES {
                return invalid("draft evidence exceeds bounded input");
            }
            let row = db(sqlx::query(
                "SELECT kind,summary,feedback_json,recorded_at,observed_at FROM events \
                 WHERE event_id=? AND namespace=? AND owner='world'",
            )
            .bind(&id)
            .bind(&r.namespace)
            .fetch_one(&mut *tx)
            .await)?;
            evidence.push(DraftEvidence {
                event_id: id,
                payload: serde_json::json!({
                    "kind": row.get::<String, _>("kind"),
                    "summary": row.get::<String, _>("summary"),
                    "feedback_json": row.get::<Option<String>, _>("feedback_json"),
                    "recorded_at": row.get::<String, _>("recorded_at"),
                    "observed_at": row.get::<Option<String>, _>("observed_at"),
                }),
            });
        }
        let record = ExperienceDraftInput {
            request: r,
            source_use,
            evidence,
        };
        record.validate()?;
        let result = ExperienceWriteResult {
            record,
            replayed: false,
            operation_id: key.operation_id.clone(),
        };
        audit(&mut tx, &key, &result).await?;
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn experience_draft_completion(
        &self,
        r: &GenerateExperienceDraftRequest,
    ) -> Result<Option<DraftCompletion>, AppError> {
        r.validate()?;
        let mut tx = db(self.pool.begin().await)?;
        let result = replay::<DraftCompletion>(&mut tx, &receipt(r, true)?)
            .await?
            .map(|r| r.record);
        db(tx.commit().await)?;
        Ok(result)
    }
    async fn complete_experience_draft(
        &self,
        r: &GenerateExperienceDraftRequest,
        completion: DraftCompletion,
    ) -> Result<(), AppError> {
        r.validate()?;
        let mut tx = db(self.pool.begin_with("BEGIN IMMEDIATE").await)?;
        let reserved = replay::<ExperienceDraftInput>(&mut tx, &receipt(r, false)?)
            .await?
            .ok_or_else(|| {
                AppError::InvalidParams("draft completion requires durable reservation".into())
            })?;
        if let DraftCompletion::SavedProposal { proposal } = &completion {
            proposal.validate(&reserved.record)?;
        }
        let key = receipt(r, true)?;
        if let Some(previous) = replay::<DraftCompletion>(&mut tx, &key).await? {
            if previous.record != completion {
                return invalid("draft terminal result is immutable");
            }
        } else {
            audit(
                &mut tx,
                &key,
                &ExperienceWriteResult {
                    record: completion,
                    replayed: false,
                    operation_id: key.operation_id.clone(),
                },
            )
            .await?;
        }
        db(tx.commit().await)?;
        Ok(())
    }
}
