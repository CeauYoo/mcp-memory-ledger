//! Explicit single-attempt generation with durable reservations and recoverable derivation.
use crate::{
    domain::{experience_draft::*, experience_use::DeriveExperienceRequest},
    error::AppError,
    ports::{
        ModelPort, experience_draft_store::ExperienceDraftStore, experience_store::ExperienceStore,
        experience_use_store::ExperienceUseStore,
    },
};
pub async fn generate_experience_draft<
    S: ExperienceDraftStore + ExperienceUseStore + ExperienceStore + Sync,
    M: ModelPort + Sync + ?Sized,
>(
    store: &S,
    model: &M,
    r: GenerateExperienceDraftRequest,
) -> Result<GenerateExperienceDraftResult, AppError> {
    r.validate()?;
    let reservation = store.reserve_experience_draft(r.clone()).await?;
    let mut out = GenerateExperienceDraftResult {
        reservation_id: reservation.operation_id.clone(),
        replayed: reservation.replayed,
        status: ExperienceDraftStatus::IncompleteOrInFlight,
        derivation: None,
    };
    let completion = if reservation.replayed {
        store.experience_draft_completion(&r).await?
    } else {
        let completion = match model
            .propose_experience_draft(reservation.record.clone())
            .await
        {
            Ok(proposal) if proposal.validate(&reservation.record).is_ok() => {
                DraftCompletion::SavedProposal {
                    proposal: Box::new(proposal),
                }
            }
            Ok(_) => DraftCompletion::Rejected,
            Err(AppError::InvalidParams(_)) => DraftCompletion::Rejected,
            Err(_) => DraftCompletion::KnownFailed,
        };
        store
            .complete_experience_draft(&r, completion.clone())
            .await?;
        Some(completion)
    };
    match completion {
        None => {}
        Some(DraftCompletion::KnownFailed) => out.status = ExperienceDraftStatus::KnownFailed,
        Some(DraftCompletion::Rejected) => out.status = ExperienceDraftStatus::Rejected,
        Some(DraftCompletion::SavedProposal { proposal }) => {
            proposal.validate(&reservation.record)?;
            out.status = ExperienceDraftStatus::SavedProposal;
            let result = super::experience_use::derive_experience_from_use(
                store,
                DeriveExperienceRequest {
                    namespace: r.namespace,
                    request_id: format!("{}:derive", reservation.operation_id),
                    use_id: r.use_id,
                    episode_id: r.episode_id,
                    candidate_id: r.candidate_id,
                    episode_content: proposal.episode_content,
                    candidate_content: proposal.candidate_content,
                },
            )
            .await?;
            if result.candidate.is_some() {
                out.status = ExperienceDraftStatus::PendingCandidateCreated;
            }
            out.derivation = Some(result);
        }
    }
    Ok(out)
}
