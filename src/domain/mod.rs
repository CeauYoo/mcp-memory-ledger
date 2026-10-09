#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DomainError {
    InsufficientEvidence,
    InvalidClaimReference,
    InvalidEventReference,
    InvalidNamespace,
    NamespaceOwnerMismatch,
    UnknownOwnerNotWritable,
    InvalidSnapshotTimeWindow,
    InvalidFeedbackMetadata,
    InvalidObservedAt,
}

pub mod caller_budget;
pub mod claim;
pub mod commitment;
pub mod episode;
pub mod episode_projection;
pub mod event;
pub mod evidence_link;
pub mod evidence_relation;
pub mod identity_core;
pub mod memory_layer_projection;
pub mod memory_semantics_projection;
pub mod operation_log;
pub mod reflection;
pub mod rules;
pub mod self_revision;
pub mod snapshot;
pub mod types;

pub mod feedback;

pub mod experience;
pub mod feedback_candidate;

pub mod temporal;

pub mod reflection_scope;

pub mod ledger_export;
