use crate::{
    domain::{experience::ExperienceWriteResult, experience_draft::*},
    error::AppError,
};
use async_trait::async_trait;
#[async_trait]
pub trait ExperienceDraftStore {
    /// Commit before calling a model. Only a fresh reservation may invoke it.
    async fn reserve_experience_draft(
        &self,
        request: GenerateExperienceDraftRequest,
    ) -> Result<ExperienceWriteResult<ExperienceDraftInput>, AppError>;
    async fn experience_draft_completion(
        &self,
        request: &GenerateExperienceDraftRequest,
    ) -> Result<Option<DraftCompletion>, AppError>;
    /// Immutable terminal result; raw provider errors are never stored.
    async fn complete_experience_draft(
        &self,
        request: &GenerateExperienceDraftRequest,
        completion: DraftCompletion,
    ) -> Result<(), AppError>;
}
