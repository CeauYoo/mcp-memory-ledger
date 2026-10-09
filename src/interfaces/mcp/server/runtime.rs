//! Concrete model/store dependencies and application-port delegation.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::{
    adapters::{
        model::{mock::MockModel, openai_compatible::OpenAiCompatibleModel},
        sqlite::SqliteStore,
    },
    domain::event::EventReference,
    domain::identity_core::IdentityCore,
    domain::self_revision::{SelfRevisionProposal, SelfRevisionRequest},
    domain::snapshot::SnapshotTimeWindow,
    domain::types::MemoryScope,
    error::AppError,
    interfaces::dashboard::DashboardObserver,
    ports::{
        ClaimReadRecord, ClaimRecordQuery, ClaimReflectionHistoryPage, ClaimReflectionHistoryQuery,
        ClaimStatus, ClaimStore, Clock, CommitmentStore, EpisodeReadRecord, EpisodeRecordQuery,
        EpisodeStore, EventReadRecord, EventRecordQuery, EventStore, EvidenceQuery, IdGenerator,
        IdentityStore, IngestTransaction, IngestTransactionRunner, MemoryReadStore, ModelDecision,
        ModelDecisionRequest, ModelPort, ReflectionReadRecord, ReflectionRecordQuery,
        ReflectionStore, ReflectionTransaction, ReflectionTransactionRunner, ScopedEventIdQuery,
        SelfModelHistoryPage, SelfModelHistoryQuery, StoredClaim, StoredEvent, StoredReflection,
        StoredTriggerLedgerEntry, TriggerLedgerStore,
    },
    support::config::{AppConfig, ModelConfig},
};

#[derive(Clone)]
pub(super) struct Runtime {
    pub(super) store: SqliteStore,
    model: RuntimeModel,
    pub(super) dashboard: DashboardObserver,
}

#[derive(Clone)]
enum RuntimeModel {
    Mock(MockModel),
    OpenAiCompatible(OpenAiCompatibleModel),
}

impl Runtime {
    pub(super) async fn from_store(
        config: &AppConfig,
        store: SqliteStore,
        dashboard: DashboardObserver,
    ) -> Result<Self, AppError> {
        config.validate().map_err(AppError::Message)?;

        let runtime = Self {
            store,
            model: build_runtime_model(config)?,
            dashboard,
        };
        runtime.validate_default_identity().await?;
        Ok(runtime)
    }

    async fn validate_default_identity(&self) -> Result<(), AppError> {
        self.store.load_identity().await.map(|_| ())
    }
}

fn build_runtime_model(config: &AppConfig) -> Result<RuntimeModel, AppError> {
    match &config.model_config {
        ModelConfig::Mock => Ok(RuntimeModel::Mock(MockModel)),
        ModelConfig::OpenAiCompatible(model_config) => Ok(RuntimeModel::OpenAiCompatible(
            OpenAiCompatibleModel::new(model_config.clone())?,
        )),
        ModelConfig::OpenRouter(model_config) => Ok(RuntimeModel::OpenAiCompatible(
            OpenAiCompatibleModel::new_for_provider(model_config.clone(), "openrouter")?,
        )),
    }
}

#[async_trait]
impl Clock for Runtime {
    async fn now(&self) -> Result<DateTime<Utc>, AppError> {
        Ok(Utc::now())
    }
}

#[async_trait]
impl IdGenerator for Runtime {
    async fn next_id(&self) -> Result<String, AppError> {
        Ok(Uuid::new_v4().to_string())
    }
}

#[async_trait]
impl EventStore for Runtime {
    async fn append_event(&self, event: StoredEvent) -> Result<(), AppError> {
        self.store.append_event(event).await
    }

    async fn list_event_references(&self) -> Result<Vec<String>, AppError> {
        self.store.list_event_references().await
    }

    async fn list_event_references_in_scope(
        &self,
        scope: &MemoryScope,
        evidence_manifest: Option<&[EventReference]>,
    ) -> Result<Vec<String>, AppError> {
        self.store
            .list_event_references_in_scope(scope, evidence_manifest)
            .await
    }

    async fn list_event_references_for_snapshot(
        &self,
        scope: &MemoryScope,
        evidence_manifest: Option<&[EventReference]>,
        time_window: &SnapshotTimeWindow,
    ) -> Result<Vec<String>, AppError> {
        self.store
            .list_event_references_for_snapshot(scope, evidence_manifest, time_window)
            .await
    }

