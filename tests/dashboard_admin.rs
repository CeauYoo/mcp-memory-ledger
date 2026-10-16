use std::time::Duration;

use agent_llm_mm::{
    interfaces::dashboard::{
        AdminGate, DashboardObserver, DashboardRuntimeInfo, OperationRecorder,
        start_admin_dashboard_service,
    },
    support::config::DashboardConfig,
};
use reqwest::{Client, Method, StatusCode, header};
use serde_json::{Value, json};

// Synthetic test capability only; never generated from user configuration or environment.
const CAPABILITY: &str = "synthetic-dashboard-owner-capability-for-isolated-tests";

fn config(base_path: &str) -> DashboardConfig {
    DashboardConfig {
        enabled: true,
        host: "127.0.0.1".into(),
        port: 0,
        base_path: base_path.into(),
        event_capacity: 32,
        sse_enabled: true,
        open_browser: false,
        required: true,
    }
}

fn runtime() -> DashboardRuntimeInfo {
    DashboardRuntimeInfo {
        service_name: "isolated-core".into(),
        transport: "stdio".into(),
        provider: "mock".into(),
        dashboard_enabled: true,
        read_only: true,
    }
}

fn observer() -> DashboardObserver {
    DashboardObserver::enabled(OperationRecorder::new(32))
}

