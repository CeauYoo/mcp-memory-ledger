//! Best-effort operation logging, safe diagnostics, and MCP error conversion.

use anyhow::Result;
use chrono::Utc;
use rmcp::ErrorData as McpError;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    application::auto_reflect_if_needed,
    domain::operation_log::{ActorKind, OperationLogEntry, OperationLogKind, OperationLogStatus},
    domain::self_revision::TriggerType,
    error::AppError,
    interfaces::dashboard::{DashboardObserver, OperationStatus},
    ports::{OperationLogStore, TriggerLedgerStatus},
};

use super::AUTO_REFLECTION_RUNTIME_HOOKS;
use super::runtime::Runtime;

impl Runtime {
    pub(super) async fn record_tool_operation(&self, record: ToolOperationRecord) {
        let entry = OperationLogEntry {
            operation_id: Uuid::new_v4().to_string(),
            occurred_at: Utc::now(),
            namespace: record.namespace,
            actor_kind: ActorKind::System,
            actor_id: "mcp-stdio".to_string(),
            entrypoint: record.entrypoint.to_string(),
            operation_kind: OperationLogKind::Tool,
            status: record.status,
            correlation_id: record.correlation_id,
            request_summary_json: record.request_summary.map(|value| value.to_string()),
            response_summary_json: record.response_summary.map(|value| value.to_string()),
            diagnostic_summary_json: record.diagnostic_summary.map(|value| value.to_string()),
            redaction_version: 1,
        };

        if let Err(error) = self.store.append_operation(entry).await {
            warn!(
                entrypoint = record.entrypoint,
                error = %error,
                "failed to append MCP tool operation log entry"
            );
        }
    }

    /// B1：把 auto-reflection 的诊断落到 durable operation log（OperationLogKind::Trigger）。
    /// 在此之前没有任何代码写 Trigger 条目，导致 doctor 的 trigger_candidates_suppressed
    /// 运行时恒为 0（只有测试手动 seed 才非零）。这里 best-effort 写入：失败仅 warn，
    /// 绝不影响主工具流程，与上面的 record_tool_operation 语义一致。
    ///
    /// 只写「有诊断价值」的结局（Handled/Rejected/Suppressed/Pending）；NotTriggered（→ Ok）
    /// 是绝大多数 ingest 的常态，doctor 也只统计 Failed/Suppressed，写它纯属噪声且会污染
    /// operation-log 历史，故在此提前返回跳过。
    pub(super) async fn record_auto_reflection_operation(
        &self,
        entrypoint: &'static str,
        result: &auto_reflect_if_needed::AutoReflectResult,
        namespace: Option<String>,
        correlation_id: Option<String>,
    ) {
        let status = auto_reflection_operation_status(result);
        if status == OperationLogStatus::Ok {
            return;
        }
        let diagnostic_summary_json = Some(auto_reflection_diagnostic_summary(result).to_string());
        let entry = OperationLogEntry {
            operation_id: Uuid::new_v4().to_string(),
            occurred_at: Utc::now(),
            namespace,
            actor_kind: ActorKind::System,
            actor_id: "mcp-stdio".to_string(),
            entrypoint: entrypoint.to_string(),
            operation_kind: OperationLogKind::Trigger,
            status,
            correlation_id,
            request_summary_json: None,
            response_summary_json: None,
            diagnostic_summary_json,
            redaction_version: 1,
        };

        if let Err(error) = self.store.append_operation(entry).await {
            warn!(
                entrypoint,
                error = %error,
                "failed to append auto-reflection trigger operation log entry"
            );
        }
    }

    pub(super) async fn record_auto_reflection_failure_operation(
        &self,
        entrypoint: &'static str,
        namespace: Option<String>,
        correlation_id: Option<String>,
        trigger_type: TriggerType,
        trigger_key: &str,
        error: &AppError,
    ) {
        let status = auto_reflection_failure_operation_status(error);
        let entry = OperationLogEntry {
            operation_id: Uuid::new_v4().to_string(),
            occurred_at: Utc::now(),
            namespace,
            actor_kind: ActorKind::System,
            actor_id: "mcp-stdio".to_string(),
            entrypoint: entrypoint.to_string(),
            operation_kind: OperationLogKind::Trigger,
            status,
            correlation_id,
            request_summary_json: None,
            response_summary_json: None,
            diagnostic_summary_json: Some(
                auto_reflection_failure_diagnostic_summary(trigger_type, trigger_key, error)
                    .to_string(),
            ),
            redaction_version: 1,
        };

        if let Err(error) = self.store.append_operation(entry).await {
            warn!(
                entrypoint,
                error = %error,
                "failed to append failed auto-reflection trigger operation log entry"
            );
        }
    }
}

pub(super) struct ToolOperationRecord {
    entrypoint: &'static str,
    namespace: Option<String>,
    status: OperationLogStatus,
    correlation_id: Option<String>,
    request_summary: Option<serde_json::Value>,
    response_summary: Option<serde_json::Value>,
    diagnostic_summary: Option<serde_json::Value>,
}

