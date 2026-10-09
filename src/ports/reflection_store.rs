use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{
    domain::{
        commitment::Commitment,
        reflection::{Reflection, ReflectionIdentityUpdate},
        reflection_scope::ReflectionScopeMetadata,
    },
    error::AppError,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredReflection {
    #[serde(default)]
    pub scope: ReflectionScopeMetadata,
    pub reflection_id: String,
    pub recorded_at: DateTime<Utc>,
    pub reflection: Reflection,
    pub superseded_claim_id: Option<String>,
    pub replacement_claim_id: Option<String>,
    pub supporting_evidence_event_ids: Vec<String>,
    pub requested_identity_update: Option<ReflectionIdentityUpdate>,
    pub requested_commitment_updates: Option<Vec<Commitment>>,
}

impl StoredReflection {
    pub fn new(
        reflection_id: String,
        recorded_at: DateTime<Utc>,
        reflection: Reflection,
        superseded_claim_id: Option<String>,
        replacement_claim_id: Option<String>,
    ) -> Self {
        Self {
            scope: ReflectionScopeMetadata::default(),
            reflection_id,
            recorded_at,
            reflection,
            superseded_claim_id,
            replacement_claim_id,
            supporting_evidence_event_ids: Vec::new(),
            requested_identity_update: None,
            requested_commitment_updates: None,
        }
    }

    pub fn with_scope(mut self, scope: ReflectionScopeMetadata) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_supporting_evidence_event_ids(
        mut self,
        supporting_evidence_event_ids: Vec<String>,
    ) -> Self {
        self.supporting_evidence_event_ids = supporting_evidence_event_ids;
        self
    }

    pub fn with_requested_identity_update(
        mut self,
        requested_identity_update: Option<ReflectionIdentityUpdate>,
    ) -> Self {
        self.requested_identity_update = requested_identity_update;
        self
    }

    pub fn with_requested_commitment_updates(
        mut self,
        requested_commitment_updates: Option<Vec<Commitment>>,
    ) -> Self {
        self.requested_commitment_updates = requested_commitment_updates;
        self
    }
}

#[async_trait]
pub trait ReflectionStore {
    async fn append_reflection(&self, reflection: StoredReflection) -> Result<(), AppError>;
}
