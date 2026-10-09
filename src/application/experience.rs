//! Local-first experience workflow. Caller-authored content is structurally checked;
//! source existence and scope do not prove its claims true. No model or execution.
pub use crate::domain::experience::*;
use crate::{error::AppError, ports::experience_store::ExperienceStore};

pub async fn create_episode<S: ExperienceStore + Sync>(
    store: &S,
    request: CreateEpisodeRequest,
) -> Result<ExperienceWriteResult<PersistedEpisode>, AppError> {
    validate_namespace(&request.namespace)?;
    identifier(&request.episode_id, "episode_id")?;
    request.content.validate()?;
    store.create_episode(request).await
}
pub async fn get_episode<S: ExperienceStore + Sync>(
    store: &S,
    request: GetEpisodeRequest,
) -> Result<Option<PersistedEpisode>, AppError> {
    validate_namespace(&request.namespace)?;
    identifier(&request.episode_id, "episode_id")?;
    store.get_episode(request).await
}
pub async fn list_episodes<S: ExperienceStore + Sync>(
    store: &S,
    request: ListEpisodesRequest,
) -> Result<Vec<PersistedEpisode>, AppError> {
    validate_namespace(&request.namespace)?;
    validate_limit(request.limit)?;
    store.list_episodes(request).await
}
pub async fn create_candidate<S: ExperienceStore + Sync>(
    store: &S,
    request: CreateCandidateRequest,
) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
    validate_namespace(&request.namespace)?;
    identifier(&request.candidate_id, "candidate_id")?;
    request.content.validate()?;
    store.create_candidate(request).await
}
pub async fn get_candidate<S: ExperienceStore + Sync>(
    store: &S,
    request: GetCandidateRequest,
) -> Result<Option<ExperienceCandidate>, AppError> {
    validate_namespace(&request.namespace)?;
    identifier(&request.candidate_id, "candidate_id")?;
    if let Some(version) = request.version {
        validate_version(version)?;
    }
    store.get_candidate(request).await
}
pub async fn list_candidates<S: ExperienceStore + Sync>(
    store: &S,
    request: ListCandidatesRequest,
) -> Result<Vec<ExperienceCandidate>, AppError> {
    validate_namespace(&request.namespace)?;
    validate_limit(request.limit)?;
    store.list_candidates(request).await
}
pub async fn update_candidate_status<S: ExperienceStore + Sync>(
    store: &S,
    request: UpdateCandidateStatusRequest,
) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
    validate_namespace(&request.namespace)?;
    identifier(&request.candidate_id, "candidate_id")?;
    validate_version(request.expected_version)?;
    store.update_candidate_status(request).await
}
pub async fn revise_candidate<S: ExperienceStore + Sync>(
    store: &S,
    request: ReviseCandidateRequest,
) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
    validate_namespace(&request.namespace)?;
    identifier(&request.candidate_id, "candidate_id")?;
    validate_version(request.expected_version)?;
    request.content.validate()?;
    store.revise_candidate(request).await
}
pub async fn rollback_candidate<S: ExperienceStore + Sync>(
    store: &S,
    request: RollbackCandidateRequest,
) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError> {
    validate_namespace(&request.namespace)?;
    identifier(&request.candidate_id, "candidate_id")?;
    validate_version(request.expected_version)?;
    validate_version(request.target_version)?;
    store.rollback_candidate(request).await
}
pub async fn recall_candidates<S: ExperienceStore + Sync>(
    store: &S,
    request: RecallCandidatesRequest,
) -> Result<ExperienceRecall, AppError> {
    recall_terms(&request)?;
    store.recall_candidates(request).await
}
