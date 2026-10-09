use crate::{domain::experience::*, error::AppError};
use async_trait::async_trait;

/// Atomic experience-only persistence. These capabilities deliberately expose no
/// identity, commitment, permission, model, network or action-execution operation.
#[async_trait]
pub trait ExperienceStore {
    async fn create_episode(
        &self,
        request: CreateEpisodeRequest,
    ) -> Result<ExperienceWriteResult<PersistedEpisode>, AppError>;
    async fn get_episode(
        &self,
        request: GetEpisodeRequest,
    ) -> Result<Option<PersistedEpisode>, AppError>;
    async fn list_episodes(
        &self,
        request: ListEpisodesRequest,
    ) -> Result<Vec<PersistedEpisode>, AppError>;
    async fn create_candidate(
        &self,
        request: CreateCandidateRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    async fn get_candidate(
        &self,
        request: GetCandidateRequest,
    ) -> Result<Option<ExperienceCandidate>, AppError>;
    async fn list_candidates(
        &self,
        request: ListCandidatesRequest,
    ) -> Result<Vec<ExperienceCandidate>, AppError>;
    async fn update_candidate_status(
        &self,
        request: UpdateCandidateStatusRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    async fn revise_candidate(
        &self,
        request: ReviseCandidateRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    async fn rollback_candidate(
        &self,
        request: RollbackCandidateRequest,
    ) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    async fn recall_candidates(
        &self,
        request: RecallCandidatesRequest,
    ) -> Result<ExperienceRecall, AppError>;
}
