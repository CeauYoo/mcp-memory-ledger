use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::{
    domain::{claim::ClaimDraft, types::MemoryScope},
    error::AppError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ClaimStatus {
    Active,
    Disputed,
    Superseded,
}

impl ClaimStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disputed => "disputed",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StoredClaim {
    pub claim_id: String,
    pub claim: ClaimDraft,
    pub status: ClaimStatus,
    /// Application-clock ledger creation time. Legacy records remain unknown.
    #[serde(default)]
    pub recorded_at: Option<DateTime<Utc>>,
    /// Caller-supplied observation time, never an effective/valid-from time.
    #[serde(default)]
    pub observed_at: Option<String>,
}

impl StoredClaim {
    pub fn new(claim_id: String, claim: ClaimDraft, status: ClaimStatus) -> Self {
        Self {
            claim_id,
            claim,
            status,
            recorded_at: None,
            observed_at: None,
        }
    }

    pub fn with_recorded_at(mut self, recorded_at: DateTime<Utc>) -> Self {
        self.recorded_at = Some(recorded_at);
        self
    }

    pub fn with_temporal_metadata(
        mut self,
        recorded_at: Option<DateTime<Utc>>,
        observed_at: Option<String>,
    ) -> Self {
        self.recorded_at = recorded_at;
        self.observed_at = observed_at;
        self
    }

    pub fn snapshot_value(&self) -> String {
        format!(
            "{}:{} {} {}",
            self.claim.namespace(),
            self.claim.subject(),
            self.claim.predicate(),
            self.claim.object()
        )
    }
}

#[async_trait]
pub trait ClaimStore {
    async fn upsert_claim(&self, claim: StoredClaim) -> Result<(), AppError>;
    async fn link_evidence(&self, claim_id: String, event_id: String) -> Result<(), AppError>;
    async fn list_active_claims(&self) -> Result<Vec<StoredClaim>, AppError>;
    async fn list_active_claims_in_scope(
        &self,
        scope: &MemoryScope,
    ) -> Result<Vec<StoredClaim>, AppError> {
        Ok(self
            .list_active_claims()
            .await?
            .into_iter()
            .filter(|claim| {
                scope
                    .owner()
                    .is_none_or(|owner| claim.claim.owner() == owner)
                    && scope
                        .namespace()
                        .is_none_or(|namespace| claim.claim.namespace() == namespace)
            })
            .collect())
    }
    async fn update_claim_status(
        &self,
        claim_id: &str,
        status: ClaimStatus,
    ) -> Result<(), AppError>;
}
