use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::daemon::DaemonHandle,
    domain::operation_log::{ActorKind, OperationLogEntry, OperationLogKind, OperationLogStatus},
    ports::OperationLogStore,
    support::config::{AppConfig, DaemonConfig, TransportKind},
};
use chrono::Utc;
use sqlx::sqlite::SqlitePool;
use tempfile::tempdir;
use tokio::time::{Duration, sleep, timeout};

#[test]
fn daemon_defaults_to_disabled() {
    let config = AppConfig::default();
    assert!(!config.daemon.enabled);
    assert_eq!(config.daemon.poll_interval_ms, 60_000);
    assert_eq!(config.daemon.max_concurrent_tasks, 1);
}

#[test]
fn daemon_config_rejects_zero_polling_interval() {
    let config = AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        daemon: DaemonConfig {
            enabled: true,
            poll_interval_ms: 0,
            max_concurrent_tasks: 1,
        },
        ..Default::default()
    };
    let result = config.validate();
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("poll_interval_ms"));
}

#[tokio::test]
async fn doctor_reports_daemon_config_without_starting_daemon() {
    let temp_dir = tempdir().expect("temp dir");
    let config = AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        database_url: sqlite_url(temp_dir.path().join("daemon-config-doctor.sqlite")),
        ..Default::default()
    };
    let report = agent_llm_mm::run_doctor(config).await.unwrap();
    assert!(!report.daemon_enabled);
    assert_eq!(report.daemon_poll_interval_ms, 60_000);
    assert_eq!(report.daemon_max_concurrent_tasks, 1);
}

#[tokio::test]
async fn doctor_reports_observe_only_daemon_diagnostics_without_semantic_writes() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("daemon-observe-only.sqlite"));
    let config = AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        transport: TransportKind::Stdio,
        database_url: database_url.clone(),
        daemon: DaemonConfig {
            enabled: true,
            poll_interval_ms: 250,
            max_concurrent_tasks: 1,
        },
        ..Default::default()
    };

    agent_llm_mm::adapters::sqlite::initialize_database(&database_url)
        .await
        .expect("explicit database init should pass");
    let before = semantic_counts(&database_url).await;

    let report = agent_llm_mm::run_doctor(config)
        .await
        .expect("doctor should report observe-only diagnostics");
    let after = semantic_counts(&database_url).await;

    assert!(report.daemon_enabled);
    assert_eq!(report.daemon_observe_only.mode, "observe_only");
    assert!(report.daemon_observe_only.local_only);
    assert!(!report.daemon_observe_only.write_gate_approved);
    assert!(!report.daemon_observe_only.writes_allowed);
    assert!(!report.daemon_observe_only.remote_listener_enabled);
    assert!(report.daemon_observe_only.write_blockers.contains(
        &"daemon write gate is not approved; run_reflection remains the only durable write path"
            .to_string()
    ));
    assert!(
        report
            .daemon_observe_only
            .remote_blockers
            .contains(&"remote listener is blocked until auth, authorization, audit, rollback, and tenant isolation gates exist".to_string())
    );
    assert_eq!(report.daemon_observe_only.in_flight_task_count, 0);
    assert_eq!(report.daemon_observe_only.trigger_candidates_observed, 0);
    assert_eq!(report.daemon_observe_only.read_errors, Vec::<String>::new());
    assert!(
        report
            .daemon_observe_only
            .data_sources
            .contains(&"daemon_config".to_string())
    );
    assert!(
        report
            .daemon_observe_only
            .data_sources
            .contains(&"operation_log".to_string())
    );
    assert_eq!(
        before, after,
        "observe-only daemon diagnostics must not write semantic memory tables"
    );
}

#[tokio::test]
async fn doctor_observe_only_daemon_diagnostics_count_local_operation_candidates() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("daemon-operation-candidates.sqlite"));
    let store = SqliteStore::bootstrap(&database_url).await.unwrap();
    store
        .append_operation(operation_entry(
            "op-failed-tool",
            OperationLogKind::Tool,
            OperationLogStatus::Failed,
        ))
        .await
        .unwrap();
    store
        .append_operation(operation_entry(
            "op-suppressed-trigger",
            OperationLogKind::Trigger,
            OperationLogStatus::Suppressed,
        ))
        .await
        .unwrap();
    for index in 0..30 {
        store
            .append_operation(operation_entry(
                &format!("op-ok-tool-{index}"),
                OperationLogKind::Tool,
                OperationLogStatus::Ok,
            ))
            .await
            .unwrap();
        store
            .append_operation(operation_entry(
                &format!("op-ok-trigger-{index}"),
                OperationLogKind::Trigger,
                OperationLogStatus::Ok,
            ))
            .await
            .unwrap();
    }

    let report = agent_llm_mm::run_doctor(AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        database_url,
        daemon: DaemonConfig {
            enabled: true,
            poll_interval_ms: 250,
            max_concurrent_tasks: 1,
        },
        ..Default::default()
    })
    .await
    .expect("doctor should read local operation-log diagnostics");

    assert_eq!(report.daemon_observe_only.trigger_candidates_observed, 1);
    assert_eq!(report.daemon_observe_only.trigger_candidates_suppressed, 1);
    assert_eq!(report.daemon_observe_only.cooldown_status, "observe_only");
    assert_eq!(report.daemon_observe_only.read_errors, Vec::<String>::new());
}