fn client() -> Client {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

fn record(observer: &DashboardObserver, namespace: &str) {
    observer.record_tool_ok(
        "recall_memory",
        Some(namespace.into()),
        None,
        "synthetic read completed".into(),
        &json!({ "result_count": 0 }),
    );
}

#[test]
fn gate_rejects_invalid_capabilities_and_never_formats_the_secret() {
    for invalid in [
        "",
        " ",
        "Bearer value",
        "a\nb",
        "a\tb",
        "a=b",
        "=",
        "非ASCII",
    ] {
        assert!(AdminGate::new(invalid).is_err());
    }
    let gate = AdminGate::new(CAPABILITY).unwrap();
    assert_eq!(format!("{gate:?}"), "AdminGate { capability: [REDACTED] }");
    assert!(AdminGate::new("synthetic-._~+/=").is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_data_free_shell_is_public_and_every_api_requires_authorization() {
    let client = client();
    for base_path in ["/", "/private/desk"] {
        let handle = start_admin_dashboard_service(
            config(base_path),
            runtime(),
            observer(),
            None,
            AdminGate::new(CAPABILITY).unwrap(),
        )
        .await
        .unwrap();
        let base = handle.base_url();
        for suffix in [
            "",
            "/",
            "/assets/memory-chan-hero.png",
            "/assets/memory-chan-sidebar.png",
            "/assets/dashboard.css",
            "/assets/app.mjs",
            "/assets/render.mjs",
            "/assets/transport.mjs",
            "/assets/state.mjs",
            "/assets/i18n.mjs",
            "/assets/tasks.mjs",
            "/api/status",
            "/api/summary",
            "/api/events",
            "/api/events/missing",
            "/api/events/stream",
            "/api/operation-log",
            "/api/operation-log/missing",
            "/api/health",
            "/api/task-workspaces",
            "/api/tasks?namespace=project/test",
            "/api/tasks/missing?namespace=project/test",
            "/api/tasks/missing/experience-uses?namespace=project/test",
            "/not-a-route",
        ] {
            for credential in [None, Some("Bearer synthetic-wrong-capability")] {
                let mut request = client.get(format!("{base}{suffix}"));
                if let Some(value) = credential {
                    request = request.header(header::AUTHORIZATION, value);
                }
                let response = request.send().await.unwrap();
                if credential.is_none()
                    && (suffix.is_empty() || suffix == "/" || suffix.starts_with("/assets/"))
                {
                    assert!(
                        response.status().is_success() || response.status().is_redirection(),
                        "{suffix}"
                    );
                    let body = response.bytes().await.unwrap();
                    assert!(!String::from_utf8_lossy(&body).contains(CAPABILITY));
                    continue;
                }
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{suffix}");
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                assert_eq!(
                    response.headers()[header::WWW_AUTHENTICATE],
                    "Bearer realm=\"memory-ledger-admin\""
                );
                assert_eq!(
                    response.json::<Value>().await.unwrap(),
                    json!({ "error": "admin authorization required" })
                );
            }
        }
        // The outer gate also protects the fallback outside a configured mount.
        let origin = reqwest::Url::parse(&base)
            .unwrap()
            .join("/unmounted")
            .unwrap();
        assert_eq!(
            client.get(origin).send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_one_well_formed_authorization_header_is_accepted() {
    let handle = start_admin_dashboard_service(
        config("/"),
        runtime(),
        observer(),
        None,
        AdminGate::new(CAPABILITY).unwrap(),
    )
    .await
    .unwrap();
    let url = format!("{}/api/health", handle.base_url());
    let client = client();
    for value in [
        String::new(),
        format!("Basic {CAPABILITY}"),
        format!("Bearer  {CAPABILITY}"),
        format!("Bearer {CAPABILITY}, Bearer {CAPABILITY}"),
    ] {
        assert_eq!(
            client
                .get(&url)
                .header(header::AUTHORIZATION, value)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let mut duplicate = header::HeaderMap::new();
    duplicate.append(
        header::AUTHORIZATION,
        format!("Bearer {CAPABILITY}").parse().unwrap(),
    );
    duplicate.append(
        header::AUTHORIZATION,
        "Bearer synthetic-wrong-capability".parse().unwrap(),
    );
    assert_eq!(
        client
            .get(&url)
            .headers(duplicate)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // Attempted query/cookie credentials are ignored; only the header is authority.
    assert_eq!(
        client
            .get(&url)
            .query(&[("access_token", CAPABILITY)])
            .header(header::COOKIE, format!("admin_capability={CAPABILITY}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .get(&url)
            .header(header::AUTHORIZATION, format!("bEaReR {CAPABILITY}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_requests_preserve_the_existing_read_only_routes() {
    let handle = start_admin_dashboard_service(
        config("/admin"),
        runtime(),
        observer(),
        None,
        AdminGate::new(CAPABILITY).unwrap(),
    )
    .await
    .unwrap();
    let client = client();
    let base = handle.base_url();
    for (suffix, expected) in [
        ("", StatusCode::PERMANENT_REDIRECT),
        ("/", StatusCode::OK),
        ("/assets/memory-chan-hero.png", StatusCode::OK),
        ("/assets/memory-chan-sidebar.png", StatusCode::OK),
        ("/assets/dashboard.css", StatusCode::OK),
        ("/assets/app.mjs", StatusCode::OK),
        ("/api/status", StatusCode::OK),
        ("/api/summary", StatusCode::OK),
        ("/api/events", StatusCode::OK),
        ("/api/events/missing", StatusCode::NOT_FOUND),
        ("/api/operation-log", StatusCode::SERVICE_UNAVAILABLE),
        (
            "/api/operation-log/missing",
            StatusCode::SERVICE_UNAVAILABLE,
        ),
        ("/api/health", StatusCode::OK),
    ] {
        let response = client
            .get(format!("{base}{suffix}"))
            .bearer_auth(CAPABILITY)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), expected, "{suffix}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!response.text().await.unwrap().contains(CAPABILITY));
    }
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        for suffix in [
            "/",
            "/api/events",
            "/api/events/stream",
            "/api/operation-log",
        ] {
            let url = format!("{base}{suffix}");
            assert_eq!(
                client
                    .request(method.clone(), &url)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                client
                    .request(method.clone(), &url)
                    .bearer_auth(CAPABILITY)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_reads_the_cores_shared_recorder_without_resetting_sequence() {
    let core = observer();
    record(&core, "project/before-http");
    let handle = start_admin_dashboard_service(
        config("/"),
        runtime(),
        core.clone(),
        None,
        AdminGate::new(CAPABILITY).unwrap(),
    )
    .await
    .unwrap();
    let (first, second) = (core.clone(), core.clone());
    let first = tokio::spawn(async move { record(&first, "project/client-one") });
    let second = tokio::spawn(async move { record(&second, "project/client-two") });
    first.await.unwrap();
    second.await.unwrap();
    let base = handle.base_url();
    let events: Vec<Value> = client()
        .get(format!("{base}/api/events"))
        .bearer_auth(CAPABILITY)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut sequences: Vec<_> = events
        .iter()
        .map(|event| event["sequence"].as_u64().unwrap())
        .collect();
    sequences.sort_unstable();
    assert_eq!(sequences, [1, 2, 3]);
    for namespace in [
        "project/before-http",
        "project/client-one",
        "project/client-two",
    ] {
        assert!(events.iter().any(|event| event["namespace"] == namespace));
    }
    let summary: Value = client()
        .get(format!("{base}/api/summary"))
        .bearer_auth(CAPABILITY)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summary["total_events"], 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_sse_observes_the_same_core_events() {
    let core = observer();
    let handle = start_admin_dashboard_service(
        config("/"),
        runtime(),
        core.clone(),
        None,
        AdminGate::new(CAPABILITY).unwrap(),
    )
    .await
    .unwrap();
    let mut stream = client()
        .get(format!("{}/api/events/stream", handle.base_url()))
        .bearer_auth(CAPABILITY)
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    assert_eq!(stream.headers()[header::CONTENT_TYPE], "text/event-stream");
    record(&core, "project/sse-fixture");
    let event = tokio::time::timeout(Duration::from_secs(2), stream.chunk())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let event = String::from_utf8(event.to_vec()).unwrap();
    assert!(event.contains("project/sse-fixture"));
    assert!(!event.contains(CAPABILITY));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_is_explicit_loopback_only_and_requires_an_existing_observer() {
    let mut disabled = config("/");
    disabled.enabled = false;
    assert!(
        start_admin_dashboard_service(
            disabled,
            runtime(),
            observer(),
            None,
            AdminGate::new(CAPABILITY).unwrap()
        )
        .await
        .is_err()
    );
    for host in ["0.0.0.0", "::", "192.0.2.1"] {
        let mut remote = config("/");
        remote.host = host.into();
        assert!(
            start_admin_dashboard_service(
                remote,
                runtime(),
                observer(),
                None,
                AdminGate::new(CAPABILITY).unwrap()
            )
            .await
            .is_err()
        );
    }
    assert!(
        start_admin_dashboard_service(
            config("/"),
            runtime(),
            DashboardObserver::disabled(),
            None,
            AdminGate::new(CAPABILITY).unwrap()
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn admin_rejects_foreign_host_origin_and_browser_cross_site_requests() {
    let handle = start_admin_dashboard_service(
        config("/"),
        runtime(),
        observer(),
        None,
        AdminGate::new(CAPABILITY).unwrap(),
    )
    .await
    .unwrap();
    let client = client();
    for (name, value) in [
        ("host", "attacker.example"),
        ("origin", "https://attacker.example"),
        ("origin", "null"),
        ("sec-fetch-site", "cross-site"),
        ("sec-fetch-site", "same-site"),
    ] {
        for path in ["/", "/api/events", "/api/events/stream"] {
            let response = client
                .get(format!("{}{path}", handle.base_url()))
                .bearer_auth(CAPABILITY)
                .header(name, value)
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{name}: {path}");
            assert!(
                response
                    .headers()
                    .get("access-control-allow-origin")
                    .is_none()
            );
        }
    }
    let response = client
        .get(format!("{}/api/health", handle.base_url()))
        .bearer_auth(CAPABILITY)
        .header("origin", handle.base_url())
        .header("sec-fetch-site", "same-origin")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-frame-options"], "DENY");
    assert!(
        response.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_task_reads_are_scoped_read_only_and_preserve_checkpoint_version() {
    use agent_llm_mm::{adapters::sqlite::SqliteStore, ports::task_store::TaskStore};
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite://{}",
        directory.path().join("owner-tasks.sqlite").display()
    );
    let store = SqliteStore::bootstrap(&database_url).await.unwrap();
    let handle = start_admin_dashboard_service(
        config("/tasks/desk"),
        runtime(),
        observer(),
        Some(store.clone()),
        AdminGate::new(CAPABILITY).unwrap(),
    )
    .await
    .unwrap();
    let client = client();
    let base = handle.base_url();
    let get = |route: &str| {
        client
            .get(format!("{base}/api/{route}"))
            .bearer_auth(CAPABILITY)
    };
    let empty: Value = get("tasks?namespace=project/test")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(empty, json!({"tasks":[],"limit":100,"read_only":true}));
    let workspaces: Value = get("task-workspaces")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        workspaces,
        json!({"namespaces":[],"limit":100,"has_more":false,"read_only":true})
    );
    let source = json!({"kind":"caller_reported","reference":null});
    store.create_task(serde_json::from_value(json!({
        "namespace":"project/test","task_id":"task-1","request_id":"create-1","title":"Synthetic task",
        "requirements":[
            {"requirement_id":"goal","kind":"goal","text":"Keep original goal","state":"reported","source":source,"replaces":null},
            {"requirement_id":"candidate","kind":"requirement","text":"Unconfirmed addition","state":"candidate","source":{"kind":"model_asserted"},"replaces":null}
        ],"source":source,"reason":"Synthetic owner API test"
    })).unwrap()).await.unwrap();
    use agent_llm_mm::{
        application::{experience, experience_use},
        domain::{
            event::Event,
            types::{EventKind, Namespace, Owner},
        },
        ports::{EventStore, StoredEvent},
    };
    store
        .append_event(StoredEvent::new(
            "use-evidence".into(),
            chrono::Utc::now(),
            Event::new_with_namespace(
                Owner::World,
                Namespace::parse("project/test").unwrap(),
                EventKind::Observation,
                "Synthetic result",
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    experience::create_episode(&store,serde_json::from_value(json!({
        "namespace":"project/test","request_id":"use-ep","episode_id":"use-ep",
        "content":{"title":"Synthetic episode","objective":"Check","actions":["Run"],"observations":["Result"],"outcome":"Reported pass","lesson":"Check","limitations":["Synthetic only"],"source_event_refs":["event:use-evidence"]}
    })).unwrap()).await.unwrap();
    experience::create_candidate(&store,serde_json::from_value(json!({
        "namespace":"project/test","request_id":"use-candidate","candidate_id":"use-candidate",
        "content":{"kind":"semantic","title":"Synthetic experience","statement":"Check","steps":[],"limitations":["Synthetic only"],"source_episode_ids":["use-ep"]}
    })).unwrap()).await.unwrap();
    experience::update_candidate_status(&store,serde_json::from_value(json!({
        "namespace":"project/test","request_id":"activate-use","candidate_id":"use-candidate","expected_version":1,"status":"active"
    })).unwrap()).await.unwrap();
    experience_use::record_experience_use(&store,serde_json::from_value(json!({
        "namespace":"project/test","request_id":"report-use","use_id":"use-1","task_id":"task-1","task_version":1,"expected_task_version":1,
        "candidate_id":"use-candidate","candidate_version":2,"expected_candidate_version":2,"decision":"adopted","reason":"Synthetic use",
        "conditions":[{"condition":"Fixture","assessment":"caller_reported_satisfied"}],"action_event_refs":["event:use-evidence"],"outcome_event_refs":["event:use-evidence"],
        "verification":"passed","evaluation_method":"Synthetic observation","limitations":["No independent verification"]
    })).unwrap()).await.unwrap();
    let uses: Value = get("tasks/task-1/experience-uses?namespace=project/test")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(uses["read_only"], true);
    assert_eq!(uses["uses"][0]["currently_reusable"], true);
    assert_eq!(
        uses["uses"][0]["record"]["observation"]["outcome_event_refs"],
        json!(["event:use-evidence"])
    );
    assert!(
        uses["uses"][0]["record"]["observation"]
            .get("request_id")
            .is_none()
    );
    store.record_task_checkpoint(serde_json::from_value(json!({
        "namespace":"project/test","task_id":"task-1","request_id":"checkpoint-1","checkpoint_id":"cp-1",
        "expected_version":1,"expected_checkpoint_sequence":0,
        "steps":[{"step_id":"planned","text":"Plan only","mode":"Draft","verification":"not_performed","evidence_refs":[]}],
        "blockers":["Awaiting result"],"next_steps":["Verify output"],"artifacts":[],"source":source
    })).unwrap()).await.unwrap();
    store.revise_task(serde_json::from_value(json!({
        "namespace":"project/test","task_id":"task-1","request_id":"pause-1","expected_version":1,
        "changes":[],"status":"paused","source":source,"reason":"Explicit pause"
    })).unwrap()).await.unwrap();
    let uses: Value = get("tasks/task-1/experience-uses?namespace=project/test")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(uses["uses"][0]["currently_reusable"], false);
    assert_eq!(
        uses["uses"][0]["record"]["observation"]["decision"],
        "adopted"
    );
    assert!(
        uses["uses"][0]["invalidation_reasons"]
            .as_array()
            .unwrap()
            .contains(&json!("task_revision_no_longer_current_active"))
    );
    let isolated: Value = get("tasks/task-1/experience-uses?namespace=project/other")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(isolated["uses"], json!([]));
    assert_eq!(
        get("tasks/task-1/experience-uses?namespace=self")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let workspaces: Value = get("task-workspaces")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        workspaces,
        json!({"namespaces":["project/test"],"limit":100,"has_more":false,"read_only":true})
    );
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        assert_eq!(
            client
                .request(method, format!("{base}/api/task-workspaces"))
                .bearer_auth(CAPABILITY)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
    let list: Value = get("tasks?namespace=project/test")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["tasks"][0]["status"], "paused");
    assert_eq!(list["tasks"][0]["version"], 2);
    let snapshot: Value = get("tasks/task-1?namespace=project/test")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snapshot["read_only"], true);
    assert_eq!(snapshot["snapshot"]["task"]["version"], 2);
    assert_eq!(snapshot["snapshot"]["checkpoint"]["task_version"], 1);
    assert_eq!(
        snapshot["snapshot"]["checkpoint"]["steps"][0]["verification"],
        "not_performed"
    );
    let other: Value = get("tasks?namespace=project/other")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(other["tasks"], json!([]));
    assert_eq!(
        get("tasks/task-1?namespace=project/other")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    for route in ["tasks?namespace=self", "tasks/task-1?namespace=self"] {
        let response = get(route).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<Value>().await.unwrap(),
            json!({"error":"invalid task query"})
        );
    }
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        for route in ["tasks", "tasks/task-1", "tasks/task-1/experience-uses"] {
            assert_eq!(
                client
                    .request(
                        method.clone(),
                        format!("{base}/api/{route}?namespace=project/test")
                    )
                    .bearer_auth(CAPABILITY)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
        }
    }
    let status: Value = get("status").send().await.unwrap().json().await.unwrap();
    assert_eq!(status["tasks_available"], true);
    // A damaged/older task schema is unavailable, never a fabricated empty task list.
    let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();
    sqlx::query("DROP TABLE task_requirements")
        .execute(&pool)
        .await
        .unwrap();
    for route in [
        "tasks?namespace=project/test",
        "tasks/task-1?namespace=project/test",
    ] {
        let response = get(route).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.json::<Value>().await.unwrap(),
            json!({"error":"task records unavailable"})
        );
    }
    sqlx::query("ALTER TABLE experience_uses RENAME TO unavailable_experience_uses")
        .execute(&pool)
        .await
        .unwrap();
    let unavailable = get("tasks/task-1/experience-uses?namespace=project/test")
        .send()
        .await
        .unwrap();
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        unavailable.json::<Value>().await.unwrap(),
        json!({"error":"experience use records unavailable"})
    );
    pool.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_task_workspace_discovery_is_bounded_and_reports_missing_schema() {
    use agent_llm_mm::{adapters::sqlite::SqliteStore, ports::task_store::TaskStore};
    let directory = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}",
        directory.path().join("workspace-list.sqlite").display()
    );
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    for index in 0..102 {
        store.create_task(serde_json::from_value(json!({
            "namespace":format!("project/{index:03}"),"task_id":"task","request_id":format!("create-{index}"),"title":"Synthetic task",
            "requirements":[{"requirement_id":"goal","kind":"goal","text":"Synthetic goal","state":"reported","source":{"kind":"caller_reported"}}],
            "source":{"kind":"caller_reported"},"reason":"Bounded discovery fixture"
        })).unwrap()).await.unwrap();
    }
    let handle = start_admin_dashboard_service(
        config("/"),
        runtime(),
        observer(),
        Some(store),
        AdminGate::new(CAPABILITY).unwrap(),
    )
    .await
    .unwrap();
    let client = client();
    let endpoint = format!("{}/api/task-workspaces", handle.base_url());
    let list: Value = client
        .get(&endpoint)
        .bearer_auth(CAPABILITY)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list["namespaces"].as_array().unwrap().len(), 100);
    assert_eq!(list["namespaces"][0], "project/000");
    assert_eq!(list["namespaces"][99], "project/099");
    assert_eq!(list["has_more"], true);
    assert_eq!(list.as_object().unwrap().len(), 4);
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query("ALTER TABLE task_heads RENAME TO unavailable_task_heads")
        .execute(&pool)
        .await
        .unwrap();
    let error = client
        .get(&endpoint)
        .bearer_auth(CAPABILITY)
        .send()
        .await
        .unwrap();
    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        error.json::<Value>().await.unwrap(),
        json!({"error":"task workspaces unavailable"})
    );
    pool.close().await;
}
