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
}
