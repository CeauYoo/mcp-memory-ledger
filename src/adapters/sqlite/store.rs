use crate::error::AppError;
use sqlx::sqlite::SqlitePool;

mod operation_log;
mod ports;
mod reads;
mod rows;
mod schema_compat;
mod transactions;

pub(super) use schema_compat::{
    ensure_claims_namespace_column, ensure_events_namespace_column,
    ensure_reflection_audit_columns, seed_baseline_commitments,
};

#[derive(Clone)]
pub struct SqliteStore {
    pub(super) pool: SqlitePool,
}

impl SqliteStore {
    pub async fn bootstrap(database_url: &str) -> Result<Self, AppError> {
        super::lifecycle::bootstrap_database(database_url).await
    }
}
