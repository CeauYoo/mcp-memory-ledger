//! Workspace task records are caller-reported knowledge, never execution authority.
//! Source labels and verification results authenticate neither producer nor outcome.
use crate::domain::{
    event::EventReference,
    experience::{identifier, validate_version},
    feedback::{FeedbackSourceKind, FeedbackVerificationResult},
    types::{Mode, Namespace},
};
use crate::error::AppError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_TASK_BYTES: usize = 131072;
pub const MAX_TASK_ITEMS: usize = 128;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Active,
    Paused,
    Completed,
    Cancelled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    Goal,
    Requirement,
    Prohibition,
    CompletionCriterion,
    OpenQuestion,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequirementState {
    Reported,
    Candidate,
    Withdrawn,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskSource {
    pub kind: FeedbackSourceKind,
    #[serde(default)]
    pub reference: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskRequirement {
    pub requirement_id: String,
    pub kind: RequirementKind,
    pub text: String,
    pub state: RequirementState,
    pub source: TaskSource,
    #[serde(default)]
    pub replaces: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRevision {
    pub namespace: String,
    pub task_id: String,
    pub version: i64,
    pub title: String,
    pub status: TaskStatus,
    pub requirements: Vec<TaskRequirement>,
    pub source: TaskSource,
    pub reason: String,
    pub recorded_at: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTaskRequest {
    pub namespace: String,
    pub task_id: String,
    pub request_id: String,
    pub title: String,
    pub requirements: Vec<TaskRequirement>,
    pub source: TaskSource,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequirementChange {
    Add {
        requirement: TaskRequirement,
    },
    Replace {
        requirement_id: String,
        requirement: TaskRequirement,
    },
    Withdraw {
        requirement_id: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviseTaskRequest {
    pub namespace: String,
    pub task_id: String,
    pub request_id: String,
    pub expected_version: i64,
    pub changes: Vec<RequirementChange>,
    #[serde(default)]
    pub status: Option<TaskStatus>,
    pub source: TaskSource,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetTaskRequest {
    pub namespace: String,
    pub task_id: String,
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
pub struct ListTasksRequest {
    pub namespace: String,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub after_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckpointStep {
    pub step_id: String,
    pub text: String,
    #[schemars(with = "String")]
    pub mode: Mode,
    pub verification: FeedbackVerificationResult,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub uri: String,
    #[serde(default)]
    pub digest: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecordTaskCheckpointRequest {
    pub namespace: String,
    pub task_id: String,
    pub request_id: String,
    pub checkpoint_id: String,
    pub expected_version: i64,
    pub expected_checkpoint_sequence: i64,
    pub steps: Vec<CheckpointStep>,
    pub blockers: Vec<String>,
    pub next_steps: Vec<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub repository_revision: Option<String>,
    pub artifacts: Vec<ArtifactReference>,
    pub source: TaskSource,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCheckpoint {
    pub namespace: String,
    pub task_id: String,
    pub checkpoint_id: String,
    pub task_version: i64,
    pub sequence: i64,
    pub recorded_at: String,
    pub steps: Vec<CheckpointStep>,
    pub blockers: Vec<String>,
    pub next_steps: Vec<String>,
    pub environment: Option<String>,
    pub repository_revision: Option<String>,
    pub artifacts: Vec<ArtifactReference>,
    pub source: TaskSource,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskSnapshot {
    pub task: TaskRevision,
    pub checkpoint: Option<TaskCheckpoint>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskContextRequest {
    pub namespace: String,
    pub task_id: String,
    #[serde(default = "default_budget")]
    pub max_bytes: usize,
}
#[derive(Debug, Clone, Serialize)]
pub struct TaskResumeContext {
    pub namespace: String,
    pub task_id: String,
    pub title: String,
    pub task_version: i64,
    pub status: TaskStatus,
    pub context_id: String,
    pub complete: bool,
    pub can_resume: bool,
    pub mandatory_requirements: Vec<TaskRequirement>,
    pub mandatory_requirement_count: usize,
    pub checkpoint: Option<TaskCheckpoint>,
    pub diagnostics: Vec<String>,
    pub snapshot_consistent: bool,
    pub mandatory_snapshot_consistent: bool,
    pub task_version_after_selection: Option<i64>,
    pub related_memory: Option<serde_json::Value>,
    pub experiences: Vec<crate::domain::experience::ExperienceCandidate>,
    pub serialized_bytes: usize,
    pub max_bytes: usize,
    pub authority: &'static str,
    pub verification_basis: &'static str,
    pub artifact_verification: FeedbackVerificationResult,
}
pub type TaskWriteResult<T> = crate::domain::experience::ExperienceWriteResult<T>;
pub fn invalid<T>(message: &str) -> Result<T, AppError> {
    Err(AppError::InvalidParams(message.into()))
}
pub fn scope(namespace: &str) -> Result<Namespace, AppError> {
    identifier(namespace, "namespace")?;
    let ns = Namespace::parse(namespace)?;
    if !namespace.starts_with("project/") {
        return invalid("tasks require an exact project namespace");
    }
    Ok(ns)
}
pub fn text(value: &str, name: &str, max: usize) -> Result<(), AppError> {
    if value.trim().is_empty()
        || value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return invalid(&format!("invalid bounded {name}"));
    }
    Ok(())
}
pub fn bounded<T: Serialize>(value: &T) -> Result<(), AppError> {
    if serde_json::to_vec(value)
        .map_err(|e| AppError::Message(e.to_string()))?
        .len()
        > MAX_TASK_BYTES
    {
        return invalid("task payload exceeds 131072 bytes");
    }
    Ok(())
}
pub fn validate_source(source: &TaskSource) -> Result<(), AppError> {
    if let Some(r) = &source.reference {
        identifier(r, "source reference")?;
        EventReference::parse(r)?;
    }
    Ok(())
}
fn validate_requirement(r: &TaskRequirement, initial: bool) -> Result<(), AppError> {
    identifier(&r.requirement_id, "requirement_id")?;
    text(&r.text, "requirement", 4096)?;
    validate_source(&r.source)?;
    if initial && r.state == RequirementState::Withdrawn {
        return invalid("new requirements cannot start withdrawn");
    }
    if r.source.kind != FeedbackSourceKind::CallerReported && r.state == RequirementState::Reported
    {
        return invalid(
            "model/tool extracted requirements must remain candidates; labels do not authenticate callers",
        );
    }
    if let Some(id) = &r.replaces {
        identifier(id, "replaces")?;
    }
    Ok(())
}
pub fn validate_create(r: &CreateTaskRequest) -> Result<(), AppError> {
    scope(&r.namespace)?;
    identifier(&r.task_id, "task_id")?;
    text(&r.title, "title", 256)?;
    text(&r.reason, "reason", 4096)?;
    validate_source(&r.source)?;
    if r.requirements.is_empty() || r.requirements.len() > MAX_TASK_ITEMS {
        return invalid("create requires 1..128 requirements");
    }
    let mut ids = BTreeSet::new();
    for q in &r.requirements {
        validate_requirement(q, true)?;
        if !ids.insert(&q.requirement_id) || q.replaces.is_some() {
            return invalid(
                "initial requirement IDs must be unique without replacement references",
            );
        }
        if r.source.kind != FeedbackSourceKind::CallerReported
            && q.state != RequirementState::Candidate
        {
            return invalid("extracted task requirements must remain candidates");
        }
    }
    bounded(r)
}
/// Pure merge: additions never erase old entries; replacement is explicit and preserves the old ID as withdrawn.
pub fn apply_revision(
    current: &TaskRevision,
    r: &ReviseTaskRequest,
    recorded_at: String,
) -> Result<TaskRevision, AppError> {
    scope(&r.namespace)?;
    identifier(&r.task_id, "task_id")?;
    validate_version(r.expected_version)?;
    validate_source(&r.source)?;
    text(&r.reason, "reason", 4096)?;
    bounded(r)?;
    if current.namespace != r.namespace
        || current.task_id != r.task_id
        || current.version != r.expected_version
    {
        return invalid("task version conflict or scope mismatch");
    }
    if r.changes.len() > MAX_TASK_ITEMS || (r.changes.is_empty() && r.status.is_none()) {
        return invalid("revision needs bounded changes or explicit status");
    }
    if matches!(
        current.status,
        TaskStatus::Completed | TaskStatus::Cancelled
    ) {
        return invalid("completed/cancelled tasks are terminal; create a new task to restart");
    }
    if current.version >= i64::MAX - 1 {
        return invalid("task revision version exhausted");
    }
    let reported = r.source.kind == FeedbackSourceKind::CallerReported;
    if r.status.is_some() && !reported {
        return invalid("candidate extraction cannot change task lifecycle");
    }
    let mut next = current.clone();
    next.version += 1;
    next.recorded_at = recorded_at;
    next.reason = r.reason.clone();
    next.source = r.source.clone();
    for change in &r.changes {
        match change {
            RequirementChange::Add { requirement } => {
                validate_requirement(requirement, true)?;
                if requirement.replaces.is_some() {
                    return invalid("use replace for replacement relationships");
                }
                if !reported && requirement.state != RequirementState::Candidate {
                    return invalid("extraction can add candidates only");
                }
                if next
                    .requirements
                    .iter()
                    .any(|q| q.requirement_id == requirement.requirement_id)
                {
                    return invalid("requirement ID already exists");
                }
                next.requirements.push(requirement.clone());
            }
            RequirementChange::Replace {
                requirement_id,
                requirement,
            } => {
                validate_requirement(requirement, true)?;
                if !reported {
                    return invalid("extraction cannot replace existing requirements");
                }
                if requirement.replaces.as_deref() != Some(requirement_id.as_str())
                    || next
                        .requirements
                        .iter()
                        .any(|q| q.requirement_id == requirement.requirement_id)
                {
                    return invalid(
                        "replacement needs new stable ID and explicit replaces reference",
                    );
                }
                let old = next
                    .requirements
                    .iter_mut()
                    .find(|q| &q.requirement_id == requirement_id)
                    .ok_or_else(|| {
                        AppError::InvalidParams("replacement target not found".into())
                    })?;
                if old.state == RequirementState::Withdrawn
                    || (old.state == RequirementState::Reported
                        && requirement.state != RequirementState::Reported)
                {
                    return invalid(
                        "candidate cannot replace reported requirement or withdrawn target",
                    );
                }
                old.state = RequirementState::Withdrawn;
                next.requirements.push(requirement.clone());
            }
            RequirementChange::Withdraw { requirement_id } => {
                if !reported {
                    return invalid("extraction cannot withdraw requirements");
                }
                let old = next
                    .requirements
                    .iter_mut()
                    .find(|q| &q.requirement_id == requirement_id)
                    .ok_or_else(|| AppError::InvalidParams("withdraw target not found".into()))?;
                if old.state == RequirementState::Withdrawn {
                    return invalid("requirement already withdrawn");
                }
                old.state = RequirementState::Withdrawn;
            }
        }
    }
    if let Some(status) = r.status {
        next.status = status;
    }
    if next.requirements.len() > MAX_TASK_ITEMS {
        return invalid("task requirement history exceeds 128 entries; narrow task scope");
    }
    bounded(&next)?;
    Ok(next)
}
pub fn validate_checkpoint(r: &RecordTaskCheckpointRequest) -> Result<(), AppError> {
    scope(&r.namespace)?;
    identifier(&r.task_id, "task_id")?;
    identifier(&r.checkpoint_id, "checkpoint_id")?;
    validate_version(r.expected_version)?;
    validate_source(&r.source)?;
    if r.expected_checkpoint_sequence < 0 || r.expected_checkpoint_sequence >= i64::MAX - 1 {
        return invalid("invalid checkpoint sequence");
    }
    if r.steps.len() > MAX_TASK_ITEMS
        || r.blockers.len() > 32
        || r.next_steps.len() > 32
        || r.artifacts.len() > 32
    {
        return invalid("checkpoint item limit exceeded");
    }
    let mut ids = BTreeSet::new();
    for s in &r.steps {
        identifier(&s.step_id, "step_id")?;
        text(&s.text, "step", 4096)?;
        if !ids.insert(&s.step_id) || s.evidence_refs.len() > 32 {
            return invalid("duplicate step or too many evidence references");
        }
        for e in &s.evidence_refs {
            identifier(e, "evidence_ref")?;
            EventReference::parse(e)?;
        }
        if s.verification != FeedbackVerificationResult::NotPerformed
            && (s.mode != Mode::Observed || s.evidence_refs.is_empty())
        {
            return invalid(
                "reported verification requires Observed mode and evidence; plans/attempts are not verified results",
            );
        }
    }
    for s in r.blockers.iter().chain(&r.next_steps) {
        text(s, "checkpoint item", 4096)?;
    }
    for s in r.environment.iter().chain(r.repository_revision.iter()) {
        text(s, "environment/version", 4096)?;
    }
    for a in &r.artifacts {
        text(&a.uri, "artifact uri", 4096)?;
        if let Some(d) = &a.digest {
            text(d, "digest", 256)?;
        }
    }
    bounded(r)
}

/// Fail closed on structurally corrupted persisted projections as well as new writes.
pub fn validate_persisted_revision(r: &TaskRevision) -> Result<(), AppError> {
    scope(&r.namespace)?;
    identifier(&r.task_id, "task_id")?;
    validate_version(r.version)?;
    text(&r.title, "title", 256)?;
    text(&r.reason, "reason", 4096)?;
    validate_source(&r.source)?;
    if r.requirements.is_empty() || r.requirements.len() > MAX_TASK_ITEMS {
        return invalid("persisted requirement count invalid");
    }
    let mut ids = BTreeSet::new();
    for q in &r.requirements {
        validate_requirement(q, false)?;
        if !ids.insert(&q.requirement_id) {
            return invalid("duplicate persisted requirement ID");
        }
    }
    for q in &r.requirements {
        if let Some(id) = &q.replaces
            && (id == &q.requirement_id || !ids.contains(id))
        {
            return invalid("persisted replacement target invalid");
        }
    }
    chrono::DateTime::parse_from_rfc3339(&r.recorded_at)
        .map_err(|_| AppError::Message("invalid task recording time".into()))?;
    bounded(r)
}
pub fn validate_persisted_checkpoint(cp: &TaskCheckpoint) -> Result<(), AppError> {
    if cp.sequence < 1 {
        return invalid("invalid persisted checkpoint sequence");
    }
    chrono::DateTime::parse_from_rfc3339(&cp.recorded_at)
        .map_err(|_| AppError::Message("invalid checkpoint recording time".into()))?;
    validate_checkpoint(&RecordTaskCheckpointRequest {
        namespace: cp.namespace.clone(),
        task_id: cp.task_id.clone(),
        request_id: String::new(),
        checkpoint_id: cp.checkpoint_id.clone(),
        expected_version: cp.task_version,
        expected_checkpoint_sequence: cp.sequence - 1,
        steps: cp.steps.clone(),
        blockers: cp.blockers.clone(),
        next_steps: cp.next_steps.clone(),
        environment: cp.environment.clone(),
        repository_revision: cp.repository_revision.clone(),
        artifacts: cp.artifacts.clone(),
        source: cp.source.clone(),
    })?;
    bounded(cp)
}
