use agent_llm_mm::interfaces::dashboard::{
    DashboardRuntimeInfo, OperationEvent, OperationKind, OperationStatus, build_summary,
    project_event_detail,
};
use chrono::Utc;
use serde_json::json;

fn event(sequence: u64, kind: OperationKind, status: OperationStatus) -> OperationEvent {
    OperationEvent {
        id: format!("op_{sequence}"),
        sequence,
        timestamp: Utc::now(),
        kind,
        status,
        operation: "run_reflection".to_string(),
        namespace: Some("self".to_string()),
        summary: "reflection handled".to_string(),
        correlation_id: Some("corr-1".to_string()),
        payload: json!({ "reflection_id": "reflection-1" }),
    }
}

#[test]
fn summary_counts_events_by_kind_and_status() {
    let events = vec![
        event(1, OperationKind::Tool, OperationStatus::Ok),
        event(2, OperationKind::Reflection, OperationStatus::Handled),
        event(3, OperationKind::Reflection, OperationStatus::Rejected),
    ];
    let runtime = DashboardRuntimeInfo {
        service_name: "agent-llm-mm".to_string(),
        transport: "stdio".to_string(),
        provider: "mock".to_string(),
        dashboard_enabled: true,
        read_only: true,
    };

    let summary = build_summary(&events, &runtime);

    assert_eq!(summary.total_events, 3);
    assert_eq!(summary.reflection_events, 2);
    assert_eq!(summary.failed_events, 0);
    assert_eq!(summary.runtime.service_name, "agent-llm-mm");
}

#[test]
fn detail_projection_preserves_payload_and_read_only_boundary() {
    let detail = project_event_detail(&event(
        7,
        OperationKind::Reflection,
        OperationStatus::Handled,
    ));

    assert_eq!(detail.id, "op_7");
    assert_eq!(detail.operation, "run_reflection");
    assert!(detail.read_only);
    assert_eq!(detail.payload["reflection_id"], "reflection-1");
    assert_eq!(detail.correlation_id.as_deref(), Some("corr-1"));
}

#[test]
fn historical_detail_preserves_original_time_and_nullable_summaries() {
    use agent_llm_mm::domain::operation_log::{
        ActorKind, OperationLogEntry, OperationLogKind, OperationLogStatus,
    };
    use agent_llm_mm::interfaces::dashboard::project_from_log_entry;
    let entry = OperationLogEntry {
        operation_id: "old-record".into(),
        occurred_at: "2020-01-02T03:04:05Z".parse().unwrap(),
        namespace: Some("scope/a".into()),
        actor_kind: ActorKind::System,
        actor_id: "mcp-stdio".into(),
        entrypoint: "build_self_snapshot".into(),
        operation_kind: OperationLogKind::Tool,
        status: OperationLogStatus::Ok,
        correlation_id: Some("corr".into()),
        request_summary_json: None,
        response_summary_json: Some("{}".into()),
        diagnostic_summary_json: Some(r#"{"category":"synthetic"}"#.into()),
        redaction_version: 1,
    };
    let detail = project_from_log_entry(&entry);
    assert_eq!(detail.timestamp, entry.occurred_at);
    assert_eq!(detail.kind, OperationKind::Tool);
    assert_eq!(
        detail.payload["request_summary_json"],
        serde_json::Value::Null
    );
    assert_eq!(detail.payload["response_summary_json"], "{}");
    assert_eq!(
        detail.payload["diagnostic_summary_json"],
        r#"{"category":"synthetic"}"#
    );
    assert_eq!(detail.payload["actor_id"], "mcp-stdio");
    assert_eq!(detail.payload["redaction_version"], 1);
}