    async fn list_recorded_at_for_snapshot_manifest(
        &self,
        scope: &MemoryScope,
        evidence_manifest: &[EventReference],
    ) -> Result<Vec<DateTime<Utc>>, AppError> {
        self.store
            .list_recorded_at_for_snapshot_manifest(scope, evidence_manifest)
            .await
    }

    async fn query_evidence_event_ids(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        self.store.query_evidence_event_ids(query).await
    }

    async fn query_evidence_event_ids_unbounded(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        self.store.query_evidence_event_ids_unbounded(query).await
    }

    async fn has_event(&self, event_id: &str) -> Result<bool, AppError> {
        self.store.has_event(event_id).await
    }
}

#[async_trait]
impl MemoryReadStore for Runtime {
    async fn query_event_records_for_union(
        &self,
        query: EventRecordQuery,
    ) -> Result<Vec<EventReadRecord>, AppError> {
        self.store.query_event_records_for_union(query).await
    }
    async fn query_claim_records_for_union(
        &self,
        query: ClaimRecordQuery,
    ) -> Result<Vec<ClaimReadRecord>, AppError> {
        self.store.query_claim_records_for_union(query).await
    }
    async fn query_episode_records_for_union(
        &self,
        query: EpisodeRecordQuery,
    ) -> Result<Vec<EpisodeReadRecord>, AppError> {
        self.store.query_episode_records_for_union(query).await
    }
    async fn query_reflection_records_for_union(
        &self,
        query: ReflectionRecordQuery,
    ) -> Result<Vec<ReflectionReadRecord>, AppError> {
        self.store.query_reflection_records_for_union(query).await
    }
    async fn query_event_records(
        &self,
        query: EventRecordQuery,
    ) -> Result<Vec<EventReadRecord>, AppError> {
        self.store.query_event_records(query).await
    }

    async fn query_claim_records(
        &self,
        query: ClaimRecordQuery,
    ) -> Result<Vec<ClaimReadRecord>, AppError> {
        self.store.query_claim_records(query).await
    }

    async fn query_episode_records(
        &self,
        query: EpisodeRecordQuery,
    ) -> Result<Vec<EpisodeReadRecord>, AppError> {
        self.store.query_episode_records(query).await
    }

    async fn query_reflection_records(
        &self,
        query: ReflectionRecordQuery,
    ) -> Result<Vec<ReflectionReadRecord>, AppError> {
        self.store.query_reflection_records(query).await
    }

    async fn query_scoped_event_ids(
        &self,
        query: ScopedEventIdQuery,
    ) -> Result<std::collections::BTreeSet<String>, AppError> {
        self.store.query_scoped_event_ids(query).await
    }

    async fn query_claim_reflection_history(
        &self,
        query: ClaimReflectionHistoryQuery,
    ) -> Result<ClaimReflectionHistoryPage, AppError> {
        self.store.query_claim_reflection_history(query).await
    }

    async fn query_self_model_history(
        &self,
        query: SelfModelHistoryQuery,
    ) -> Result<SelfModelHistoryPage, AppError> {
        self.store.query_self_model_history(query).await
    }
}

#[async_trait]
impl ClaimStore for Runtime {
    async fn upsert_claim(&self, claim: StoredClaim) -> Result<(), AppError> {
        self.store.upsert_claim(claim).await
    }

    async fn link_evidence(&self, claim_id: String, event_id: String) -> Result<(), AppError> {
        self.store.link_evidence(claim_id, event_id).await
    }

    async fn list_active_claims(&self) -> Result<Vec<StoredClaim>, AppError> {
        self.store.list_active_claims().await
    }

    async fn list_active_claims_in_scope(
        &self,
        scope: &MemoryScope,
    ) -> Result<Vec<StoredClaim>, AppError> {
        self.store.list_active_claims_in_scope(scope).await
    }

    async fn update_claim_status(
        &self,
        claim_id: &str,
        status: ClaimStatus,
    ) -> Result<(), AppError> {
        self.store.update_claim_status(claim_id, status).await
    }
}

