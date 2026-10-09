//! Minimal explicit feedback loop; no model call or authority/identity patch.
use crate::{
    domain::{
        claim::{ClaimDraft, ClaimReference},
        event::EventReference,
        feedback::MAX_FEEDBACK_ITEMS,
        feedback_candidate::{
            FeedbackCandidate, FeedbackCandidateProposal, FeedbackCandidateState,
            FeedbackValidation, claim_version, validate_candidate,
        },
        reflection::Reflection,
        types::{MemoryScope, Namespace},
    },
    error::AppError,
    ports::{
        ClaimRecordQuery, Clock, EventStore, IdGenerator, MemoryReadStore, ReflectionTransaction,
        ReflectionTransactionRunner, WriteReceiptRequest,
        feedback_candidate_store::FeedbackCandidateStore, write_receipt::receipt_result,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackTargetInput {
    #[schemars(with = "String")]
    pub namespace: Namespace,
    #[schemars(with = "String")]
    pub target_claim_reference: ClaimReference,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackTargetVersion {
    pub target_claim_reference: ClaimReference,
    pub target_version: String,
    pub status: crate::ports::ClaimStatus,
    pub current_object: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProposeFeedbackCandidateInput {
    #[schemars(with = "String")]
    pub namespace: Namespace,
    #[schemars(with = "String")]
    pub target_claim_reference: ClaimReference,
    pub expected_target_version: String,
    pub replacement_object: String,
    #[schemars(with = "Vec<String>")]
    pub evidence_event_ids: Vec<EventReference>,
    pub summary: String,
    pub request_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetFeedbackCandidateInput {
    #[schemars(with = "String")]
    pub namespace: Namespace,
    pub candidate_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackCandidateActionInput {
    #[schemars(with = "String")]
    pub namespace: Namespace,
    pub candidate_id: String,
    pub request_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RejectFeedbackCandidateInput {
    #[schemars(with = "String")]
    pub namespace: Namespace,
    pub candidate_id: String,
    pub reason: String,
    pub request_id: String,
}

pub async fn get_target_version<D: MemoryReadStore + Sync>(
    deps: &D,
    input: FeedbackTargetInput,
) -> Result<FeedbackTargetVersion, AppError> {
    let target = deps
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(input.namespace),
            claim_reference: Some(input.target_claim_reference.clone()),
            status: None,
            mode: None,
            limit: 1,
        })
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| {
            AppError::InvalidParams("feedback target was not found in the requested scope".into())
        })?
        .claim;
    Ok(FeedbackTargetVersion {
        target_claim_reference: input.target_claim_reference,
        target_version: claim_version(&target)?,
        status: target.status,
        current_object: target.claim.object().into(),
    })
}
pub async fn get<D: FeedbackCandidateStore + Sync>(
    deps: &D,
    input: GetFeedbackCandidateInput,
) -> Result<FeedbackCandidate, AppError> {
    deps.get_feedback_candidate(&input.namespace, &input.candidate_id)
        .await?
        .ok_or_else(|| {
            AppError::InvalidParams(
                "feedback candidate was not found in the requested scope".into(),
            )
        })
}
pub async fn propose<D: ReflectionTransactionRunner + IdGenerator + Clock + Sync>(
    deps: &D,
    mut input: ProposeFeedbackCandidateInput,
) -> Result<FeedbackCandidate, AppError> {
    require_text(&input.summary, "summary")?;
    require_text(&input.replacement_object, "replacement_object")?;
    require_text(&input.expected_target_version, "expected_target_version")?;
    if input.evidence_event_ids.len() > MAX_FEEDBACK_ITEMS {
        return Err(AppError::InvalidParams(format!(
            "feedback candidates support at most {MAX_FEEDBACK_ITEMS} evidence events"
        )));
    }
    input
        .evidence_event_ids
        .sort_by(|a, b| a.event_id().cmp(b.event_id()));
    input.evidence_event_ids.dedup();
    let request = WriteReceiptRequest::new(
        "propose_feedback_candidate",
        input.namespace.as_str(),
        &input.request_id,
        &input,
    )?;
    let candidate_id = deps.next_id().await?;
    let now = deps.now().await?;
    let mut tx = deps.begin_reflection_transaction().await?;
    if let Some(receipt) = tx.load_write_receipt(&request.operation_id).await? {
        let result = receipt.replay(&request)?;
        tx.commit().await?;
        return Ok(result);
    }
    let target = tx
        .load_claim_for_reflection(input.target_claim_reference.claim_id())
        .await?
        .filter(|target| {
            target.claim.namespace() == &input.namespace
                && target.claim.owner() == input.namespace.derived_owner()
        })
        .ok_or_else(|| {
            AppError::InvalidParams("feedback target was not found in the requested scope".into())
        })?;
    let replacement_claim = ClaimDraft::new_with_namespace(
        target.claim.owner(),
        input.namespace.clone(),
        target.claim.subject(),
        target.claim.predicate(),
        input.replacement_object,
        target.claim.mode(),
    );
    let proposal = FeedbackCandidateProposal {
        namespace: input.namespace.clone(),
        target_claim_reference: input.target_claim_reference,
        expected_target_version: input.expected_target_version,
        replacement_claim,
        evidence_event_ids: input.evidence_event_ids,
        summary: input.summary,
    };
    let prior = tx
        .list_feedback_candidates_for_target(
            &proposal.namespace,
            proposal.target_claim_reference.claim_id(),
            &proposal.expected_target_version,
        )
        .await?;
    if let Some(existing) = prior
        .iter()
        .find(|candidate| candidate.proposal == proposal)
    {
        let result = existing.clone();
        tx.append_write_receipt(&request, receipt_result(&request, &result)?, now)
            .await?;
        tx.commit().await?;
        return Ok(result);
    }
    if !prior.is_empty()
        && proposal.evidence_event_ids.iter().all(|event| {
            prior
                .iter()
                .any(|candidate| candidate.proposal.evidence_event_ids.contains(event))
        })
    {
        return Err(AppError::InvalidParams("no_new_evidence: a new candidate for this target version requires at least one new evidence event".into()));
    }
    let candidate = FeedbackCandidate {
        candidate_id,
        proposal,
        state: FeedbackCandidateState::Proposed,
        validation: FeedbackValidation::pending(),
        created_at: now,
        updated_at: now,
        revision: 0,
        rejection_reason: None,
        reflection_id: None,
        replacement_claim_id: None,
    };
    tx.insert_feedback_candidate(&candidate).await?;
    tx.append_write_receipt(&request, receipt_result(&request, &candidate)?, now)
        .await?;
    tx.commit().await?;
    Ok(candidate)
}

pub async fn validate<D: ReflectionTransactionRunner + Clock + Sync>(
    deps: &D,
    input: FeedbackCandidateActionInput,
) -> Result<FeedbackCandidate, AppError> {
    let request = WriteReceiptRequest::new(
        "validate_feedback_candidate",
        input.namespace.as_str(),
        &input.request_id,
        &input,
    )?;
    let now = deps.now().await?;
    let mut tx = deps.begin_reflection_transaction().await?;
    if let Some(receipt) = tx.load_write_receipt(&request.operation_id).await? {
        let result = receipt.replay(&request)?;
        tx.commit().await?;
        return Ok(result);
    }
    let mut candidate = load_candidate(tx.as_mut(), &input.namespace, &input.candidate_id).await?;
    if candidate.state.terminal() {
        return Err(AppError::InvalidParams(
            "terminal feedback candidate cannot be validated again".into(),
        ));
    }
    candidate.validation = validate_in_transaction(tx.as_mut(), &candidate.proposal).await?;
    candidate.state = if candidate.validation.passed {
        FeedbackCandidateState::Validated
    } else {
        FeedbackCandidateState::Blocked
    };
    let revision = candidate.revision;
    candidate.revision += 1;
    candidate.updated_at = now;
    tx.update_feedback_candidate(&candidate, revision).await?;
    tx.append_write_receipt(&request, receipt_result(&request, &candidate)?, now)
        .await?;
    tx.commit().await?;
    Ok(candidate)
}

pub async fn reject<D: ReflectionTransactionRunner + Clock + Sync>(
    deps: &D,
    input: RejectFeedbackCandidateInput,
) -> Result<FeedbackCandidate, AppError> {
    require_text(&input.reason, "reason")?;
    let request = WriteReceiptRequest::new(
        "reject_feedback_candidate",
        input.namespace.as_str(),
        &input.request_id,
        &input,
    )?;
    let now = deps.now().await?;
    let mut tx = deps.begin_reflection_transaction().await?;
    if let Some(receipt) = tx.load_write_receipt(&request.operation_id).await? {
        let result = receipt.replay(&request)?;
        tx.commit().await?;
        return Ok(result);
    }
    let mut candidate = load_candidate(tx.as_mut(), &input.namespace, &input.candidate_id).await?;
    if candidate.state.terminal() {
        return Err(AppError::InvalidParams(
            "terminal feedback candidate cannot be rejected again".into(),
        ));
    }
    let revision = candidate.revision;
    candidate.revision += 1;
    candidate.state = FeedbackCandidateState::Rejected;
    candidate.rejection_reason = Some(input.reason);
    candidate.updated_at = now;
    tx.update_feedback_candidate(&candidate, revision).await?;
    tx.append_write_receipt(&request, receipt_result(&request, &candidate)?, now)
        .await?;
    tx.commit().await?;
    Ok(candidate)
}

pub async fn commit<D>(
    deps: &D,
    input: FeedbackCandidateActionInput,
) -> Result<FeedbackCandidate, AppError>
where
    D: FeedbackCandidateStore
        + ReflectionTransactionRunner
        + EventStore
        + IdGenerator
        + Clock
        + Sync,
{
    let candidate = get(
        deps,
        GetFeedbackCandidateInput {
            namespace: input.namespace.clone(),
            candidate_id: input.candidate_id.clone(),
        },
    )
    .await?;
    let request = WriteReceiptRequest::new(
        "commit_feedback_candidate",
        input.namespace.as_str(),
        &input.request_id,
        &input,
    )?;
    let reflection = super::run_reflection::ReflectionInput::new(
        Reflection::new(&candidate.proposal.summary),
        candidate.proposal.target_claim_reference.claim_id(),
        Some(candidate.proposal.replacement_claim.clone()),
        candidate
            .proposal
            .evidence_event_ids
            .iter()
            .map(|id| id.event_id().to_string())
            .collect(),
    )
    .with_strict_evidence_scope()
    .with_write_receipt(request)
    .with_feedback_candidate(input.namespace.clone(), input.candidate_id.clone());
    super::run_reflection::execute(deps, reflection).await?;
    get(
        deps,
        GetFeedbackCandidateInput {
            namespace: input.namespace,
            candidate_id: input.candidate_id,
        },
    )
    .await
}

pub(crate) async fn load_candidate(
    tx: &mut (dyn ReflectionTransaction + Send),
    namespace: &Namespace,
    candidate_id: &str,
) -> Result<FeedbackCandidate, AppError> {
    tx.load_feedback_candidate(namespace, candidate_id)
        .await?
        .ok_or_else(|| {
            AppError::InvalidParams(
                "feedback candidate was not found in the requested scope".into(),
            )
        })
}
pub(crate) async fn validate_in_transaction(
    tx: &mut (dyn ReflectionTransaction + Send),
    proposal: &FeedbackCandidateProposal,
) -> Result<FeedbackValidation, AppError> {
    let target = tx
        .load_claim_for_reflection(proposal.target_claim_reference.claim_id())
        .await?;
    let mut events = Vec::new();
    for reference in &proposal.evidence_event_ids {
        if let Some(event) = tx.load_event_for_reflection(reference.event_id()).await? {
            events.push(event);
        }
    }
    validate_candidate(proposal, target.as_ref(), &events)
}
fn require_text(text: &str, field: &str) -> Result<(), AppError> {
    if text.is_empty()
        || text.trim() != text
        || text.len() > 4096
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(AppError::InvalidParams(format!(
            "{field} must be nonblank, trimmed, and at most 4096 bytes"
        )));
    }
    Ok(())
}
