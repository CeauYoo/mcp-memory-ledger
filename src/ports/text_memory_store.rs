//! Bounded, scope-required, literal text recall. This is not semantic/vector search.
use crate::{domain::types::MemoryScope, error::AppError};
use async_trait::async_trait;

pub const MAX_RECALL_QUERY_BYTES: usize = 512;
pub const MAX_RECALL_TERMS: usize = 8;
pub const MAX_RECALL_LIMIT: usize = 100;

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
}

#[async_trait]
pub trait TextMemoryStore {
    async fn recall_text(&self, query: TextMemoryQuery) -> Result<TextMemoryPage, AppError>;
}
