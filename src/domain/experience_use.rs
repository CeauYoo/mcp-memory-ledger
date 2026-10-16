//! Immutable caller-reported use observations. Applicability is not certification.
use crate::{
    domain::{
        event::EventReference,
        experience::{identifier, validate_limit, validate_version},
        feedback::FeedbackVerificationResult,
        task::{invalid, scope, text},
    },
    error::AppError,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub const MAX_EXPERIENCE_USE_BYTES: usize = 32768;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceUseDecision {
    Proposed,
    Adopted,
    Rejected,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApplicabilityAssessment {
    CallerReportedSatisfied,
    CallerReportedNotSatisfied,
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicabilityCondition {
    pub condition: String,
    pub assessment: ApplicabilityAssessment,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordExperienceUseRequest {
    pub namespace: String,
    pub request_id: String,
    pub use_id: String,
    pub task_id: String,
    pub task_version: i64,
    pub expected_task_version: i64,
    pub candidate_id: String,
    pub candidate_version: i64,
    pub expected_candidate_version: i64,
    pub decision: ExperienceUseDecision,
    pub reason: String,
    pub conditions: Vec<ApplicabilityCondition>,
    pub action_event_refs: Vec<String>,
    pub outcome_event_refs: Vec<String>,
    pub verification: FeedbackVerificationResult,
    pub evaluation_method: String,
    pub limitations: Vec<String>,
    #[serde(default)]
    pub related_use_id: Option<String>,
}
/// Persisted report omits the caller retry key and transient CAS expectations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceUseObservation {
    pub namespace: String,
    pub use_id: String,
    pub task_id: String,
    pub task_version: i64,
    pub candidate_id: String,
    pub candidate_version: i64,
    pub decision: ExperienceUseDecision,
    pub reason: String,
    pub conditions: Vec<ApplicabilityCondition>,
    pub action_event_refs: Vec<String>,
    pub outcome_event_refs: Vec<String>,
    pub verification: FeedbackVerificationResult,
    pub evaluation_method: String,
    pub limitations: Vec<String>,
    pub related_use_id: Option<String>,
}
impl From<RecordExperienceUseRequest> for ExperienceUseObservation {
    fn from(r: RecordExperienceUseRequest) -> Self {
        Self {
            namespace: r.namespace,
            use_id: r.use_id,
            task_id: r.task_id,
            task_version: r.task_version,
            candidate_id: r.candidate_id,
            candidate_version: r.candidate_version,
            decision: r.decision,
            reason: r.reason,
            conditions: r.conditions,
            action_event_refs: r.action_event_refs,
            outcome_event_refs: r.outcome_event_refs,
            verification: r.verification,
            evaluation_method: r.evaluation_method,
            limitations: r.limitations,
            related_use_id: r.related_use_id,
        }
    }
}
impl ExperienceUseObservation {
    pub fn validate(&self) -> Result<(), AppError> {
        RecordExperienceUseRequest {
            namespace: self.namespace.clone(),
            request_id: "x".into(),
            use_id: self.use_id.clone(),
            task_id: self.task_id.clone(),
            task_version: self.task_version,
            expected_task_version: self.task_version,
            candidate_id: self.candidate_id.clone(),
            candidate_version: self.candidate_version,
            expected_candidate_version: self.candidate_version,
            decision: self.decision,
            reason: self.reason.clone(),
            conditions: self.conditions.clone(),
            action_event_refs: self.action_event_refs.clone(),
            outcome_event_refs: self.outcome_event_refs.clone(),
            verification: self.verification,
            evaluation_method: self.evaluation_method.clone(),
            limitations: self.limitations.clone(),
            related_use_id: self.related_use_id.clone(),
        }
        .validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceUse {
    pub observation: ExperienceUseObservation,
    pub recorded_at: DateTime<Utc>,
    pub verification_basis: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceUseView {
    pub record: ExperienceUse,
    pub currently_reusable: bool,
    pub invalidation_reasons: Vec<String>,
}
fn default_limit() -> usize {
    20
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListExperienceUsesRequest {
    pub namespace: String,
    pub task_id: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub after_id: Option<String>,
}
impl RecordExperienceUseRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        scope(&self.namespace)?;
        for (s, n) in [
            (&self.use_id, "use_id"),
            (&self.task_id, "task_id"),
            (&self.candidate_id, "candidate_id"),
            (&self.request_id, "request_id"),
        ] {
            identifier(s, n)?;
        }
        for v in [
            self.task_version,
            self.expected_task_version,
            self.candidate_version,
            self.expected_candidate_version,
        ] {
            validate_version(v)?;
        }
        if let Some(id) = &self.related_use_id {
            identifier(id, "related_use_id")?;
            if id == &self.use_id {
                return invalid("a use cannot relate to itself");
            }
        }
        text(&self.reason, "reason", 4096)?;
        text(&self.evaluation_method, "evaluation_method", 4096)?;
        if self.conditions.is_empty() || self.conditions.len() > 16 {
            return invalid("conditions must contain 1..16 explicit applicability reports");
        }
        for c in &self.conditions {
            text(&c.condition, "condition", 1024)?;
        }
        if self.decision == ExperienceUseDecision::Adopted
            && self.related_use_id.is_none()
            && self
                .conditions
                .iter()
                .any(|c| c.assessment != ApplicabilityAssessment::CallerReportedSatisfied)
        {
            return invalid(
                "adoption requires explicitly caller-reported satisfied conditions; unknown conditions need review",
            );
        }
        if self.limitations.is_empty() || self.limitations.len() > 16 {
            return invalid("limitations must contain 1..16 reports");
        }
        for l in &self.limitations {
            text(l, "limitation", 2048)?;
        }
        for refs in [&self.action_event_refs, &self.outcome_event_refs] {
            if refs.len() > 32 {
                return invalid("at most 32 references per event role");
            }
            let mut seen = std::collections::BTreeSet::new();
            for r in refs {
                let e = EventReference::parse(r)?;
                identifier(e.event_id(), "event reference")?;
                if !seen.insert(e.event_id().to_owned()) {
                    return invalid("event references must be unique after normalization");
                }
            }
        }
        if self.verification != FeedbackVerificationResult::NotPerformed
            && self.outcome_event_refs.is_empty()
        {
            return invalid("reported verification requires outcome event references");
        }
        if serde_json::to_vec(self)
            .map_err(|e| AppError::Message(e.to_string()))?
            .len()
            > MAX_EXPERIENCE_USE_BYTES
        {
            return invalid("experience use exceeds 32768 bytes");
        }
        Ok(())
    }
}
impl ListExperienceUsesRequest {
    pub fn validate(&self) -> Result<(), AppError> {
        scope(&self.namespace)?;
        identifier(&self.task_id, "task_id")?;
        validate_limit(self.limit)?;
        if let Some(id) = &self.after_id {
            identifier(id, "after_id")?;
        }
        Ok(())
    }
}

/// An explicitly requested, restartable composition of existing episode/candidate writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeriveExperienceRequest {
    pub namespace: String,
    pub request_id: String,
    pub use_id: String,
    pub episode_id: String,
    pub candidate_id: String,
    pub episode_content: crate::domain::experience::EpisodeContent,
    pub candidate_content: crate::domain::experience::ExperienceContent,
}
impl DeriveExperienceRequest {
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
        self.episode_content.validate()?;
        self.candidate_content.validate()?;
        if self.candidate_content.source_episode_ids != vec![self.episode_id.clone()] {
            return invalid("derived candidate must reference exactly the newly derived episode");
        }
        if serde_json::to_vec(self)
            .map_err(|e| AppError::Message(e.to_string()))?
            .len()
            > 65536
        {
            return invalid("derivation exceeds 65536 bytes");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeriveExperienceResult {
    pub reservation_id: String,
    pub use_id: String,
    pub episode_id: String,
    pub candidate_id: String,
    pub episode: Option<
        crate::domain::experience::ExperienceWriteResult<
            crate::domain::experience::PersistedEpisode,
        >,
    >,
    pub candidate: Option<
        crate::domain::experience::ExperienceWriteResult<
            crate::domain::experience::ExperienceCandidate,
        >,
    >,
    pub stage: String,
    pub retryable_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationReservation {
    pub use_id: String,
    pub episode_id: String,
    pub candidate_id: String,
}
