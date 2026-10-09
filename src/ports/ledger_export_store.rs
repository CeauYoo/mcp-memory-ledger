use crate::{
    domain::ledger_export::{ExportMemoryRequest, LedgerExport},
    error::AppError,
};
use async_trait::async_trait;

#[async_trait]
pub trait LedgerExportStore {
    /// Read all selected facts in one consistent transaction. Never initializes,
    /// migrates, repairs indexes, logs an operation, or exports retry credentials.
    async fn export_memory(&self, request: ExportMemoryRequest) -> Result<LedgerExport, AppError>;
}
