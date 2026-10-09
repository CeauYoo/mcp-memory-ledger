//! Caller-authored, evidence-linked experience. Activation only makes knowledge available;
//! procedure steps are inert data, never executable authority or identity policy.
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    domain::{event::EventReference, types::Namespace},
    error::AppError,
};

pub const MAX_EXPERIENCE_TEXT_BYTES: usize = 4096;
pub const MAX_EXPERIENCE_REFERENCES: usize = 64;
pub const MAX_EXPERIENCE_LIST_LIMIT: usize = 100;
pub const MAX_EXPERIENCE_PAYLOAD_BYTES: usize = 65536;
pub const MAX_EXPERIENCE_RECALL_BYTES: usize = 65536;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EpisodeContent {
    pub title: String,
    pub objective: String,
    pub actions: Vec<String>,
    pub observations: Vec<String>,
    pub outcome: String,
    pub lesson: String,
    pub limitations: Vec<String>,
    pub source_event_refs: Vec<String>,
}

impl EpisodeContent {
    pub fn validate(&self) -> Result<(), AppError> {
        text(&self.title, "title", 256)?;
        text(&self.objective, "objective", MAX_EXPERIENCE_TEXT_BYTES)?;
        text(&self.outcome, "outcome", MAX_EXPERIENCE_TEXT_BYTES)?;
        text(&self.lesson, "lesson", MAX_EXPERIENCE_TEXT_BYTES)?;
        text_list(&self.actions, "actions", 32, false)?;
        text_list(&self.observations, "observations", 32, true)?;
        text_list(&self.limitations, "limitations", 16, false)?;
        if self.source_event_refs.is_empty()
            || self.source_event_refs.len() > MAX_EXPERIENCE_REFERENCES
        {
            return invalid("source_event_refs must contain 1 to 64 references");
        }
        let mut ids = BTreeSet::new();
        for reference in &self.source_event_refs {
            let event = EventReference::parse(reference)?;
            identifier(event.event_id(), "source_event_ref")?;
            if !ids.insert(event.event_id().to_owned()) {
                return invalid("source_event_refs must be unique after normalization");
            }
        }
        payload_size(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceKind {
    Semantic,
    Procedural,
}
impl ExperienceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceStatus {
    Pending,
    Active,
    Rejected,
    Superseded,
}
impl ExperienceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Rejected => "rejected",
            Self::Superseded => "superseded",
        }
    }
    pub fn validate_transition(self, next: Self) -> Result<(), AppError> {
        let allowed = matches!(
            (self, next),
            (
                Self::Pending,
                Self::Active | Self::Rejected | Self::Superseded
            ) | (Self::Active, Self::Rejected | Self::Superseded)
                | (Self::Rejected, Self::Superseded)
        );
        if allowed {
            Ok(())
        } else {
            invalid(
                "invalid experience status transition; revise or rollback creates a new pending version",
            )
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExperienceContent {
    pub kind: ExperienceKind,
    pub title: String,
    /// Caller-supplied knowledge statement or the objective of an inert procedure.
    pub statement: String,
    pub steps: Vec<String>,
    pub limitations: Vec<String>,
    pub source_episode_ids: Vec<String>,
}
impl ExperienceContent {
    pub fn validate(&self) -> Result<(), AppError> {
        text(&self.title, "title", 256)?;
        text(&self.statement, "statement", MAX_EXPERIENCE_TEXT_BYTES)?;
        text_list(
            &self.steps,
            "steps",
            32,
            self.kind == ExperienceKind::Procedural,
        )?;
        if self.kind == ExperienceKind::Semantic && !self.steps.is_empty() {
            return invalid("semantic candidates cannot contain procedure steps");
        }
        text_list(&self.limitations, "limitations", 16, false)?;
        references(&self.source_episode_ids, "source_episode_ids")?;
        payload_size(self)
    }
    pub fn search_text(&self) -> String {
        std::iter::once(self.title.as_str())
            .chain(std::iter::once(self.statement.as_str()))
            .chain(self.steps.iter().map(String::as_str))
            .chain(self.limitations.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedEpisode {
    pub episode_id: String,
    pub namespace: Namespace,
    pub recorded_at: DateTime<Utc>,
    pub content: EpisodeContent,
}

/// Historical snapshots preserve the status at that time. Only the current version
/// can supply active knowledge; a historical `active` snapshot grants nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceCandidate {
    pub candidate_id: String,
    pub namespace: Namespace,
    pub version: i64,
    pub status: ExperienceStatus,
    pub recorded_at: DateTime<Utc>,
    pub content: ExperienceContent,
    pub change_kind: ExperienceChangeKind,
    pub previous_version: Option<i64>,
    pub rollback_target_version: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceChangeKind {
    Created,
    StatusUpdated,
    Revised,
    RolledBack,
}
impl ExperienceChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::StatusUpdated => "status_updated",
            Self::Revised => "revised",
            Self::RolledBack => "rolled_back",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceWriteResult<T> {
    pub record: T,
    pub replayed: bool,
    /// A local audit/receipt identifier, never the caller's raw retry key.
    pub operation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEpisodeRequest {
    pub namespace: String,
    pub request_id: String,
    pub episode_id: String,
    pub content: EpisodeContent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCandidateRequest {
    pub namespace: String,
    pub request_id: String,
    pub candidate_id: String,
    pub content: ExperienceContent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCandidateStatusRequest {
    pub namespace: String,
    pub request_id: String,
    pub candidate_id: String,
    pub expected_version: i64,
    pub status: ExperienceStatus,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviseCandidateRequest {
    pub namespace: String,
    pub request_id: String,
    pub candidate_id: String,
    pub expected_version: i64,
    pub content: ExperienceContent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RollbackCandidateRequest {
    pub namespace: String,
    pub request_id: String,
    pub candidate_id: String,
    pub expected_version: i64,
    pub target_version: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetEpisodeRequest {
    pub namespace: String,
    pub episode_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCandidateRequest {
    pub namespace: String,
    pub candidate_id: String,
    /// Omit to inspect the current snapshot. A historical version is inspection only.
    #[serde(default)]
    pub version: Option<i64>,
}
fn default_limit() -> usize {
    20
}
fn default_budget() -> usize {
    16384
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEpisodesRequest {
    pub namespace: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub after_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCandidatesRequest {
    pub namespace: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub after_id: Option<String>,
    #[serde(default)]
    pub status: Option<ExperienceStatus>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecallCandidatesRequest {
    pub namespace: String,
    pub query: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default = "default_budget")]
    pub max_bytes: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceRecall {
    pub candidates: Vec<ExperienceCandidate>,
    /// Exact UTF-8 byte size of this complete compact JSON result.
    pub used_bytes: usize,
    pub truncated: bool,
    pub selection_reason: String,
}

pub fn validate_namespace(value: &str) -> Result<Namespace, AppError> {
    identifier(value, "namespace")?;
    Namespace::parse(value).map_err(Into::into)
}
pub fn validate_version(version: i64) -> Result<(), AppError> {
    if version <= 0 || version == i64::MAX {
        invalid("version must be between 1 and i64::MAX - 1")
    } else {
        Ok(())
    }
}
pub fn validate_limit(limit: usize) -> Result<(), AppError> {
    if limit == 0 || limit > MAX_EXPERIENCE_LIST_LIMIT {
        invalid("limit must be between 1 and 100")
    } else {
        Ok(())
    }
}
pub fn identifier(value: &str, name: &str) -> Result<(), AppError> {
    if value.is_empty()
        || value.len() > 256
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        invalid(&format!(
            "{name} must contain 1 to 256 bytes without whitespace or control characters"
        ))
    } else {
        Ok(())
    }
}
pub fn recall_terms(request: &RecallCandidatesRequest) -> Result<Vec<String>, AppError> {
    validate_namespace(&request.namespace)?;
    validate_limit(request.limit)?;
    text(&request.query, "query", 1024)?;
    if !(512..=MAX_EXPERIENCE_RECALL_BYTES).contains(&request.max_bytes) {
        return invalid("max_bytes must be between 512 and 65536");
    }
    let terms: Vec<_> = request
        .query
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if terms.is_empty() || terms.len() > 16 {
        return invalid("query must contain between 1 and 16 literal terms");
    }
    Ok(terms)
}
fn references(values: &[String], name: &str) -> Result<(), AppError> {
    if values.is_empty() || values.len() > MAX_EXPERIENCE_REFERENCES {
        return invalid(&format!("{name} must contain 1 to 64 identifiers"));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        identifier(value, name)?;
        if !seen.insert(value) {
            return invalid(&format!("{name} must contain unique identifiers"));
        }
    }
    Ok(())
}
fn text(value: &str, name: &str, max: usize) -> Result<(), AppError> {
    if value.trim().is_empty() || value.len() > max || value.contains('\0') {
        invalid(&format!(
            "{name} must contain nonblank text of at most {max} bytes without NUL"
        ))
    } else {
        Ok(())
    }
}
fn text_list(values: &[String], name: &str, max: usize, required: bool) -> Result<(), AppError> {
    if values.len() > max || (required && values.is_empty()) {
        return invalid(&format!(
            "{name} must contain {} to {max} items",
            usize::from(required)
        ));
    }
    for value in values {
        text(value, name, MAX_EXPERIENCE_TEXT_BYTES)?;
    }
    Ok(())
}
fn payload_size<T: Serialize>(value: &T) -> Result<(), AppError> {
    let size = serde_json::to_vec(value)
        .map_err(|e| AppError::Message(e.to_string()))?
        .len();
    if size > MAX_EXPERIENCE_PAYLOAD_BYTES {
        invalid("experience content exceeds 65536 serialized bytes")
    } else {
        Ok(())
    }
}
fn invalid<T>(message: &str) -> Result<T, AppError> {
    Err(AppError::InvalidParams(message.to_owned()))
}
