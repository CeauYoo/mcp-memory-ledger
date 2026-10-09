use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{
    domain::{
        claim::ClaimReference,
        commitment::Commitment,
        event::EventReference,
        reflection::ReflectionIdentityUpdate,
        reflection_scope::ReflectionScopeMetadata,
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
};

use super::{ClaimStatus, StoredClaim, StoredEvent};

pub const MAX_EVENT_RECORD_QUERY_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRecordQuery {
    pub scope: MemoryScope,
    pub event_reference: Option<EventReference>,
    pub kind: Option<EventKind>,
    pub recorded_after: Option<DateTime<Utc>>,
    pub recorded_before: Option<DateTime<Utc>>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventReadRecord {
    pub event: StoredEvent,
    pub claim_ids: Vec<String>,
    pub episode_references: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeRecordQuery {
    pub scope: MemoryScope,
    pub episode_reference: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeReadRecord {
    pub episode_reference: String,
    pub recorded_at: DateTime<Utc>,
    pub owner: Owner,
    pub namespace: Namespace,
    pub event_references: Vec<EventReference>,
    pub claim_references: Vec<ClaimReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRecordQuery {
    pub scope: MemoryScope,
    pub claim_reference: Option<ClaimReference>,
    pub status: Option<ClaimStatus>,
    pub mode: Option<Mode>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaimRevisionLinks {
    pub source_reflection_id: Option<String>,
    pub supersedes_claim_reference: Option<ClaimReference>,
    pub superseded_by_reflection_id: Option<String>,
    pub replacement_claim_reference: Option<ClaimReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReadRecord {
    pub claim: StoredClaim,
    pub evidence_event_references: Vec<EventReference>,
    pub episode_references: Vec<String>,
    pub revision: ClaimRevisionLinks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionRecordQuery {
    pub scope: MemoryScope,
    pub reflection_reference: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReflectionProvenanceLinks {
    pub superseded_claim_reference: Option<ClaimReference>,
    pub replacement_claim_reference: Option<ClaimReference>,
    pub supporting_evidence_event_references: Vec<EventReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionReadRecord {
    pub scope: ReflectionScopeMetadata,
    pub reflection_id: String,
    pub recorded_at: DateTime<Utc>,
    pub owner: Owner,
    pub namespace: Namespace,
    pub summary: String,
    pub provenance: ReflectionProvenanceLinks,
}

impl ReflectionReadRecord {
    pub fn new(
        reflection_id: String,
        recorded_at: DateTime<Utc>,
        owner: Owner,
        namespace: Namespace,
        summary: String,
        provenance: ReflectionProvenanceLinks,
    ) -> Self {
        Self {
            scope: ReflectionScopeMetadata::default(),
            reflection_id,
            recorded_at,
            owner,
            namespace,
            summary,
            provenance,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopedEventIdQuery {
    pub scope: MemoryScope,
    pub event_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReflectionHistoryQuery {
    pub scope: MemoryScope,
    pub claim_reference: ClaimReference,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReflectionHistoryRecord {
    pub scope: ReflectionScopeMetadata,
    pub reflection_id: String,
    pub recorded_at: DateTime<Utc>,
    pub summary: String,
    pub superseded_claim_reference: Option<ClaimReference>,
    pub replacement_claim_reference: Option<ClaimReference>,
    pub supporting_evidence_event_references: Vec<EventReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReflectionHistoryPage {
    pub records: Vec<ClaimReflectionHistoryRecord>,
    pub has_more: bool,
}

impl ClaimReadRecord {
    pub fn new(
        claim: StoredClaim,
        evidence_event_references: Vec<EventReference>,
        episode_references: Vec<String>,
        revision: ClaimRevisionLinks,
    ) -> Self {
        Self {
            claim,
            evidence_event_references,
            episode_references,
            revision,
        }
    }
}

impl EventReadRecord {
    pub fn new(
        event: StoredEvent,
        claim_ids: Vec<String>,
        episode_references: Vec<String>,
    ) -> Self {
        Self {
            event,
            claim_ids,
            episode_references,
        }
    }
}

impl EpisodeReadRecord {
    pub fn new(
        episode_reference: String,
        recorded_at: DateTime<Utc>,
        owner: Owner,
        namespace: Namespace,
        event_references: Vec<EventReference>,
        claim_references: Vec<ClaimReference>,
    ) -> Self {
        Self {
            episode_reference,
            recorded_at,
            owner,
            namespace,
            event_references,
            claim_references,
        }
    }
}

#[async_trait]
pub trait MemoryReadStore {
    /// Union candidate reads apply descending recording time and descending
    /// stable ID before the per-type limit. Adapters should override these when
    /// standalone browsing uses a different tie-break or has unknown times.
    async fn query_event_records_for_union(
        &self,
        query: EventRecordQuery,
    ) -> Result<Vec<EventReadRecord>, AppError> {
        self.query_event_records(query).await
    }

    async fn query_claim_records_for_union(
        &self,
        query: ClaimRecordQuery,
    ) -> Result<Vec<ClaimReadRecord>, AppError> {
        self.query_claim_records(query).await
    }

    async fn query_episode_records_for_union(
        &self,
        query: EpisodeRecordQuery,
    ) -> Result<Vec<EpisodeReadRecord>, AppError> {
        self.query_episode_records(query).await
    }

    async fn query_reflection_records_for_union(
        &self,
        query: ReflectionRecordQuery,
    ) -> Result<Vec<ReflectionReadRecord>, AppError> {
        self.query_reflection_records(query).await
    }

    async fn query_event_records(
        &self,
        query: EventRecordQuery,
    ) -> Result<Vec<EventReadRecord>, AppError>;

    async fn query_claim_records(
        &self,
        query: ClaimRecordQuery,
    ) -> Result<Vec<ClaimReadRecord>, AppError>;

    async fn query_episode_records(
        &self,
        query: EpisodeRecordQuery,
    ) -> Result<Vec<EpisodeReadRecord>, AppError>;

    async fn query_reflection_records(
        &self,
        query: ReflectionRecordQuery,
    ) -> Result<Vec<ReflectionReadRecord>, AppError>;

    async fn query_scoped_event_ids(
        &self,
        query: ScopedEventIdQuery,
    ) -> Result<BTreeSet<String>, AppError>;

    async fn query_claim_reflection_history(
        &self,
        query: ClaimReflectionHistoryQuery,
    ) -> Result<ClaimReflectionHistoryPage, AppError>;

    async fn query_self_model_history(
        &self,
        query: SelfModelHistoryQuery,
    ) -> Result<SelfModelHistoryPage, AppError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfModelHistoryKind {
    Identity,
    Commitment,
}

impl SelfModelHistoryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Commitment => "commitment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfModelHistoryQuery {
    pub scope: MemoryScope,
    pub history_kind: SelfModelHistoryKind,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfModelHistoryRecord {
    pub scope: ReflectionScopeMetadata,
    pub reflection_id: String,
    pub recorded_at: DateTime<Utc>,
    pub summary: String,
    pub superseded_claim_reference: Option<ClaimReference>,
    pub replacement_claim_reference: Option<ClaimReference>,
    pub supporting_evidence_event_references: Vec<EventReference>,
    pub identity_update: Option<ReflectionIdentityUpdate>,
    pub commitment_updates: Option<Vec<Commitment>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelfModelHistoryPage {
    pub records: Vec<SelfModelHistoryRecord>,
    pub has_more: bool,
}
