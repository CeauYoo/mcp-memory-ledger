//! Inspectable, evidence-bound object corrections. Validation proves the report
//! contract only; producer labels are not authenticated and semantic truth is not inferred.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        claim::{ClaimDraft, ClaimReference},
        event::EventReference,
        feedback::{FeedbackSourceKind, FeedbackVerificationResult},
        types::Namespace,
    },
    error::AppError,
    ports::{ClaimStatus, StoredClaim, StoredEvent},
};

pub const FEEDBACK_CONTRACT_LIMITATION: &str = "Validation checks report structure, exact target/version/scope and object alignment only; it does not authenticate the producer, prove semantic truth, or grant action authority.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackCandidateState {
    Proposed,
    Validated,
    Blocked,
    Rejected,
    Committed,
}
impl FeedbackCandidateState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Validated => "validated",
            Self::Blocked => "blocked",
            Self::Rejected => "rejected",
            Self::Committed => "committed",
        }
    }
    pub fn terminal(self) -> bool {
        matches!(self, Self::Rejected | Self::Committed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackValidationReason {
    pub code: String,
    pub detail: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackValidation {
    pub passed: bool,
    pub reasons: Vec<FeedbackValidationReason>,
    pub limitation: String,
}
impl FeedbackValidation {
    pub fn pending() -> Self {
        Self {
            passed: false,
            reasons: vec![FeedbackValidationReason {
                code: "not_validated".into(),
                detail: "Explicit deterministic validation has not run.".into(),
            }],
            limitation: FEEDBACK_CONTRACT_LIMITATION.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackCandidateProposal {
    pub namespace: Namespace,
    pub target_claim_reference: ClaimReference,
    pub expected_target_version: String,
    pub replacement_claim: ClaimDraft,
    pub evidence_event_ids: Vec<EventReference>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackCandidate {
    pub candidate_id: String,
    pub proposal: FeedbackCandidateProposal,
    pub state: FeedbackCandidateState,
    pub validation: FeedbackValidation,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Optimistic version of candidate lifecycle, distinct from the target fingerprint.
    pub revision: u64,
    pub rejection_reason: Option<String>,
    pub reflection_id: Option<String>,
    pub replacement_claim_id: Option<String>,
}

/// Stable v1 content fingerprint of the original Claim identity, content and
/// status fields. Additive provenance metadata is deliberately excluded: a
/// schema migration must not invalidate already-persisted feedback candidates.
pub fn claim_version(claim: &StoredClaim) -> Result<String, AppError> {
    #[derive(Serialize)]
    struct ClaimVersionV1<'a> {
        claim_id: &'a str,
        claim: &'a ClaimDraft,
        status: ClaimStatus,
    }
    let payload = ClaimVersionV1 {
        claim_id: &claim.claim_id,
        claim: &claim.claim,
        status: claim.status,
    };
    let bytes = serde_json::to_vec(&payload).map_err(|e| AppError::Message(e.to_string()))?;
    Ok(format!("claim-version:v1:{:x}", Sha256::digest(bytes)))
}

pub fn validate_candidate(
    proposal: &FeedbackCandidateProposal,
    target: Option<&StoredClaim>,
    events: &[StoredEvent],
) -> Result<FeedbackValidation, AppError> {
    let mut reasons = Vec::new();
    let mut fail = |code: &str, detail: String| {
        reasons.push(FeedbackValidationReason {
            code: code.into(),
            detail,
        })
    };
    let Some(target) = target else {
        fail(
            "target_missing",
            "Target Claim does not exist in the requested scope.".into(),
        );
        return Ok(FeedbackValidation {
            passed: false,
            reasons,
            limitation: FEEDBACK_CONTRACT_LIMITATION.into(),
        });
    };
    if target.claim.namespace() != &proposal.namespace
        || target.claim.owner() != proposal.namespace.derived_owner()
    {
        fail(
            "target_scope_mismatch",
            "Target Claim must belong to the exact requested owner and namespace.".into(),
        );
    }
    if target.claim_id != proposal.target_claim_reference.claim_id() {
        fail(
            "target_mismatch",
            "Target Claim ID differs from the proposal.".into(),
        );
    }
    if target.status == ClaimStatus::Superseded {
        fail(
            "target_terminal",
            "Target Claim is already superseded.".into(),
        );
    }
    if claim_version(target)? != proposal.expected_target_version {
        fail(
            "target_version_mismatch",
            "Target Claim content or status changed since the observation.".into(),
        );
    }
    let replacement = &proposal.replacement_claim;
    if replacement.owner() != target.claim.owner()
        || replacement.namespace() != target.claim.namespace()
        || replacement.subject() != target.claim.subject()
        || replacement.predicate() != target.claim.predicate()
        || replacement.mode() != target.claim.mode()
    {
        fail(
            "replacement_contract_mismatch",
            "Feedback candidates may change only the target Claim object.".into(),
        );
    }
    if replacement.object() == target.claim.object() || replacement.object().trim().is_empty() {
        fail(
            "replacement_unchanged_or_blank",
            "The proposed object must be nonblank and differ from the target.".into(),
        );
    }
    if proposal.evidence_event_ids.is_empty() {
        fail(
            "insufficient_evidence",
            "At least one externally reported observation is required.".into(),
        );
    }
    for reference in &proposal.evidence_event_ids {
        let Some(event) = events
            .iter()
            .find(|event| event.event_id == reference.event_id())
        else {
            fail(
                "evidence_missing",
                format!("Evidence {} was not found.", reference.canonical()),
            );
            continue;
        };
        if event.event.namespace() != &proposal.namespace
            || event.event.owner() != target.claim.owner()
        {
            fail(
                "evidence_scope_mismatch",
                format!(
                    "Evidence {} is outside the target scope.",
                    reference.canonical()
                ),
            );
            continue;
        }
        let Some(feedback) = event.event.feedback() else {
            fail(
                "feedback_missing",
                format!(
                    "Evidence {} has no structured feedback report.",
                    reference.canonical()
                ),
            );
            continue;
        };
        if feedback.validate().is_err() {
            fail(
                "feedback_invalid",
                format!(
                    "Evidence {} has malformed feedback metadata.",
                    reference.canonical()
                ),
            );
        }
        // This contract cannot determine the applicability of natural-language
        // restrictions. Keep every limited report inspectable, but fail closed
        // rather than guessing that its limitations permit this correction.
        if !feedback.limitations.is_empty() {
            fail(
                "limitations_require_review",
                format!(
                    "Evidence {} declares limitations whose applicability cannot be determined by this validator; limited reports cannot authorize a candidate correction.",
                    reference.canonical()
                ),
            );
        }
        if feedback.source_kind == FeedbackSourceKind::ModelAsserted {
            fail(
                "model_asserted_insufficient",
                format!(
                    "Evidence {} is model-asserted, not an external observation.",
                    reference.canonical()
                ),
            );
        }
        if matches!(
            feedback.verification_result,
            FeedbackVerificationResult::Inconclusive | FeedbackVerificationResult::NotPerformed
        ) {
            fail(
                "verification_insufficient",
                format!(
                    "Evidence {} has no definitive reported verification.",
                    reference.canonical()
                ),
            );
        }
        if feedback.observed_target != proposal.target_claim_reference.canonical() {
            fail(
                "feedback_target_mismatch",
                format!(
                    "Evidence {} does not identify the exact canonical target.",
                    reference.canonical()
                ),
            );
        }
        if feedback.observed_version.as_deref() != Some(proposal.expected_target_version.as_str()) {
            fail(
                "feedback_version_mismatch",
                format!(
                    "Evidence {} does not identify the observed target version.",
                    reference.canonical()
                ),
            );
        }
        if feedback.expected != target.claim.object() || feedback.actual != replacement.object() {
            fail(
                "feedback_object_mismatch",
                format!(
                    "Evidence {} must bind expected to the old object and actual to the proposed object exactly.",
                    reference.canonical()
                ),
            );
        }
        for linked in &feedback.evidence_refs {
            if !proposal.evidence_event_ids.contains(linked) {
                fail(
                    "feedback_reference_unbound",
                    format!(
                        "Feedback evidence {} must be included in the candidate evidence manifest.",
                        linked.canonical()
                    ),
                );
            }
        }
    }
    Ok(FeedbackValidation {
        passed: reasons.is_empty(),
        reasons,
        limitation: FEEDBACK_CONTRACT_LIMITATION.into(),
    })
}
