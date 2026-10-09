//! Best-effort operation audit storage and filtered diagnostics.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::Row;

use crate::{
    domain::operation_log::{OperationLogEntry, redact_secrets},
    error::AppError,
    ports::{OperationLogQuery, OperationLogStore},
};

use super::rows::{
    map_sqlite, parse_actor_kind, parse_operation_log_kind, parse_operation_log_status,
};

use super::SqliteStore;

#[async_trait]
impl OperationLogStore for SqliteStore {
    async fn append_operation(&self, entry: OperationLogEntry) -> Result<(), AppError> {
        map_sqlite(
            sqlx::query(
                "INSERT INTO operation_log (operation_id, occurred_at, namespace, actor_kind, actor_id, entrypoint, operation_kind, status, correlation_id, request_summary_json, response_summary_json, diagnostic_summary_json, redaction_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&entry.operation_id)
            .bind(entry.occurred_at.to_rfc3339())
            .bind(&entry.namespace)
            .bind(entry.actor_kind.as_str())
            .bind(&entry.actor_id)
            .bind(&entry.entrypoint)
            .bind(entry.operation_kind.as_str())
            .bind(entry.status.as_str())
            .bind(&entry.correlation_id)
            .bind(entry.request_summary_json.as_deref().map(redact_secrets))
            .bind(entry.response_summary_json.as_deref().map(redact_secrets))
            .bind(entry.diagnostic_summary_json.as_deref().map(redact_secrets))
            .bind(entry.redaction_version)
            .execute(&self.pool)
            .await,
        )?;
        Ok(())
    }

    async fn query_operations(
        &self,
        query: OperationLogQuery,
    ) -> Result<Vec<OperationLogEntry>, AppError> {
        let mut sql = String::from(
            "SELECT operation_id, occurred_at, namespace, actor_kind, actor_id, entrypoint, operation_kind, status, correlation_id, request_summary_json, response_summary_json, diagnostic_summary_json, redaction_version FROM operation_log",
        );
        let mut predicates = Vec::new();

        if query.operation_id.is_some() {
            predicates.push("operation_id = ?");
        }
        if query.namespace.is_some() {
            predicates.push("namespace = ?");
        }
        if query.operation_kind.is_some() {
            predicates.push("operation_kind = ?");
        }
        if query.status.is_some() {
            predicates.push("status = ?");
        }
        if query.correlation_id.is_some() {
            predicates.push("correlation_id = ?");
        }
        if query.after.is_some() {
            predicates.push("occurred_at > ?");
        }
        if query.before.is_some() {
            predicates.push("occurred_at < ?");
        }

        if !predicates.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&predicates.join(" AND "));
        }

        sql.push_str(" ORDER BY occurred_at DESC, operation_id DESC");
        if query.limit.is_some() {
            sql.push_str(" LIMIT ?");
        }

        let mut query_builder = sqlx::query(&sql);

        if let Some(ref operation_id) = query.operation_id {
            query_builder = query_builder.bind(operation_id.clone());
        }
        if let Some(ref ns) = query.namespace {
            query_builder = query_builder.bind(ns.clone());
        }
        if let Some(ref kind) = query.operation_kind {
            query_builder = query_builder.bind(kind.clone());
        }
        if let Some(ref status) = query.status {
            query_builder = query_builder.bind(status.clone());
        }
        if let Some(ref correlation_id) = query.correlation_id {
            query_builder = query_builder.bind(correlation_id.clone());
        }
        if let Some(after) = query.after {
            query_builder = query_builder.bind(after.to_rfc3339());
        }
        if let Some(before) = query.before {
            query_builder = query_builder.bind(before.to_rfc3339());
        }
        if let Some(limit) = query.limit {
            let limit = i64::try_from(limit).map_err(|_| {
                AppError::InvalidParams(
                    "operation log query limit exceeds the supported maximum".to_string(),
                )
            })?;
            query_builder = query_builder.bind(limit);
        }

        let rows = map_sqlite(query_builder.fetch_all(&self.pool).await)?;

        rows.iter()
            .map(|row| {
                let occurred_at_str: String = row.get("occurred_at");
                let occurred_at = DateTime::parse_from_rfc3339(&occurred_at_str)
                    .map_err(|e| AppError::Message(e.to_string()))?
                    .with_timezone(&Utc);
                let actor_kind_str: String = row.get("actor_kind");
                let operation_kind_str: String = row.get("operation_kind");
                let status_str: String = row.get("status");

                Ok(OperationLogEntry {
                    operation_id: row.get("operation_id"),
                    occurred_at,
                    namespace: row.get("namespace"),
                    actor_kind: parse_actor_kind(&actor_kind_str)?,
                    actor_id: row.get("actor_id"),
                    entrypoint: row.get("entrypoint"),
                    operation_kind: parse_operation_log_kind(&operation_kind_str)?,
                    status: parse_operation_log_status(&status_str)?,
                    correlation_id: row.get("correlation_id"),
                    request_summary_json: row.get("request_summary_json"),
                    response_summary_json: row.get("response_summary_json"),
                    diagnostic_summary_json: row.get("diagnostic_summary_json"),
                    redaction_version: row.get("redaction_version"),
                })
            })
            .collect()
    }
}
