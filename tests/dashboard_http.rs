use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    domain::operation_log::{ActorKind, OperationLogEntry, OperationLogKind, OperationLogStatus},
    interfaces::dashboard::{
        DashboardRuntimeInfo, EventQuery, OperationEvent, OperationKind, OperationRecorder,
        OperationStatus, start_dashboard_service, start_dashboard_service_with_operation_log,
    },
    ports::OperationLogStore,
    support::config::DashboardConfig,
};
use chrono::{Duration, Utc};
use reqwest::{Client, Method, StatusCode, header};
use serde_json::json;
use tempfile::tempdir;

fn config(base_path: &str) -> DashboardConfig {
    DashboardConfig {
        enabled: true,
        host: "127.0.0.1".to_string(),
        port: 0,
        base_path: base_path.to_string(),
        event_capacity: 20,
        sse_enabled: true,
        open_browser: false,
        required: true,
    }
}

fn runtime() -> DashboardRuntimeInfo {
    DashboardRuntimeInfo {
        service_name: "agent-llm-mm".to_string(),
        transport: "stdio".to_string(),
        provider: "mock".to_string(),
        dashboard_enabled: true,
        read_only: true,
    }
}

fn recorder_with_event() -> OperationRecorder {
    let recorder = OperationRecorder::new(20);
    recorder.append(OperationEvent {
        id: "op_1".to_string(),
        sequence: 1,
        timestamp: Utc::now(),
        kind: OperationKind::Reflection,
        status: OperationStatus::Handled,
        operation: "auto_reflect".to_string(),
        namespace: Some("self".to_string()),
        summary: "proposal passed evidence gate".to_string(),
        correlation_id: None,
        payload: json!({ "reflection_id": "reflection-1" }),
    });
    recorder
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_serves_html_summary_events_detail_and_health() {
    let recorder = recorder_with_event();
    let handle = start_dashboard_service(config("/"), recorder.clone(), runtime())
        .await
        .expect("dashboard starts");
    let base_url = handle.base_url();
    let client = Client::new();

    let html = client
        .get(format!("{base_url}/"))
        .send()
        .await
        .expect("html response")
        .text()
        .await
        .expect("html body");
    assert!(html.contains("Memory-chan Live Desk"));
    assert!(html.contains("assets/memory-chan-hero.png"));
    assert!(html.contains("assets/memory-chan-sidebar.png"));

    let hero = client
        .get(format!("{base_url}/assets/memory-chan-hero.png"))
        .send()
        .await
        .expect("hero asset response");
    assert_eq!(
        hero.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .expect("hero content type"),
        "image/png"
    );
    assert!(
        hero.bytes().await.expect("hero bytes").len() > 1000,
        "hero asset should be bundled"
    );

    let summary: serde_json::Value = client
        .get(format!("{base_url}/api/summary"))
        .send()
        .await
        .expect("summary response")
        .json()
        .await
        .expect("summary json");
    assert_eq!(summary["total_events"], 1);
    assert_eq!(summary["reflection_events"], 1);

    let events: serde_json::Value = client
        .get(format!("{base_url}/api/events?limit=5"))
        .send()
        .await
        .expect("events response")
        .json()
        .await
        .expect("events json");
    assert_eq!(events.as_array().expect("events array").len(), 1);

    let detail: serde_json::Value = client
        .get(format!("{base_url}/api/events/op_1"))
        .send()
        .await
        .expect("detail response")
        .json()
        .await
        .expect("detail json");
    assert_eq!(detail["id"], "op_1");
    assert_eq!(detail["read_only"], true);

    let health: serde_json::Value = client
        .get(format!("{base_url}/api/health"))
        .send()
        .await
        .expect("health response")
        .json()
        .await
        .expect("health json");
    assert_eq!(health["status"], "ok");
    assert_eq!(health["read_only"], true);

    let stream = client
        .get(format!("{base_url}/api/events/stream"))
        .send()
        .await
        .expect("event stream response");
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(
        stream
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("stream content type"),
        "text/event-stream"
    );

    drop(handle);
    let _ = recorder.recent(EventQuery::default());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_honors_configured_base_path() {
    let recorder = recorder_with_event();
    let handle = start_dashboard_service(config("/agent-llm-mm"), recorder, runtime())
        .await
        .expect("dashboard starts");
    let base_url = handle.base_url();

    let summary: serde_json::Value = Client::new()
        .get(format!("{base_url}/api/summary"))
        .send()
        .await
        .expect("summary response")
        .json()
        .await
        .expect("summary json");

    assert!(base_url.ends_with("/agent-llm-mm"));
    assert_eq!(summary["total_events"], 1);

    let sidebar = Client::new()
        .get(format!("{base_url}/assets/memory-chan-sidebar.png"))
        .send()
        .await
        .expect("sidebar asset response");
    assert_eq!(sidebar.status(), reqwest::StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_serves_read_only_operation_log_history() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("dashboard-history.sqlite"));
    let store = SqliteStore::bootstrap(&database_url)
        .await
        .expect("store should bootstrap");
    store
        .append_operation(OperationLogEntry {
            operation_id: "op-log-1".to_string(),
            occurred_at: Utc::now(),
            namespace: Some("self".to_string()),
            actor_kind: ActorKind::System,
            actor_id: "mcp-stdio".to_string(),
            entrypoint: "ingest_interaction".to_string(),
            operation_kind: OperationLogKind::Tool,
            status: OperationLogStatus::Ok,
            correlation_id: Some("corr-history-1".to_string()),
            request_summary_json: None,
            response_summary_json: Some(r#"{"event_id":"event-1"}"#.to_string()),
            diagnostic_summary_json: None,
            redaction_version: 1,
        })
        .await
        .expect("operation log append should succeed");

    let handle = start_dashboard_service_with_operation_log(
        config("/"),
        recorder_with_event(),
        runtime(),
        Some(store),
    )
    .await
    .expect("dashboard starts");
    let base_url = handle.base_url();
    let client = Client::new();

    let history: serde_json::Value = client
        .get(format!(
            "{base_url}/api/operation-log?correlation_id=corr-history-1&limit=5"
        ))
        .send()
        .await
        .expect("operation log response")
        .json()
        .await
        .expect("operation log json");

    let entries = history.as_array().expect("operation log array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "op-log-1");
    assert_eq!(entries[0]["operation"], "ingest_interaction");
    assert_eq!(entries[0]["kind"], "tool");
    assert_eq!(entries[0]["status"], "ok");
    assert_eq!(entries[0]["namespace"], "self");
    assert_eq!(entries[0]["correlation_id"], "corr-history-1");
    assert_eq!(entries[0]["read_only"], true);

    let detail: serde_json::Value = client
        .get(format!("{base_url}/api/operation-log/op-log-1"))
        .send()
        .await
        .expect("operation log detail response")
        .json()
        .await
        .expect("operation log detail json");
    assert_eq!(detail["id"], "op-log-1");
    assert_eq!(detail["correlation_id"], "corr-history-1");
    assert_eq!(detail["read_only"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_bounds_operation_log_history_limit() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("dashboard-history-limit.sqlite"));
    let store = SqliteStore::bootstrap(&database_url)
        .await
        .expect("store should bootstrap");
    let now = Utc::now();

    for index in 0..101 {
        store
            .append_operation(OperationLogEntry {
                operation_id: format!("op-log-{index:03}"),
                occurred_at: now + Duration::milliseconds(index),
                namespace: Some("self".to_string()),
                actor_kind: ActorKind::System,
                actor_id: "mcp-stdio".to_string(),
                entrypoint: "ingest_interaction".to_string(),
                operation_kind: OperationLogKind::Tool,
                status: OperationLogStatus::Ok,
                correlation_id: Some("corr-history-limit".to_string()),
                request_summary_json: None,
                response_summary_json: Some(r#"{"event_id":"event-1"}"#.to_string()),
                diagnostic_summary_json: None,
                redaction_version: 1,
            })
            .await
            .expect("operation log append should succeed");
    }

    let handle = start_dashboard_service_with_operation_log(
        config("/"),
        recorder_with_event(),
        runtime(),
        Some(store),
    )
    .await
    .expect("dashboard starts");
    let base_url = handle.base_url();
    let client = Client::new();

    let history: serde_json::Value = client
        .get(format!(
            "{base_url}/api/operation-log?correlation_id=corr-history-limit"
        ))
        .send()
        .await
        .expect("operation log response")
        .json()
        .await
        .expect("operation log json");

    assert_eq!(
        history.as_array().expect("operation log array").len(),
        100,
        "operation-log history should apply a default bounded limit"
    );

    let oversized = client
        .get(format!("{base_url}/api/operation-log?limit=101"))
        .send()
        .await
        .expect("oversized operation log response");
    assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_rejects_write_methods_on_read_only_routes() {
    let recorder = recorder_with_event();
    let handle = start_dashboard_service(config("/"), recorder, runtime())
        .await
        .expect("dashboard starts");
    let base_url = handle.base_url();
    let client = Client::new();

    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        for path in [
            "/",
            "/api/status",
            "/assets/app.mjs",
            "/api/summary",
            "/api/events",
            "/api/events/op_1",
            "/api/operation-log",
            "/api/operation-log/op-log-1",
            "/api/events/stream",
            "/api/health",
        ] {
            let response = client
                .request(method.clone(), format!("{base_url}{path}"))
                .json(&json!({ "attempt": "write" }))
                .send()
                .await
                .expect("write method response");
            assert_eq!(
                response.status(),
                StatusCode::METHOD_NOT_ALLOWED,
                "dashboard route {path} must reject {method} write method"
            );
        }
    }
}

fn sqlite_url(path: std::path::PathBuf) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dashboard_base_path_redirect_assets_and_instance_metadata() {
    let mut settings = config("/nested/desk/");
    settings.sse_enabled = false;
    let handle = start_dashboard_service(settings, recorder_with_event(), runtime())
        .await
        .unwrap();
    let base = handle.base_url();
    let client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let redirect = client.get(&base).send().await.unwrap();
    assert_eq!(redirect.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(redirect.headers()[header::LOCATION], "/nested/desk/");
    let html = client
        .get(format!("{base}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("type=\"module\" src=\"assets/app.mjs\""));
    for name in [
        "dashboard.css",
        "app.mjs",
        "state.mjs",
        "transport.mjs",
        "render.mjs",
        "i18n.mjs",
        "tasks.mjs",
    ] {
        let response = client
            .get(format!("{base}/assets/{name}"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{name}");
        assert!(
            response.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with(if name.ends_with("css") {
                    "text/css"
                } else {
                    "text/javascript"
                })
        );
        assert!(!response.text().await.unwrap().is_empty());
    }
    assert_eq!(
        client
            .get(format!("{base}/assets/not-bundled.mjs"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .get(format!("{base}/api/events/stream"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let status: serde_json::Value = client
        .get(format!("{base}/api/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["process_id"], std::process::id());
    assert_eq!(status["event_capacity"], 20);
    assert_eq!(status["history_available"], false);
    assert_eq!(status["sse_enabled"], false);
    assert_eq!(status["read_only"], true);
    let again: serde_json::Value = client
        .get(format!("{base}/api/status"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["instance_id"], again["instance_id"]);
    assert_eq!(status["started_at"], again["started_at"]);
    let second = start_dashboard_service(config("/"), recorder_with_event(), runtime())
        .await
        .unwrap();
    let other: serde_json::Value = client
        .get(format!("{}/api/status", second.base_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_ne!(status["instance_id"], other["instance_id"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_dashboard_never_exposes_task_records() {
    let directory = tempdir().unwrap();
    let store = SqliteStore::bootstrap(&sqlite_url(directory.path().join("tasks.sqlite")))
        .await
        .unwrap();
    let handle = start_dashboard_service_with_operation_log(
        config("/"),
        recorder_with_event(),
        runtime(),
        Some(store),
    )
    .await
    .unwrap();
    let client = Client::new();
    let status: serde_json::Value = client
        .get(format!("{}/api/status", handle.base_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["tasks_available"], false);
    let workspaces = client
        .get(format!("{}/api/task-workspaces", handle.base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(workspaces.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        workspaces.json::<serde_json::Value>().await.unwrap(),
        json!({"error":"task workspaces unavailable"})
    );
    for route in ["tasks", "tasks/anything", "tasks/anything/experience-uses"] {
        let response = client
            .get(format!(
                "{}/api/{route}?namespace=project/test",
                handle.base_url()
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap(),
            json!({"error": if route.ends_with("experience-uses") { "experience use records unavailable" } else { "task records unavailable" }})
        );
    }
}
