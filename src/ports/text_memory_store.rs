//! Bounded, scope-required, literal text recall. This is not semantic/vector search.
use crate::{
    domain::{event::EventReference, experience::PersistedEpisode, types::MemoryScope},
    error::AppError,
};
use async_trait::async_trait;
use serde::Serialize;

pub const MAX_RECALL_QUERY_BYTES: usize = 512;
pub const MAX_RECALL_TERMS: usize = 8;
pub const MAX_RECALL_LIMIT: usize = 100;
pub const CLAIM_STATUS_SAMPLE_LIMIT: usize = 256;
pub const CLAIM_STATUS_REFERENCE_LIMIT: usize = 8;
pub const LINKED_EPISODE_SOURCE_LIMIT: usize = 64;
pub const LINKED_EPISODE_LIMIT: usize = 8;

#[derive(Debug, Clone)]
pub struct LinkedEpisodeQuery {
    pub scope: MemoryScope,
    pub event_references: Vec<EventReference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedEpisodePage {
    pub records: Vec<PersistedEpisode>,
    pub has_more: bool,
    pub unavailable: usize,
}

#[derive(Debug, Clone)]
pub struct TextMemoryQuery {
    pub scope: MemoryScope,
    pub terms: Vec<String>,
    pub limit: usize,
}

impl TextMemoryQuery {
    pub fn validate(&self) -> Result<(), AppError> {
        if !self.scope.is_explicitly_scoped()
            || self.limit == 0
            || self.limit > MAX_RECALL_LIMIT
            || self.terms.is_empty()
            || self.terms.len() > MAX_RECALL_TERMS
            || self
                .terms
                .iter()
                .any(|term| term.is_empty() || term.contains('\0'))
            || self.terms.iter().map(String::len).sum::<usize>() > MAX_RECALL_QUERY_BYTES
        {
            return Err(AppError::InvalidParams(
                "invalid scoped text recall query".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextMemoryReference {
    Claim(String),
    Event(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMemoryHit {
    pub reference: TextMemoryReference,
    pub matched_terms: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMemoryPage {
    pub hits: Vec<TextMemoryHit>,
    pub has_more: bool,
    pub strategy: &'static str,
    /// Present only when the derived index could not safely be used.
    pub index_warning: Option<String>,
}

/// Literal matches in a deterministic, scoped source window, never an assumed total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextClaimStatusSample {
    pub sampled_claim_count: usize,
    /// A lower bound on all scoped matches unless `scope_scan_complete` is true.
    pub sampled_match_count: usize,
    pub scope_scan_complete: bool,
    pub claim_references: Vec<String>,
    pub references_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextClaimStatusDiagnostics {
    pub sample_limit_per_status: usize,
    pub reference_limit_per_status: usize,
    pub disputed: TextClaimStatusSample,
    pub superseded: TextClaimStatusSample,
}

#[async_trait]
pub trait TextMemoryStore {
    async fn recall_text(&self, query: TextMemoryQuery) -> Result<TextMemoryPage, AppError>;

    /// Optional read-only context diagnostics. Raw recall does not invoke this method.
    /// Implementations must scope before sampling and expose incomplete scans honestly.
    async fn inspect_text_claim_status(
        &self,
        query: TextMemoryQuery,
    ) -> Result<Option<TextClaimStatusDiagnostics>, AppError> {
        query.validate()?;
        Ok(None)
    }

    /// Complete inert Episode snapshots reached only through selected scoped evidence.
    async fn linked_episodes(
        &self,
        _query: LinkedEpisodeQuery,
    ) -> Result<Option<LinkedEpisodePage>, AppError> {
        Ok(None)
    }
}
