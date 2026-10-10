//! Scoped, disclosure-safe self-model reads. Aggregate snapshots never cross this port.
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    domain::{
        commitment::Commitment, reflection::ReflectionIdentityUpdate,
        self_model_version::SelfModelVersionKind, types::MemoryScope,
    },
    error::AppError,
};

pub const MAX_SELF_MODEL_VERSION_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfModelVersionQuery {
    pub scope: MemoryScope,
    pub limit: usize,
    pub before_version: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfModelVersionPage {
    /// Deliberately global metadata; callers must explicitly opt into disclosure.
    pub current_version: u64,
    pub has_more: bool,
    pub records: Vec<ScopedSelfModelVersionRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopedSelfModelVersionRecord {
    pub version: u64,
    pub previous_version: Option<u64>,
    pub kind: SelfModelVersionKind,
    pub recorded_at: DateTime<Utc>,
    pub effective_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_target_version: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity_update: Option<ScopedIdentityVersionPatch>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commitment_updates: Option<ScopedCommitmentVersionPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopedIdentityVersionPatch {
    pub source_version: u64,
    pub source_reflection_id: String,
    pub patch: ReflectionIdentityUpdate,
    pub changed: bool,
    pub previous_count: usize,
    pub current_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_patch: Option<ReflectionIdentityUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_values_redacted: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScopedCommitmentVersionPatch {
    pub source_version: u64,
    pub source_reflection_id: String,
    pub patch: Vec<Commitment>,
    pub changed: bool,
    pub previous_count: usize,
    pub current_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_patch: Option<Vec<Commitment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_values_redacted: Option<String>,
}

#[async_trait]
pub trait SelfModelVersionStore {
    /// Verify head/projections and derive the page in one consistent read transaction.
    /// Include only explicitly written components with verified, single-scope durable
    /// source evidence. Never include inherited aggregate state; disclose previous
    /// values only after independently verifying their same-scope component source.
    async fn query_self_model_versions(
        &self,
        query: SelfModelVersionQuery,
    ) -> Result<SelfModelVersionPage, AppError>;
}
