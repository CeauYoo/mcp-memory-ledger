use async_trait::async_trait;

use crate::{
    domain::{commitment::Commitment, identity_core::IdentityCore},
    error::AppError,
};

pub mod claim_store;
pub mod clock;
pub mod commitment_store;
pub mod episode_store;
pub mod event_store;
pub mod id_generator;
pub mod identity_store;
pub mod memory_read_store;
pub mod model_port;
pub mod operation_log_store;
pub mod reflection_store;
pub mod trigger_ledger_store;
pub mod write_receipt;
pub use write_receipt::{StoredWriteReceipt, WriteReceiptRequest};

pub use crate::domain::self_revision::{
    SelfRevisionCommitmentPatch, SelfRevisionIdentityPatch, SelfRevisionPatch,
    SelfRevisionProposal, SelfRevisionRequest, TriggerType,
};
pub use claim_store::{ClaimStatus, ClaimStore, StoredClaim};
pub use clock::Clock;
pub use commitment_store::CommitmentStore;
pub use episode_store::EpisodeStore;
pub use event_store::{EventStore, EvidenceQuery, StoredEvent};
pub use id_generator::IdGenerator;
pub use identity_store::IdentityStore;
pub use memory_read_store::{
    ClaimReadRecord, ClaimRecordQuery, ClaimReflectionHistoryPage, ClaimReflectionHistoryQuery,
    ClaimReflectionHistoryRecord, ClaimRevisionLinks, EpisodeReadRecord, EpisodeRecordQuery,
    EventReadRecord, EventRecordQuery, MAX_EVENT_RECORD_QUERY_LIMIT, MemoryReadStore,
    ReflectionProvenanceLinks, ReflectionReadRecord, ReflectionRecordQuery, ScopedEventIdQuery,
    SelfModelHistoryKind, SelfModelHistoryPage, SelfModelHistoryQuery, SelfModelHistoryRecord,
};
pub use model_port::{ModelDecision, ModelDecisionRequest, ModelInput, ModelPort};
pub use operation_log_store::{OperationLogQuery, OperationLogStore};
pub use reflection_store::{ReflectionStore, StoredReflection};
pub use trigger_ledger_store::{StoredTriggerLedgerEntry, TriggerLedgerStatus, TriggerLedgerStore};

#[async_trait]
pub trait IngestTransaction {
    async fn load_event_for_ingest(
        &mut self,
        _event_id: &str,
    ) -> Result<Option<StoredEvent>, AppError> {
        Err(AppError::Message(
            "ingest transaction does not support evidence revalidation".into(),
        ))
    }
    async fn load_write_receipt(
        &mut self,
        _operation_id: &str,
    ) -> Result<Option<StoredWriteReceipt>, AppError> {
        Err(AppError::Message(
            "transaction does not support durable write receipts".into(),
        ))
    }
    async fn append_write_receipt(
        &mut self,
        _request: &WriteReceiptRequest,
        _receipt: StoredWriteReceipt,
        _recorded_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), AppError> {
        Err(AppError::Message(
            "transaction does not support durable write receipts".into(),
        ))
    }
    async fn append_event(&mut self, event: StoredEvent) -> Result<(), AppError>;
    async fn record_event_in_episode(
        &mut self,
        episode_reference: String,
        event_id: String,
    ) -> Result<(), AppError>;
    async fn upsert_claim(&mut self, claim: StoredClaim) -> Result<(), AppError>;
    async fn link_evidence(&mut self, claim_id: String, event_id: String) -> Result<(), AppError>;
    async fn commit(self: Box<Self>) -> Result<(), AppError>;
}

