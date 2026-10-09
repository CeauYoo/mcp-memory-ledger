use super::SqliteStore;
use crate::{
    domain::{feedback_candidate::FeedbackCandidate, types::Namespace},
    error::AppError,
    ports::feedback_candidate_store::FeedbackCandidateStore,
};
use async_trait::async_trait;
use sqlx::{Row, Sqlite, sqlite::SqliteRow};

/// Canonical table definition consumed by explicit schema lifecycle migration.
pub(super) const FEEDBACK_CANDIDATE_SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS feedback_candidates (
    candidate_id TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    target_claim_id TEXT NOT NULL,
    expected_target_version TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('proposed', 'validated', 'blocked', 'rejected', 'committed')),
    revision INTEGER NOT NULL CHECK (revision >= 0),
    candidate_json TEXT NOT NULL,
    FOREIGN KEY (target_claim_id) REFERENCES claims(claim_id)
)"#;

fn sqlite_error(error: sqlx::Error) -> AppError {
    AppError::Message(error.to_string())
}
fn decode(row: SqliteRow) -> Result<FeedbackCandidate, AppError> {
    serde_json::from_str(&row.get::<String, _>("candidate_json"))
        .map_err(|error| AppError::Message(format!("invalid feedback candidate: {error}")))
}
#[async_trait]
impl FeedbackCandidateStore for SqliteStore {
    async fn get_feedback_candidate(
        &self,
        namespace: &Namespace,
        candidate_id: &str,
    ) -> Result<Option<FeedbackCandidate>, AppError> {
        sqlx::query("SELECT candidate_json FROM feedback_candidates WHERE candidate_id = ? AND namespace = ?")
            .bind(candidate_id).bind(namespace.as_str()).fetch_optional(&self.pool).await.map_err(sqlite_error)?.map(decode).transpose()
    }
}
pub(super) async fn load<'e, E: sqlx::Executor<'e, Database = Sqlite>>(
    executor: E,
    namespace: &Namespace,
    candidate_id: &str,
) -> Result<Option<FeedbackCandidate>, AppError> {
    sqlx::query(
        "SELECT candidate_json FROM feedback_candidates WHERE candidate_id = ? AND namespace = ?",
    )
    .bind(candidate_id)
    .bind(namespace.as_str())
    .fetch_optional(executor)
    .await
    .map_err(sqlite_error)?
    .map(decode)
    .transpose()
}
pub(super) async fn list_for_target<'e, E: sqlx::Executor<'e, Database = Sqlite>>(
    executor: E,
    namespace: &Namespace,
    target: &str,
    version: &str,
) -> Result<Vec<FeedbackCandidate>, AppError> {
    sqlx::query("SELECT candidate_json FROM feedback_candidates WHERE namespace = ? AND target_claim_id = ? AND expected_target_version = ? ORDER BY candidate_id")
        .bind(namespace.as_str()).bind(target).bind(version).fetch_all(executor).await.map_err(sqlite_error)?.into_iter().map(decode).collect()
}
pub(super) async fn insert<'e, E: sqlx::Executor<'e, Database = Sqlite>>(
    executor: E,
    candidate: &FeedbackCandidate,
) -> Result<(), AppError> {
    let json = serde_json::to_string(candidate).map_err(|e| AppError::Message(e.to_string()))?;
    sqlx::query("INSERT INTO feedback_candidates (candidate_id, namespace, target_claim_id, expected_target_version, state, revision, candidate_json) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&candidate.candidate_id).bind(candidate.proposal.namespace.as_str()).bind(candidate.proposal.target_claim_reference.claim_id()).bind(&candidate.proposal.expected_target_version)
        .bind(candidate.state.as_str()).bind(candidate.revision as i64).bind(json).execute(executor).await.map_err(sqlite_error)?;
    Ok(())
}
pub(super) async fn update(
    executor: &mut sqlx::SqliteConnection,
    candidate: &FeedbackCandidate,
    expected_revision: u64,
) -> Result<(), AppError> {
    if candidate.revision != expected_revision + 1 {
        return Err(AppError::InvalidParams(
            "candidate revision must advance exactly once".into(),
        ));
    }
    let current = load(
        &mut *executor,
        &candidate.proposal.namespace,
        &candidate.candidate_id,
    )
    .await?
    .ok_or_else(|| AppError::InvalidParams("feedback candidate lifecycle conflict".into()))?;
    use crate::domain::feedback_candidate::FeedbackCandidateState;
    if current.proposal != candidate.proposal
        || current.created_at != candidate.created_at
        || current.revision != expected_revision
        || current.state.terminal()
        || candidate.state == FeedbackCandidateState::Proposed
        || (candidate.state == FeedbackCandidateState::Committed
            && current.state != FeedbackCandidateState::Validated)
    {
        return Err(AppError::InvalidParams(
            "feedback candidate proposal is immutable and transitions must preserve its lifecycle"
                .into(),
        ));
    }
    let json = serde_json::to_string(candidate).map_err(|e| AppError::Message(e.to_string()))?;
    let changed = sqlx::query("UPDATE feedback_candidates SET state = ?, revision = ?, candidate_json = ? WHERE candidate_id = ? AND namespace = ? AND revision = ? AND state NOT IN ('rejected', 'committed')")
        .bind(candidate.state.as_str()).bind(candidate.revision as i64).bind(json).bind(&candidate.candidate_id).bind(candidate.proposal.namespace.as_str()).bind(expected_revision as i64).execute(executor).await.map_err(sqlite_error)?.rows_affected();
    if changed != 1 {
        return Err(AppError::InvalidParams(
            "feedback candidate lifecycle conflict".into(),
        ));
    }
    Ok(())
}
