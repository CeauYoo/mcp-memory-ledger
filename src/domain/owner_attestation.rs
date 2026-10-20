//! Explicit local owner-channel attestations, not authenticated human authorship.
//! Never accepted as an MCP task source or as execution permission.
use crate::domain::task::RequirementKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAttestationRequest {
    pub namespace: String,
    pub task_id: String,
    pub import_event_id: String,
    pub expected_version: i64,
    /// Exact evidence supplied by the local operator, never model-normalized.
    pub evidence_text: String,
    pub change: OwnerRequirementChange,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum OwnerRequirementChange {
    Append {
        requirement_id: String,
        kind: RequirementKind,
        exact_text: String,
    },
    Withdraw {
        requirement_id: String,
        exact_text: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerRequirementProvenance {
    pub requirement_id: String,
    pub kind: RequirementKind,
    pub import_event_id: String,
    pub exact_text: String,
    pub evidence_text: String,
    pub imported_at_version: i64,
    pub withdrawn_at_version: Option<i64>,
    pub withdrawal_event_id: Option<String>,
    pub attestation_basis: String,
}