#[async_trait]
pub trait IngestTransactionRunner {
    async fn begin_ingest_transaction(
        &self,
    ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError>;
}

#[async_trait]
pub trait ReflectionTransaction {
    /// Returns the current head only after verifying both current projections.
    async fn load_current_self_model_version(
        &mut self,
    ) -> Result<crate::domain::self_model_version::SelfModelVersion, AppError> {
        Err(AppError::Message(
            "transaction does not support self-model versions".into(),
        ))
    }
    async fn load_self_model_version(
        &mut self,
        _version: u64,
    ) -> Result<Option<crate::domain::self_model_version::SelfModelVersion>, AppError> {
        Err(AppError::Message(
            "transaction does not support self-model versions".into(),
        ))
    }
    async fn load_self_model_reflection(
        &mut self,
        _reflection_id: &str,
    ) -> Result<Option<StoredReflection>, AppError> {
        Err(AppError::Message(
            "transaction does not support self-model provenance".into(),
        ))
    }
    async fn append_self_model_version(
        &mut self,
        _expected_version: u64,
        _version: crate::domain::self_model_version::SelfModelVersion,
    ) -> Result<(), AppError> {
        Err(AppError::Message(
            "transaction does not support self-model versions".into(),
        ))
    }
    async fn load_feedback_candidate(
        &mut self,
        _namespace: &crate::domain::types::Namespace,
        _candidate_id: &str,
    ) -> Result<Option<crate::domain::feedback_candidate::FeedbackCandidate>, AppError> {
        Err(AppError::Message(
            "transaction does not support feedback candidates".into(),
        ))
    }
    async fn list_feedback_candidates_for_target(
        &mut self,
        _namespace: &crate::domain::types::Namespace,
        _target: &str,
        _version: &str,
    ) -> Result<Vec<crate::domain::feedback_candidate::FeedbackCandidate>, AppError> {
        Err(AppError::Message(
            "transaction does not support feedback candidates".into(),
        ))
    }
    async fn insert_feedback_candidate(
        &mut self,
        _candidate: &crate::domain::feedback_candidate::FeedbackCandidate,
    ) -> Result<(), AppError> {
        Err(AppError::Message(
            "transaction does not support feedback candidates".into(),
        ))
    }
    async fn update_feedback_candidate(
        &mut self,
        _candidate: &crate::domain::feedback_candidate::FeedbackCandidate,
        _expected_revision: u64,
    ) -> Result<(), AppError> {
        Err(AppError::Message(
            "transaction does not support feedback candidates".into(),
        ))
    }
    async fn load_write_receipt(
        &mut self,
        _operation_id: &str,
    ) -> Result<Option<StoredWriteReceipt>, AppError> {
        Err(AppError::Message(
            "transaction does not support durable write receipts".into(),
        ))
    }
    async fn append_write_receipt(
        &mut self,
        _request: &WriteReceiptRequest,
        _receipt: StoredWriteReceipt,
        _recorded_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), AppError> {
        Err(AppError::Message(
            "transaction does not support durable write receipts".into(),
        ))
    }
    /// Reads and conditional transitions must use the same transaction as reflection writes.
    async fn load_claim_for_reflection(
        &mut self,
        claim_id: &str,
    ) -> Result<Option<StoredClaim>, AppError>;
    async fn load_event_for_reflection(
        &mut self,
        event_id: &str,
    ) -> Result<Option<StoredEvent>, AppError>;
    async fn compare_and_set_claim_status(
        &mut self,
        claim_id: &str,
        expected: ClaimStatus,
        status: ClaimStatus,
    ) -> Result<(), AppError>;
    async fn upsert_claim(&mut self, claim: StoredClaim) -> Result<(), AppError>;
    async fn link_evidence(&mut self, claim_id: String, event_id: String) -> Result<(), AppError>;
    async fn append_reflection(&mut self, reflection: StoredReflection) -> Result<(), AppError>;
    async fn append_trigger_ledger(
        &mut self,
        _entry: StoredTriggerLedgerEntry,
    ) -> Result<(), AppError> {
        Err(AppError::Message(
            "reflection transaction does not support trigger ledger writes".to_string(),
        ))
    }
    async fn load_identity(&mut self) -> Result<IdentityCore, AppError> {
        Err(AppError::Message(
            "reflection transaction does not support identity updates".to_string(),
        ))
    }
    async fn replace_identity(&mut self, _identity: IdentityCore) -> Result<(), AppError> {
        Err(AppError::Message(
            "reflection transaction does not support identity updates".to_string(),
        ))
    }
    async fn load_commitments(&mut self) -> Result<Vec<Commitment>, AppError> {
        Err(AppError::Message(
            "reflection transaction does not support commitment updates".to_string(),
        ))
    }
    async fn replace_commitments(&mut self, _commitments: Vec<Commitment>) -> Result<(), AppError> {
        Err(AppError::Message(
            "reflection transaction does not support commitment updates".to_string(),
        ))
    }
    async fn update_claim_status(
        &mut self,
        claim_id: &str,
        status: ClaimStatus,
    ) -> Result<(), AppError>;
    async fn commit(self: Box<Self>) -> Result<(), AppError>;
}

#[async_trait]
pub trait ReflectionTransactionRunner {
    async fn begin_reflection_transaction(
        &self,
    ) -> Result<Box<dyn ReflectionTransaction + Send + '_>, AppError>;
}

pub mod text_memory_store;

pub mod experience_store;
pub mod feedback_candidate_store;

pub mod ledger_export_store;

pub mod self_model_version_store;