impl ToolOperationRecord {
    pub(super) fn ok(
        entrypoint: &'static str,
        namespace: Option<String>,
        correlation_id: Option<String>,
    ) -> Self {
        Self {
            entrypoint,
            namespace,
            status: OperationLogStatus::Ok,
            correlation_id,
            request_summary: None,
            response_summary: None,
            diagnostic_summary: None,
        }
    }

    pub(super) fn with_response_summary(mut self, summary: serde_json::Value) -> Self {
        self.response_summary = Some(summary);
        self
    }

    pub(super) fn failed(
        entrypoint: &'static str,
        namespace: Option<String>,
        correlation_id: Option<String>,
    ) -> Self {
        Self {
            entrypoint,
            namespace,
            status: OperationLogStatus::Failed,
            correlation_id,
            request_summary: None,
            response_summary: None,
            diagnostic_summary: None,
        }
    }

    pub(super) fn with_diagnostic_summary(mut self, summary: serde_json::Value) -> Self {
        self.diagnostic_summary = Some(summary);
        self
    }
}

fn app_error_to_mcp(error: AppError) -> McpError {
    match error {
        AppError::InvalidParams(message) => McpError::invalid_params(message, None),
        AppError::Message(message) => McpError::internal_error(message, None),
    }
}

pub(super) async fn map_tool_error<T>(
    runtime: &Runtime,
    operation: &'static str,
    namespace: Option<String>,
    correlation_id: Option<String>,
    result: Result<T, AppError>,
) -> Result<T, McpError> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            Err(record_app_error_to_mcp(runtime, operation, namespace, correlation_id, error).await)
        }
    }
}

#[derive(Clone, Copy)]
enum McpErrorClass {
    InvalidParams,
    InternalError,
}

impl McpErrorClass {
    fn label(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid params",
            Self::InternalError => "internal error",
        }
    }

    fn code(self) -> i64 {
        match self {
            Self::InvalidParams => -32602,
            Self::InternalError => -32603,
        }
    }
}

fn mcp_error_class(error: &AppError) -> McpErrorClass {
    match error {
        AppError::InvalidParams(_) => McpErrorClass::InvalidParams,
        AppError::Message(_) => McpErrorClass::InternalError,
    }
}

async fn record_app_error_to_mcp(
    runtime: &Runtime,
    operation: &'static str,
    namespace: Option<String>,
    correlation_id: Option<String>,
    error: AppError,
) -> McpError {
    let error_class = mcp_error_class(&error);
    let summary = format!("{operation} failed");
    let diagnostic_detail = safe_diagnostic_detail(&error);
    runtime.dashboard.record_tool_failed(
        operation,
        namespace.clone(),
        correlation_id.clone(),
        summary,
        diagnostic_detail.to_string(),
    );
    runtime
        .record_tool_operation(
            ToolOperationRecord::failed(operation, namespace, correlation_id)
                .with_diagnostic_summary(serde_json::json!({
                    "mcp_error_class": error_class.label(),
                    "mcp_error_code": error_class.code(),
                    "mcp_error_detail": diagnostic_detail,
                })),
        )
        .await;
    app_error_to_mcp(error)
}

fn safe_diagnostic_detail(error: &AppError) -> &'static str {
    match error {
        AppError::InvalidParams(message) if message.contains("missing field") => "missing field",
        AppError::InvalidParams(message) if message.contains("invalid type") => "invalid type",
        AppError::InvalidParams(_) => "invalid params",
        AppError::Message(_) => "internal error",
    }
}

fn auto_reflection_diagnostic_summary(
    result: &auto_reflect_if_needed::AutoReflectResult,
) -> serde_json::Value {
    let diagnostics = &result.diagnostics;
    serde_json::json!({
        "trigger_type": diagnostics.trigger_type,
        "namespace": diagnostics.namespace,
        "trigger_key": diagnostics.trigger_key,
        "ledger_status": result.ledger_status.map(TriggerLedgerStatus::as_str),
        "reflection_id": result.reflection_id,
        "outcome": diagnostics.outcome,
        "suppression_reason": diagnostics.suppression_reason,
        "suppression_category": diagnostics.suppression_category,
        "rejection_reason": diagnostics
            .rejection_reason
            .as_ref()
            .map(|_| "model_rationale_omitted"),
        "cooldown_boundary": diagnostics.cooldown_boundary.map(|value| value.to_rfc3339()),
        "cooldown_state": diagnostics.cooldown_state,
        "evidence_window_size": diagnostics.evidence_window_size,
        "selected_evidence_event_ids": diagnostics.selected_evidence_event_ids,
        "durable_write_path": diagnostics.durable_write_path,
    })
}