#[async_trait]
impl EpisodeStore for Runtime {
    async fn record_event_in_episode(
        &self,
        episode_reference: String,
        event_id: String,
    ) -> Result<(), AppError> {
        self.store
            .record_event_in_episode(episode_reference, event_id)
            .await
    }

    async fn list_episode_references(&self) -> Result<Vec<String>, AppError> {
        self.store.list_episode_references().await
    }

    async fn list_episode_references_supporting_claims(
        &self,
        scope: &MemoryScope,
        claim_ids: &[String],
    ) -> Result<Vec<String>, AppError> {
        self.store
            .list_episode_references_supporting_claims(scope, claim_ids)
            .await
    }

    async fn list_episode_references_in_scope(
        &self,
        scope: &MemoryScope,
    ) -> Result<Vec<String>, AppError> {
        self.store.list_episode_references_in_scope(scope).await
    }

    async fn list_episode_references_for_snapshot(
        &self,
        scope: &MemoryScope,
        time_window: &SnapshotTimeWindow,
    ) -> Result<Vec<String>, AppError> {
        self.store
            .list_episode_references_for_snapshot(scope, time_window)
            .await
    }
}

#[async_trait]
impl ReflectionStore for Runtime {
    async fn append_reflection(&self, reflection: StoredReflection) -> Result<(), AppError> {
        self.store.append_reflection(reflection).await
    }
}

#[async_trait]
impl TriggerLedgerStore for Runtime {
    async fn record_trigger_attempt(
        &self,
        entry: StoredTriggerLedgerEntry,
    ) -> Result<(), AppError> {
        self.store.record_trigger_attempt(entry).await
    }

    async fn latest_trigger_entry(
        &self,
        trigger_key: &str,
    ) -> Result<Option<StoredTriggerLedgerEntry>, AppError> {
        self.store.latest_trigger_entry(trigger_key).await
    }

    async fn latest_handled_trigger_entry(
        &self,
        trigger_key: &str,
    ) -> Result<Option<StoredTriggerLedgerEntry>, AppError> {
        self.store.latest_handled_trigger_entry(trigger_key).await
    }
}

#[async_trait]
impl IdentityStore for Runtime {
    async fn load_identity(&self) -> Result<IdentityCore, AppError> {
        self.store.load_identity().await
    }

    async fn save_identity(&self, identity: IdentityCore) -> Result<(), AppError> {
        self.store.save_identity(identity).await
    }
}

#[async_trait]
impl CommitmentStore for Runtime {
    async fn list_commitments(
        &self,
    ) -> Result<Vec<crate::domain::commitment::Commitment>, AppError> {
        self.store.list_commitments().await
    }
}

#[async_trait]
impl ModelPort for Runtime {
    async fn decide(&self, request: ModelDecisionRequest) -> Result<ModelDecision, AppError> {
        match &self.model {
            RuntimeModel::Mock(model) => model.decide(request).await,
            RuntimeModel::OpenAiCompatible(model) => model.decide(request).await,
        }
    }

    async fn propose_self_revision(
        &self,
        request: SelfRevisionRequest,
    ) -> Result<SelfRevisionProposal, AppError> {
        match &self.model {
            RuntimeModel::Mock(model) => model.propose_self_revision(request).await,
            RuntimeModel::OpenAiCompatible(model) => model.propose_self_revision(request).await,
        }
    }
}

#[async_trait]
impl IngestTransactionRunner for Runtime {
    async fn begin_ingest_transaction(
        &self,
    ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError> {
        self.store.begin_ingest_transaction().await
    }
}

#[async_trait]
impl ReflectionTransactionRunner for Runtime {
    async fn begin_reflection_transaction(
        &self,
    ) -> Result<Box<dyn ReflectionTransaction + Send + '_>, AppError> {
        self.store.begin_reflection_transaction().await
    }
}

#[async_trait]
impl crate::ports::feedback_candidate_store::FeedbackCandidateStore for Runtime {
    async fn get_feedback_candidate(
        &self,
        namespace: &crate::domain::types::Namespace,
        candidate_id: &str,
    ) -> Result<Option<crate::domain::feedback_candidate::FeedbackCandidate>, AppError> {
        crate::ports::feedback_candidate_store::FeedbackCandidateStore::get_feedback_candidate(
            &self.store,
            namespace,
            candidate_id,
        )
        .await
    }
}
