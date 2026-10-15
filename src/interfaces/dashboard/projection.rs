use serde::Serialize;
use serde_json::Value;

use crate::domain::operation_log::{OperationLogEntry, OperationLogKind, OperationLogStatus};

use super::{OperationEvent, OperationKind, OperationStatus};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DashboardRuntimeInfo {
    pub service_name: String,
    pub transport: String,
    pub provider: String,
    pub dashboard_enabled: bool,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DashboardSummary {
    pub runtime: DashboardRuntimeInfo,
    pub total_events: usize,
    pub tool_events: usize,
    pub reflection_events: usize,
    pub decision_events: usize,
    pub snapshot_events: usize,
    pub failed_events: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OperationDetail {
    pub id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub operation: String,
    pub kind: OperationKind,
    pub status: OperationStatus,
    pub namespace: Option<String>,
    pub correlation_id: Option<String>,
    pub summary: String,
    pub payload: Value,
    pub read_only: bool,
}

pub fn build_summary(
    events: &[OperationEvent],
    runtime: &DashboardRuntimeInfo,
) -> DashboardSummary {
    DashboardSummary {
        runtime: runtime.clone(),
        total_events: events.len(),
        tool_events: count_kind(events, OperationKind::Tool),
        reflection_events: count_kind(events, OperationKind::Reflection),
        decision_events: count_kind(events, OperationKind::Decision),
        snapshot_events: count_kind(events, OperationKind::Snapshot),
        failed_events: events
            .iter()
            .filter(|event| event.status == OperationStatus::Failed)
            .count(),
    }
}

pub fn project_event_detail(event: &OperationEvent) -> OperationDetail {
    OperationDetail {
        id: event.id.clone(),
        timestamp: event.timestamp,
        operation: event.operation.clone(),
        kind: event.kind,
        status: event.status,
        namespace: event.namespace.clone(),
        correlation_id: event.correlation_id.clone(),
        summary: event.summary.clone(),
        payload: event.payload.clone(),
        read_only: true,
    }
}

fn count_kind(events: &[OperationEvent], kind: OperationKind) -> usize {
    events.iter().filter(|event| event.kind == kind).count()
}

pub fn project_from_log_entry(entry: &OperationLogEntry) -> OperationDetail {
    let kind = match entry.operation_kind {
        OperationLogKind::Startup => OperationKind::Startup,
        OperationLogKind::Tool => OperationKind::Tool,
        OperationLogKind::Trigger => OperationKind::Trigger,
        OperationLogKind::Reflection => OperationKind::Reflection,
        OperationLogKind::Decision => OperationKind::Decision,
        OperationLogKind::Snapshot => OperationKind::Snapshot,
        OperationLogKind::Doctor => OperationKind::Doctor,
        OperationLogKind::Error => OperationKind::Error,
    };
    let status = match entry.status {
        OperationLogStatus::Started => OperationStatus::Started,
        OperationLogStatus::Ok => OperationStatus::Ok,
        OperationLogStatus::Handled => OperationStatus::Handled,
        OperationLogStatus::Suppressed => OperationStatus::Suppressed,
        OperationLogStatus::Rejected => OperationStatus::Rejected,
        OperationLogStatus::Failed => OperationStatus::Failed,
    };
    OperationDetail {
        id: entry.operation_id.clone(),
        timestamp: entry.occurred_at,
        operation: entry.entrypoint.clone(),
        kind,
        status,
        namespace: entry.namespace.clone(),
        correlation_id: entry.correlation_id.clone(),
        summary: entry.response_summary_json.clone().unwrap_or_default(),
        payload: serde_json::json!({
            "actor_kind": entry.actor_kind,
            "actor_id": entry.actor_id,
            "request_summary_json": entry.request_summary_json,
            "response_summary_json": entry.response_summary_json,
            "diagnostic_summary_json": entry.diagnostic_summary_json,
            "redaction_version": entry.redaction_version,
        }),
        read_only: true,
    }
}