fn auto_reflection_failure_diagnostic_summary(
    trigger_type: TriggerType,
    trigger_key: &str,
    error: &AppError,
) -> serde_json::Value {
    let error_class = mcp_error_class(error);
    let is_policy_rejection = is_auto_reflection_policy_rejection(error);
    serde_json::json!({
        "trigger_type": trigger_type,
        "trigger_key": trigger_key,
        "outcome": if is_policy_rejection { "rejected" } else { "failed" },
        "rejection_category": if is_policy_rejection { Some("governance_policy") } else { None },
        "error_class": error_class.label(),
        "error_code": error_class.code(),
        "error_detail": safe_diagnostic_detail(error),
    })
}

fn auto_reflection_failure_operation_status(error: &AppError) -> OperationLogStatus {
    if is_auto_reflection_policy_rejection(error) {
        OperationLogStatus::Rejected
    } else {
        OperationLogStatus::Failed
    }
}

fn is_auto_reflection_policy_rejection(error: &AppError) -> bool {
    matches!(error, AppError::InvalidParams(_))
}

pub(super) fn log_auto_reflection_success(
    runtime_hook: &'static str,
    result: &auto_reflect_if_needed::AutoReflectResult,
    event_id: Option<&str>,
    dashboard: &DashboardObserver,
    namespace: Option<String>,
    correlation_id: Option<String>,
) {
    info!(
        runtime_hook,
        event_id = ?event_id,
        triggered = result.triggered,
        trigger_type = ?result.trigger_type,
        trigger_key = ?result.trigger_key,
        ledger_status = ?result.ledger_status,
        reflection_id = ?result.reflection_id,
        suppression_reason = ?result.suppression_reason,
        rejection_reason = ?result
            .diagnostics
            .rejection_reason
            .as_ref()
            .map(|_| "model_rationale_omitted"),
        cooldown_until = ?result.cooldown_until,
        evidence_event_ids = ?result.evidence_event_ids,
        "best-effort auto-reflection completed"
    );
    let dashboard_payload = auto_reflection_diagnostic_summary(result);
    dashboard.record_auto_reflection(
        runtime_hook,
        namespace,
        correlation_id,
        auto_reflection_status(result),
        auto_reflection_summary(result),
        &dashboard_payload,
    );
}

pub(super) fn runtime_hook_for(source: &'static str, trigger_type: TriggerType) -> &'static str {
    match (source, trigger_type) {
        ("ingest_interaction", TriggerType::Failure) => AUTO_REFLECTION_RUNTIME_HOOKS[0],
        ("ingest_interaction", TriggerType::Conflict) => AUTO_REFLECTION_RUNTIME_HOOKS[1],
        ("decide_with_snapshot", TriggerType::Conflict) => AUTO_REFLECTION_RUNTIME_HOOKS[2],
        ("build_self_snapshot", TriggerType::Periodic) => AUTO_REFLECTION_RUNTIME_HOOKS[3],
        _ => "auto_reflection:unknown",
    }
}

fn auto_reflection_status(result: &auto_reflect_if_needed::AutoReflectResult) -> OperationStatus {
    match result.ledger_status {
        Some(TriggerLedgerStatus::Handled) => OperationStatus::Handled,
        Some(TriggerLedgerStatus::Rejected) => OperationStatus::Rejected,
        Some(TriggerLedgerStatus::Suppressed) => OperationStatus::Suppressed,
        Some(TriggerLedgerStatus::Pending) => OperationStatus::Started,
        None if result.triggered => OperationStatus::Handled,
        None => OperationStatus::Ok,
    }
}

/// 把 auto-reflection 结果映射到 durable operation log 的状态。与 doctor 的候选读取口径对齐：
/// 只有 Suppressed/Failed 会被 count_trigger_candidates 计入诊断，Handled/Rejected 留痕但不计数，
/// NotTriggered/Skipped（无 ledger_status 且未触发）落 Ok，避免污染 suppressed 计数。
fn auto_reflection_operation_status(
    result: &auto_reflect_if_needed::AutoReflectResult,
) -> OperationLogStatus {
    match result.ledger_status {
        Some(TriggerLedgerStatus::Handled) => OperationLogStatus::Handled,
        Some(TriggerLedgerStatus::Rejected) => OperationLogStatus::Rejected,
        Some(TriggerLedgerStatus::Suppressed) => OperationLogStatus::Suppressed,
        Some(TriggerLedgerStatus::Pending) => OperationLogStatus::Started,
        None if result.triggered => OperationLogStatus::Handled,
        None => OperationLogStatus::Ok,
    }
}

fn auto_reflection_summary(result: &auto_reflect_if_needed::AutoReflectResult) -> String {
    if let Some(reflection_id) = result.reflection_id.as_deref() {
        return format!("auto-reflection linked reflection {reflection_id}");
    }
    if let Some(reason) = result.suppression_reason.as_deref() {
        return format!("auto-reflection suppressed: {reason}");
    }
    if result.ledger_status == Some(TriggerLedgerStatus::Rejected) {
        return "auto-reflection rejected model proposal".to_string();
    }
    if result.reason.is_some() {
        return "auto-reflection checked runtime evidence".to_string();
    }
    "auto-reflection checked runtime evidence".to_string()
}
