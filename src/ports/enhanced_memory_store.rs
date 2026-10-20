//! Bounded authoritative source windows for opt-in retrieval only.
use crate::{
    application::search_memory::SearchMemoryRecord, domain::types::MemoryScope, error::AppError,
};
use async_trait::async_trait;
#[derive(Debug, Clone)]
pub struct EnhancedRecordPage {
    pub records: Vec<SearchMemoryRecord>,
    pub has_more: bool,
    pub oversized: usize,
}
#[async_trait]
pub trait EnhancedMemoryStore {
    /// Scope and deterministic limit are applied before payload materialization.
    async fn scan_enhanced_records(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<EnhancedRecordPage, AppError>;
    /// Full scoped record; None includes oversized/unmaterialized sources. Corruption is an error; provenance is never truncated.
    async fn hydrate_enhanced_record(
        &self,
        scope: MemoryScope,
        reference: &str,
    ) -> Result<Option<SearchMemoryRecord>, AppError>;
    /// Typed unified source hydration, including current-active experiences. Whole provenance only.
    async fn hydrate_selection_source(
        &self,
        scope: MemoryScope,
        reference: &str,
    ) -> Result<Option<crate::application::selection_source::SelectionSource>, AppError> {
        let _ = (scope, reference);
        Err(AppError::Message(
            "typed selection hydration unsupported".into(),
        ))
    }
    /// Opaque conservative source revision; not a truth or authority token.
    async fn selection_source_generation(&self, scope: MemoryScope) -> Result<String, AppError> {
        let _ = scope;
        Err(AppError::Message(
            "selection generation validation unsupported".into(),
        ))
    }
}