#[tokio::test]
async fn doctor_observe_only_daemon_diagnostics_explain_read_only_boundary() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("daemon-boundary-diagnostics.sqlite"));

    let report = agent_llm_mm::run_doctor(AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        database_url,
        daemon: DaemonConfig {
            enabled: true,
            poll_interval_ms: 250,
            max_concurrent_tasks: 1,
        },
        ..Default::default()
    })
    .await
    .expect("doctor should report observe-only diagnostics");
    let diagnostics = serde_json::to_value(&report.daemon_observe_only)
        .expect("daemon diagnostics should serialize");

    assert_eq!(diagnostics["candidate_read_data_source"], "operation_log");
    assert_eq!(
        diagnostics["candidate_read_operation_kinds"],
        serde_json::json!(["tool", "trigger"])
    );
    assert_eq!(
        diagnostics["candidate_read_statuses"],
        serde_json::json!(["failed", "suppressed"])
    );
    assert_eq!(diagnostics["candidate_read_limit_per_kind_status"], 25);
    assert_eq!(diagnostics["candidate_reads_are_read_only"], true);
    assert_eq!(
        diagnostics["suppression_diagnostics"],
        "read_only_status_count"
    );
    assert_eq!(
        diagnostics["cooldown_diagnostics"],
        "diagnostic_only_no_scheduling"
    );
    assert_eq!(
        diagnostics["clean_shutdown_status"],
        "not_started_by_doctor"
    );
    assert_eq!(
        diagnostics["lifecycle_regression_status"],
        "verified_by_handle_stop_test"
    );
    assert_eq!(diagnostics["semantic_writes_allowed"], false);
    assert_eq!(diagnostics["run_reflection_allowed_from_daemon"], false);
    assert_eq!(diagnostics["write_capable_daemon_gate_status"], "blocked");
    assert_eq!(diagnostics["background_autonomy_enabled"], false);
    assert_eq!(diagnostics["daemon_loop_connected"], false);
    assert_eq!(diagnostics["daemon_started_by_doctor"], false);
}

#[tokio::test]
async fn doctor_observe_only_candidate_reads_are_bounded_and_do_not_claim_daemon_writes() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(
        temp_dir
            .path()
            .join("daemon-bounded-candidate-reads.sqlite"),
    );
    let store = SqliteStore::bootstrap(&database_url).await.unwrap();
    for index in 0..30 {
        store
            .append_operation(operation_entry(
                &format!("op-failed-tool-{index}"),
                OperationLogKind::Tool,
                OperationLogStatus::Failed,
            ))
            .await
            .unwrap();
        store
            .append_operation(operation_entry(
                &format!("op-failed-trigger-{index}"),
                OperationLogKind::Trigger,
                OperationLogStatus::Failed,
            ))
            .await
            .unwrap();
    }

    let report = agent_llm_mm::run_doctor(AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        database_url,
        daemon: DaemonConfig {
            enabled: true,
            poll_interval_ms: 250,
            max_concurrent_tasks: 1,
        },
        ..Default::default()
    })
    .await
    .expect("doctor should report bounded candidate reads");
    let diagnostics = serde_json::to_value(&report.daemon_observe_only)
        .expect("daemon diagnostics should serialize");

    assert_eq!(report.daemon_observe_only.trigger_candidates_observed, 50);
    assert_eq!(diagnostics["candidate_read_limit_per_kind_status"], 25);
    assert_eq!(diagnostics["semantic_writes_allowed"], false);
    assert_eq!(diagnostics["background_autonomy_enabled"], false);
    assert_eq!(diagnostics["daemon_loop_connected"], false);
    assert_eq!(diagnostics["write_capable_daemon_gate_status"], "blocked");
    assert!(
        diagnostics["write_blockers"]
            .as_array()
            .expect("write blockers should be an array")
            .iter()
            .any(|blocker| blocker
                .as_str()
                .is_some_and(|text| text.contains("write-capable daemon gate is blocked"))),
        "observe-only diagnostics should name the blocked write-capable daemon gate"
    );
}

