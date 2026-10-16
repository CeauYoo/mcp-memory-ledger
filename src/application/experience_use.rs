//! Knowledge observations only. No execution, permissions, or causal attribution.
pub use crate::domain::experience_use::*;
use crate::{
    domain::experience::ExperienceWriteResult, error::AppError,
    ports::experience_use_store::ExperienceUseStore,
};
pub async fn record_experience_use<S: ExperienceUseStore + Sync>(
    store: &S,
    request: RecordExperienceUseRequest,
) -> Result<ExperienceWriteResult<ExperienceUse>, AppError> {
    request.validate()?;
    store.record_experience_use(request).await
}
pub async fn list_experience_uses<S: ExperienceUseStore + Sync>(
    store: &S,
    request: ListExperienceUsesRequest,
) -> Result<Vec<ExperienceUseView>, AppError> {
    request.validate()?;
    store.list_experience_uses(request).await
}

/// Returns the real completed steps; retry the identical request after fixing a failure.
/// Each existing write has its own atomic receipt. This is not one all-or-nothing transaction.
pub async fn derive_experience_from_use<
    S: ExperienceUseStore + crate::ports::experience_store::ExperienceStore + Sync,
>(
    store: &S,
    r: DeriveExperienceRequest,
) -> Result<DeriveExperienceResult, AppError> {
    r.validate()?;
    let reservation = store.prepare_experience_derivation(r.clone()).await?;
    let mut out = DeriveExperienceResult {
        reservation_id: reservation.operation_id.clone(),
        use_id: r.use_id,
        episode_id: r.episode_id.clone(),
        candidate_id: r.candidate_id.clone(),
        episode: None,
        candidate: None,
        stage: "reserved".into(),
        retryable_error: None,
    };
    match super::experience::create_episode(
        store,
        crate::domain::experience::CreateEpisodeRequest {
            namespace: r.namespace.clone(),
            request_id: format!("{}:episode", reservation.operation_id),
            episode_id: r.episode_id,
            content: r.episode_content,
        },
    )
    .await
    {
        Ok(episode) => {
            out.episode = Some(episode);
            out.stage = "episode_created".into();
        }
        Err(e) => {
            out.retryable_error = Some(e.to_string());
            return Ok(out);
        }
    }
    match super::experience::create_candidate(
        store,
        crate::domain::experience::CreateCandidateRequest {
            namespace: r.namespace,
            request_id: format!("{}:candidate", reservation.operation_id),
            candidate_id: r.candidate_id,
            content: r.candidate_content,
        },
    )
    .await
    {
        Ok(candidate) => {
            out.candidate = Some(candidate);
            out.stage = "pending_candidate_created".into();
        }
        Err(e) => out.retryable_error = Some(e.to_string()),
    }
    Ok(out)
}
