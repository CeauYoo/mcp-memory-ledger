//! Evidence-bounded model suggestions. No activation or execution authority.
use crate::{
    domain::{
        event::EventReference,
        experience::{EpisodeContent, ExperienceContent, identifier},
        experience_use::{DeriveExperienceResult, ExperienceUse},
        task::{invalid, scope},
    },
    error::AppError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const MAX_DRAFT_INPUT_BYTES: usize = 131072;
pub const MAX_DRAFT_PROPOSAL_BYTES: usize = 65536;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateExperienceDraftRequest {
    pub namespace: String,
    pub request_id: String,
    pub use_id: String,
    pub episode_id: String,
    pub candidate_id: String,
    #[serde(default)]
    pub allow_model_generation: bool,
}
impl GenerateExperienceDraftRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        scope(&self.namespace)?;
        for (s, n) in [
            (&self.request_id, "request_id"),
            (&self.use_id, "use_id"),
            (&self.episode_id, "episode_id"),
            (&self.candidate_id, "candidate_id"),
        ] {
            identifier(s, n)?;
        }
        if !self.allow_model_generation {
            return invalid("explicit allow_model_generation=true is required");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftEvidence {
    pub event_id: String,
    pub payload: serde_json::Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperienceDraftInput {
    pub request: GenerateExperienceDraftRequest,
    pub source_use: ExperienceUse,
    pub evidence: Vec<DraftEvidence>,
}
impl ExperienceDraftInput {
    pub fn validate(&self) -> Result<(), AppError> {
        self.request.validate()?;
        self.source_use.observation.validate()?;
        let o = &self.source_use.observation;
        if o.namespace != self.request.namespace || o.use_id != self.request.use_id {
            return invalid("draft source scope mismatch");
        }
        let allowed: BTreeSet<String> = o
            .action_event_refs
            .iter()
            .chain(&o.outcome_event_refs)
            .map(|r| EventReference::parse(r).map(|e| e.event_id().to_owned()))
            .collect::<Result<_, _>>()?;
        let actual: BTreeSet<String> = self.evidence.iter().map(|e| e.event_id.clone()).collect();
        if allowed.is_empty() || allowed != actual || actual.len() != self.evidence.len() {
            return invalid("draft requires exactly the selected use evidence");
        }
        if serde_json::to_vec(self)
            .map_err(|_| AppError::Message("draft serialization failed".into()))?
            .len()
            > MAX_DRAFT_INPUT_BYTES
        {
            return invalid("draft input exceeds 131072 bytes");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExperienceDraftProposal {
    pub episode_content: EpisodeContent,
    pub candidate_content: ExperienceContent,
}
impl ExperienceDraftProposal {
    pub fn validate(&self, input: &ExperienceDraftInput) -> Result<(), AppError> {
        input.validate()?;
        self.episode_content.validate()?;
        self.candidate_content.validate()?;
        if self.candidate_content.source_episode_ids != vec![input.request.episode_id.clone()] {
            return invalid("draft candidate must reference only the requested episode");
        }
        let allowed: BTreeSet<_> = input.evidence.iter().map(|e| e.event_id.as_str()).collect();
        for r in &self.episode_content.source_event_refs {
            if !allowed.contains(EventReference::parse(r)?.event_id()) {
                return invalid("draft cited evidence outside selected use");
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| AppError::Message("draft serialization failed".into()))?
            .len()
            > MAX_DRAFT_PROPOSAL_BYTES
        {
            return invalid("draft proposal exceeds 65536 bytes");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftCompletion {
    SavedProposal {
        proposal: Box<ExperienceDraftProposal>,
    },
    /// Local invocation returned an error; remote generation/billing may still have occurred.
    /// This is terminal for this key and never means a retry is safe.
    KnownFailed,
    Rejected,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceDraftStatus {
    IncompleteOrInFlight,
    /// Local invocation returned an error; remote generation/billing may still have occurred.
    /// This is terminal for this key and never means a retry is safe.
    KnownFailed,
    Rejected,
    SavedProposal,
    PendingCandidateCreated,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerateExperienceDraftResult {
    pub reservation_id: String,
    pub replayed: bool,
    pub status: ExperienceDraftStatus,
    pub derivation: Option<DeriveExperienceResult>,
}
