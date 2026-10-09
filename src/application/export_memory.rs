use crate::{
    domain::ledger_export::{ExportMemoryRequest, LedgerExport},
    error::AppError,
    ports::ledger_export_store::LedgerExportStore,
};

pub async fn export_memory<S: LedgerExportStore + Sync + ?Sized>(
    store: &S,
    request: ExportMemoryRequest,
) -> Result<LedgerExport, AppError> {
    request.validate()?;
    store.export_memory(request).await
}
