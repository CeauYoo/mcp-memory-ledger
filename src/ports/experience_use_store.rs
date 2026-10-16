use crate::{
    domain::{experience::ExperienceWriteResult, experience_use::*},
    error::AppError,
};
use async_trait::async_trait;
#[async_trait]
pub trait ExperienceUseStore {
    /// Atomically reserve the full composition hash, validating source membership.
    async fn prepare_experience_derivation(
        &self,
        request: DeriveExperienceRequest,
    ) -> Result<ExperienceWriteResult<DerivationReservation>, AppError>;
    async fn record_experience_use(
        &self,
        request: RecordExperienceUseRequest,
    ) -> Result<ExperienceWriteResult<ExperienceUse>, AppError>;
    async fn list_experience_uses(
        &self,
        request: ListExperienceUsesRequest,
    ) -> Result<Vec<ExperienceUseView>, AppError>;
}
