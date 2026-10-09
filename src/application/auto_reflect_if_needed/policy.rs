//! Deterministic suppression and governed identity/commitment patch validation.
use super::{
    AUTO_REFLECTION_COOLDOWN_HOURS,
    candidate::{TriggerCandidate, dedupe_strings},
};
use crate::{
    domain::{
        commitment::Commitment,
        self_revision::{SelfRevisionProposal, SuppressionCategory, TriggerType},
        types::Owner,
    },
    error::AppError,
    ports::{ClaimStore, Clock, EpisodeStore, TriggerLedgerStatus, TriggerLedgerStore},
};
use chrono::{Duration, Utc};

#[derive(Debug, Clone)]
pub(super) struct ValidatedSelfRevision {
    pub(super) identity_claims: Option<Vec<String>>,
    pub(super) commitments: Option<Vec<Commitment>>,
}

#[derive(Debug, Clone)]
struct IdentityRevisionContext {
    proposed_values: Vec<String>,
    supporting_claim_count: usize,
    cross_episode_support_count: usize,
    has_high_conflict: bool,
    now: chrono::DateTime<Utc>,
    latest_handled_at: Option<chrono::DateTime<Utc>>,
    cooldown_until: Option<chrono::DateTime<Utc>>,
    patch_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SuppressionDecision {
    pub(super) reason: SuppressionCategory,
}

pub(super) async fn evaluate_trigger_suppression<D>(
    deps: &D,
    candidate: &TriggerCandidate,
) -> Result<Option<SuppressionDecision>, AppError>
where
    D: TriggerLedgerStore + Clock + Sync,
{
    let Some(latest) = deps.latest_trigger_entry(&candidate.trigger_key).await? else {
        return Ok(None);
    };

    let now = deps.now().await?;
    if matches!(
        latest.status,
        TriggerLedgerStatus::Handled | TriggerLedgerStatus::Suppressed
    ) && latest
        .cooldown_until
        .is_some_and(|cooldown_until| cooldown_until > now)
    {
        return Ok(Some(SuppressionDecision {
            reason: SuppressionCategory::CooldownActive,
        }));
    }

    let latest_handled = deps
        .latest_handled_trigger_entry(&candidate.trigger_key)
        .await?;

    if !candidate.evidence_event_ids.is_empty()
        && latest_handled
            .as_ref()
            .is_some_and(|entry| entry.evidence_window == candidate.evidence_event_ids)
    {
        return Ok(Some(SuppressionDecision {
            reason: SuppressionCategory::EvidenceWindowUnchanged,
        }));
    }

    if candidate.trigger_type == TriggerType::Periodic
        && latest_handled.as_ref().is_some_and(|entry| {
            entry.episode_watermark.unwrap_or_default()
                >= candidate.episode_watermark.unwrap_or_default()
        })
    {
        return Ok(Some(SuppressionDecision {
            reason: SuppressionCategory::EpisodeWatermarkUnchanged,
        }));
    }

    Ok(None)
}

pub(super) async fn validate_self_revision<D>(
    deps: &D,
    candidate: &TriggerCandidate,
    proposal: &SelfRevisionProposal,
    governed_evidence_event_ids: &[String],
) -> Result<ValidatedSelfRevision, AppError>
where
    D: ClaimStore + EpisodeStore + TriggerLedgerStore + Clock + Sync,
{
    let identity_claims = if proposal.machine_patch.identity_patch.is_some() {
        Some(validate_identity_patch(
            proposal,
            &build_identity_revision_context(deps, candidate, proposal).await?,
        )?)
    } else {
        None
    };

    let commitments = if let Some(commitment_patch) = &proposal.machine_patch.commitment_patch {
        if governed_evidence_event_ids.is_empty() {
            return Err(AppError::InvalidParams(
                "commitment auto-reflection updates require supporting evidence".to_string(),
            ));
        }

        if commitment_patch.commitments.is_empty() {
            return Err(AppError::InvalidParams(
                "commitment auto-reflection patch must include at least one commitment".to_string(),
            ));
        }

        Some(
            commitment_patch
                .commitments
                .iter()
                .cloned()
                .map(|commitment| Commitment::new(Owner::Self_, commitment))
                .collect(),
        )
    } else {
        None
    };

    if identity_claims.is_none() && commitments.is_none() {
        return Err(AppError::InvalidParams(
            "auto-reflection proposals must include at least one governed patch".to_string(),
        ));
    }

    Ok(ValidatedSelfRevision {
        identity_claims,
        commitments,
    })
}

async fn build_identity_revision_context<D>(
    deps: &D,
    candidate: &TriggerCandidate,
    proposal: &SelfRevisionProposal,
) -> Result<IdentityRevisionContext, AppError>
where
    D: ClaimStore + EpisodeStore + TriggerLedgerStore + Clock + Sync,
{
    let active_claims = deps.list_active_claims().await?;
    let proposed_identity_claims = proposal
        .machine_patch
        .identity_patch
        .as_ref()
        .map(|patch| patch.canonical_claims.clone())
        .unwrap_or_default();
    let proposed_values = dedupe_strings(
        proposed_identity_claims
            .iter()
            .filter_map(|claim| claim.rsplit_once('=').map(|(_, value)| value.to_string()))
            .collect(),
    );
    let supporting_claims: Vec<_> = active_claims
        .iter()
        .filter(|claim| {
            claim.claim.namespace() == &candidate.namespace
                && proposed_values
                    .iter()
                    .any(|value| value == claim.claim.object())
        })
        .cloned()
        .collect();
    let supporting_claim_count = supporting_claims.len();
    let supporting_shapes = supporting_claims
        .iter()
        .fold(Vec::new(), |mut shapes, claim| {
            let shape = (
                claim.claim.subject().to_string(),
                claim.claim.predicate().to_string(),
            );
            if !shapes.contains(&shape) {
                shapes.push(shape);
            }
            shapes
        });
    let has_high_conflict = active_claims.iter().any(|claim| {
        claim.claim.namespace() == &candidate.namespace
            && supporting_shapes.iter().any(|(subject, predicate)| {
                claim.claim.subject() == subject
                    && claim.claim.predicate() == predicate
                    && !proposed_values
                        .iter()
                        .any(|value| value == claim.claim.object())
            })
    });
    let latest_entry = deps.latest_trigger_entry(&candidate.trigger_key).await?;
    let supporting_claim_ids = supporting_claims
        .iter()
        .map(|claim| claim.claim_id.clone())
        .collect::<Vec<_>>();
    let cross_episode_support_count = dedupe_strings(
        deps.list_episode_references_supporting_claims(&candidate.scope, &supporting_claim_ids)
            .await?,
    )
    .len();
    let now = deps.now().await?;

    Ok(IdentityRevisionContext {
        proposed_values,
        supporting_claim_count,
        cross_episode_support_count,
        has_high_conflict,
        now,
        latest_handled_at: latest_entry.as_ref().and_then(|entry| {
            (entry.status == TriggerLedgerStatus::Handled)
                .then_some(entry.handled_at)
                .flatten()
        }),
        cooldown_until: latest_entry.as_ref().and_then(|entry| {
            matches!(
                entry.status,
                TriggerLedgerStatus::Handled | TriggerLedgerStatus::Suppressed
            )
            .then_some(entry.cooldown_until)
            .flatten()
        }),
        patch_size: proposed_identity_claims.len(),
    })
}

fn validate_identity_patch(
    proposal: &SelfRevisionProposal,
    context: &IdentityRevisionContext,
) -> Result<Vec<String>, AppError> {
    ensure_min_supporting_claims(context, 3)?;
    ensure_cross_episode_support(context, 2)?;
    ensure_no_high_conflict(context)?;
    ensure_identity_cooldown_elapsed(context)?;
    ensure_identity_patch_limit(context, 2)?;
    Ok(materialize_identity_claims(proposal))
}

fn ensure_min_supporting_claims(
    context: &IdentityRevisionContext,
    minimum: usize,
) -> Result<(), AppError> {
    if context.supporting_claim_count < minimum {
        return Err(AppError::InvalidParams(format!(
            "identity auto-reflection requires at least {minimum} supporting claims"
        )));
    }

    Ok(())
}

fn ensure_cross_episode_support(
    context: &IdentityRevisionContext,
    minimum: usize,
) -> Result<(), AppError> {
    if context.cross_episode_support_count < minimum {
        return Err(AppError::InvalidParams(format!(
            "identity auto-reflection requires support across at least {minimum} episodes"
        )));
    }

    Ok(())
}

fn ensure_no_high_conflict(context: &IdentityRevisionContext) -> Result<(), AppError> {
    if context.has_high_conflict {
        return Err(AppError::InvalidParams(format!(
            "identity auto-reflection cannot proceed while high-conflict evidence remains active for {:?}",
            context.proposed_values
        )));
    }

    Ok(())
}

fn ensure_identity_cooldown_elapsed(context: &IdentityRevisionContext) -> Result<(), AppError> {
    if context
        .cooldown_until
        .is_some_and(|cooldown_until| cooldown_until > context.now)
    {
        return Err(AppError::InvalidParams(
            "identity auto-reflection cooldown has not elapsed".to_string(),
        ));
    }

    if context.latest_handled_at.is_some_and(|handled_at| {
        handled_at + Duration::hours(AUTO_REFLECTION_COOLDOWN_HOURS) > context.now
    }) {
        return Err(AppError::InvalidParams(
            "identity auto-reflection handled too recently".to_string(),
        ));
    }

    Ok(())
}

fn ensure_identity_patch_limit(
    context: &IdentityRevisionContext,
    maximum: usize,
) -> Result<(), AppError> {
    if context.patch_size == 0 || context.patch_size > maximum {
        return Err(AppError::InvalidParams(format!(
            "identity auto-reflection patch must contain between 1 and {maximum} claims"
        )));
    }

    Ok(())
}

fn materialize_identity_claims(proposal: &SelfRevisionProposal) -> Vec<String> {
    proposal
        .machine_patch
        .identity_patch
        .as_ref()
        .map(|patch| patch.canonical_claims.clone())
        .unwrap_or_default()
}
