//! Owner-dashboard discovery only. Never registered as a scoped MCP operation.
use super::SqliteStore;
use crate::{domain::task::scope, error::AppError};

impl SqliteStore {
    /// Task namespaces, not registered roots or host/session bindings. The extra
    /// row lets the caller report a bounded window without claiming completeness.
    pub async fn owner_task_namespaces(&self) -> Result<Vec<String>, AppError> {
        let namespaces: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT namespace FROM task_heads ORDER BY namespace LIMIT 101",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|_| AppError::Message("task workspace discovery unavailable".into()))?;
        for namespace in &namespaces {
            scope(namespace)
                .map_err(|_| AppError::Message("invalid task workspace record".into()))?;
        }
        Ok(namespaces)
    }
}