#[tokio::test]
async fn disabled_daemon_handle_exits_without_running_lifecycle_loop() {
    let handle = DaemonHandle::start(DaemonConfig {
        enabled: false,
        poll_interval_ms: 10,
        max_concurrent_tasks: 1,
    });

    assert!(!handle.config_enabled());
    assert_eq!(handle.mode(), "disabled");
    assert_eq!(handle.poll_interval_ms(), 10);
    timeout(Duration::from_millis(250), handle.stop())
        .await
        .expect("disabled daemon should stop promptly");
}

#[tokio::test]
async fn observe_only_daemon_handle_starts_and_stops_without_write_capability() {
    let handle = DaemonHandle::start(DaemonConfig {
        enabled: true,
        poll_interval_ms: 10,
        max_concurrent_tasks: 1,
    });

    assert!(handle.config_enabled());
    assert_eq!(handle.mode(), "observe_only");
    assert!(!handle.writes_allowed());
    assert!(!handle.remote_listener_enabled());
    assert_eq!(handle.poll_interval_ms(), 10);
    timeout(Duration::from_secs(1), handle.stop())
        .await
        .expect("observe-only daemon should stop promptly");
}

#[tokio::test]
async fn dropping_observe_only_daemon_handle_aborts_lifecycle_loop() {
    let handle = DaemonHandle::start(DaemonConfig {
        enabled: true,
        poll_interval_ms: 60_000,
        max_concurrent_tasks: 1,
    });
    let probe = handle.lifecycle_probe();

    timeout(Duration::from_secs(1), async {
        while !probe.is_running() {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("observe-only daemon should enter lifecycle loop");

    drop(handle);

    timeout(Duration::from_secs(1), async {
        while probe.is_running() {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("dropping handle should abort the observe-only lifecycle loop");
}

#[test]
fn daemon_lifecycle_remains_observe_only_and_has_no_durable_write_or_remote_paths() {
    let source = std::fs::read_to_string("src/application/daemon.rs")
        .expect("daemon source should be readable");

    assert!(source.contains("mode"));
    assert!(source.contains("observe_only"));
    assert!(source.contains("writes_allowed"));
    assert!(source.contains("remote_listener_enabled"));
    assert!(source.contains("impl Drop for DaemonHandle"));
    assert!(source.contains("task.abort()"));
    assert!(!source.contains("run_reflection"));
    assert!(!source.contains("append_event"));
    assert!(!source.contains("append_claim"));
    assert!(!source.contains("append_reflection"));
    assert!(!source.contains("TcpListener"));
    assert!(!source.contains("start_dashboard_service"));
}

#[test]
fn stdio_server_wires_daemon_only_through_observe_only_handle() {
    let source = std::fs::read_to_string("src/interfaces/mcp/server.rs")
        .expect("stdio server source should be readable");

    assert!(source.contains("start_configured_daemon(&config)"));
    assert!(source.contains("DaemonHandle::start(config.daemon.clone())"));
    assert!(source.contains("handle.stop().await"));
    assert!(!source.contains("run_reflection_allowed_from_daemon: true"));
}

fn sqlite_url(path: impl AsRef<std::path::Path>) -> String {
    format!(
        "sqlite://{}",
        path.as_ref().to_string_lossy().replace('\\', "/")
    )
}

async fn semantic_counts(database_url: &str) -> (i64, i64, i64, i64, i64) {
    let pool = SqlitePool::connect(database_url).await.unwrap();
    let events = table_count(&pool, "events").await;
    let claims = table_count(&pool, "claims").await;
    let reflections = table_count(&pool, "reflections").await;
    let identity_claims = table_count(&pool, "identity_claims").await;
    let commitments = table_count(&pool, "commitments").await;
    (events, claims, reflections, identity_claims, commitments)
}

async fn table_count(pool: &SqlitePool, table: &str) -> i64 {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    sqlx::query_scalar::<_, i64>(&sql)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn operation_entry(
    operation_id: &str,
    operation_kind: OperationLogKind,
    status: OperationLogStatus,
) -> OperationLogEntry {
    OperationLogEntry {
        operation_id: operation_id.to_string(),
        occurred_at: Utc::now(),
        namespace: Some("self".to_string()),
        actor_kind: ActorKind::System,
        actor_id: "daemon-observe-only-test".to_string(),
        entrypoint: operation_id.to_string(),
        operation_kind,
        status,
        correlation_id: Some(format!("corr-{operation_id}")),
        request_summary_json: None,
        response_summary_json: None,
        diagnostic_summary_json: None,
        redaction_version: 1,
    }
}
