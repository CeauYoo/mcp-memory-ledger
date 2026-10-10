//! Internal append-only global self-model snapshots. Never serialize these directly
//! into scoped history, export, recall, or context responses.
use crate::domain::{commitment::Commitment, identity_core::IdentityCore};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelfModelVersionKind {
    InitializationBaseline,
    MigrationBaseline,
    Update,
    Rollback,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SelfModelComponent {
    Identity,
    Commitments,
}

#[derive(
    Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct SelfModelRollbackRequest {
    pub target_version: u64,
    pub components: Vec<SelfModelComponent>,
    pub confirm: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SelfModelVersion {
    pub version: u64,
    pub previous_version: Option<u64>,
    pub kind: SelfModelVersionKind,
    pub reflection_id: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub effective_at: Option<DateTime<Utc>>,
    pub identity: IdentityCore,
    pub commitments: Vec<Commitment>,
    pub identity_written: bool,
    pub commitments_written: bool,
    pub identity_source_version: u64,
    pub commitment_source_version: u64,
    pub rollback_target_version: Option<u64>,
}
