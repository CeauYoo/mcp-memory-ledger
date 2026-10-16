use std::{convert::Infallible, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{OriginalUri, Path, Query, State},
    http::{StatusCode, header},
    middleware,
    response::{
        Html, IntoResponse, Redirect,
        sse::{Event, KeepAlive, Sse},
    },
    routing::get,
};
use serde::Deserialize;
use serde_json::json;
use tokio::net::TcpListener;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};

use crate::{
    adapters::sqlite::SqliteStore,
    application::experience_use::{ListExperienceUsesRequest, list_experience_uses},
    application::task::{get_task_snapshot, list_tasks},
    domain::task::ListTasksRequest,
    error::AppError,
    ports::{OperationLogQuery, OperationLogStore},
    support::config::DashboardConfig,
};

use super::{
    DashboardObserver, DashboardRuntimeInfo, EventQuery, OperationKind, OperationRecorder,
    OperationStatus,
    admin::{AdminGate, require_admin},
    assets::{DASHBOARD_HTML, MEMORY_CHAN_HERO_PNG, MEMORY_CHAN_SIDEBAR_PNG},
    build_summary, project_event_detail, project_from_log_entry,
};

const OPERATION_LOG_HISTORY_DEFAULT_LIMIT: usize = 100;
const OPERATION_LOG_HISTORY_MAX_LIMIT: usize = 100;

#[derive(Clone)]
struct DashboardState {
    recorder: OperationRecorder,
    runtime: DashboardRuntimeInfo,
    operation_log: Option<SqliteStore>,
    task_read_enabled: bool,
    instance_id: String,
    started_at: chrono::DateTime<chrono::Utc>,
    sse_enabled: bool,
}

pub struct DashboardHandle {
    address: SocketAddr,
    base_path: String,
    task: tokio::task::JoinHandle<()>,
}

impl DashboardHandle {
    pub fn base_url(&self) -> String {
        if self.base_path == "/" {
            format!("http://{}", self.address)
        } else {
            format!("http://{}{}", self.address, self.base_path)
        }
    }
}

impl Drop for DashboardHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    limit: Option<usize>,
    kind: Option<OperationKind>,
    status: Option<OperationStatus>,
    namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OperationLogHttpQuery {
    limit: Option<usize>,
    kind: Option<String>,
    correlation_id: Option<String>,
    namespace: Option<String>,
}

pub async fn start_dashboard_service(
    config: DashboardConfig,
    recorder: OperationRecorder,
    runtime: DashboardRuntimeInfo,
) -> Result<DashboardHandle> {
    start_dashboard_service_with_operation_log(config, recorder, runtime, None).await
}

pub async fn start_dashboard_service_with_operation_log(
    config: DashboardConfig,
    recorder: OperationRecorder,
    runtime: DashboardRuntimeInfo,
    operation_log: Option<SqliteStore>,
) -> Result<DashboardHandle> {
    start_service(config, recorder, runtime, operation_log, None).await
}

/// Opt-in administrator surface for an already-running core. This does not create
/// an observer, reset its sequence, issue or persist a credential. The browser keeps a supplied capability in tab memory.
pub async fn start_admin_dashboard_service(
    config: DashboardConfig,
    runtime: DashboardRuntimeInfo,
    observer: DashboardObserver,
    operation_log: Option<SqliteStore>,
    gate: AdminGate,
) -> Result<DashboardHandle> {
    anyhow::ensure!(config.enabled, "admin dashboard must be explicitly enabled");
    let recorder = observer
        .recorder()
        .ok_or_else(|| anyhow::anyhow!("admin dashboard requires the core's enabled observer"))?;
    start_service(config, recorder, runtime, operation_log, Some(gate)).await
}

async fn start_service(
    config: DashboardConfig,
    recorder: OperationRecorder,
    runtime: DashboardRuntimeInfo,
    operation_log: Option<SqliteStore>,
    gate: Option<AdminGate>,
) -> Result<DashboardHandle> {
    config.validate().map_err(anyhow::Error::msg)?;
    let listener = if gate.is_some() {
        // Validate resolution before binding too: even a modified localhost mapping
        // must never make the administrator listener reachable off this machine.
        let addresses: Vec<_> = tokio::net::lookup_host((config.host.as_str(), config.port))
            .await?
            .collect();
        anyhow::ensure!(
            !addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback()),
            "admin dashboard must bind only loopback addresses"
        );
        TcpListener::bind(addresses.as_slice()).await?
    } else {
        TcpListener::bind((config.host.as_str(), config.port)).await?
    };
    let address = listener.local_addr()?;
    let base_path = normalized_base_path(&config.base_path);
    let state = DashboardState {
        recorder,
        runtime,
        task_read_enabled: gate.is_some() && operation_log.is_some(),
        operation_log,
        instance_id: uuid::Uuid::new_v4().to_string(),
        started_at: chrono::Utc::now(),
        sse_enabled: config.sse_enabled,
    };
    let app = router(state, &base_path, config.sse_enabled);
    // Layer the complete router, including aliases, redirects and fallback paths.
    let app = match gate {
        Some(gate) => app.layer(middleware::from_fn_with_state(
            gate.bind(address, &base_path),
            require_admin,
        )),
        None => app,
    };
    let task = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::warn!(error = %error, "dashboard service stopped with error");
        }
    });

    Ok(DashboardHandle {
        address,
        base_path,
        task,
    })
}

fn router(state: DashboardState, base_path: &str, sse_enabled: bool) -> Router {
    let mut routes = Router::new()
        .route("/", get(index))
        .route("/assets/memory-chan-hero.png", get(memory_chan_hero))
        .route("/assets/memory-chan-sidebar.png", get(memory_chan_sidebar))
        .route("/assets/{name}", get(frontend_asset))
        .route("/api/status", get(status))
        .route("/api/summary", get(summary))
        .route("/api/events", get(events))
        .route("/api/events/{id}", get(event_detail))
        .route("/api/operation-log", get(operation_log_history))
        .route("/api/operation-log/{id}", get(operation_log_detail))
        .route("/api/health", get(health))
        .route("/api/task-workspaces", get(task_workspaces))
        .route("/api/tasks", get(task_list))
        .route("/api/tasks/{id}", get(task_snapshot))
        .route("/api/tasks/{id}/experience-uses", get(task_experience_uses));

    if sse_enabled {
        routes = routes.route("/api/events/stream", get(event_stream));
    }

    let routes = routes.with_state(Arc::new(state));
    if base_path == "/" {
        routes
    } else {
        Router::new()
            .nest(base_path, routes)
            .route(&format!("{base_path}/"), get(index))
    }
}

fn normalized_base_path(base_path: &str) -> String {
    let trimmed = base_path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

async fn index(OriginalUri(uri): OriginalUri) -> axum::response::Response {
    if uri.path().ends_with('/') {
        Html(DASHBOARD_HTML).into_response()
    } else {
        Redirect::permanent(&format!("{}/", uri.path())).into_response()
    }
}

async fn frontend_asset(Path(name): Path<String>) -> impl IntoResponse {
    match super::assets::frontend_asset(&name) {
        Some((content_type, body)) => {
            ([(header::CONTENT_TYPE, content_type)], body).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn status(State(state): State<Arc<DashboardState>>) -> Json<serde_json::Value> {
    Json(json!({
        "instance_id": state.instance_id,
        "process_id": std::process::id(),
        "started_at": state.started_at,
        "server_time": chrono::Utc::now(),
        "event_capacity": state.recorder.capacity(),
        "history_available": state.operation_log.is_some(),
        "tasks_available": state.task_read_enabled,
        "sse_enabled": state.sse_enabled,
        "poll_interval_ms": 2500,
        "read_only": true,
    }))
}

async fn memory_chan_hero() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], MEMORY_CHAN_HERO_PNG)
}

async fn memory_chan_sidebar() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/png")],
        MEMORY_CHAN_SIDEBAR_PNG,
    )
}

async fn summary(State(state): State<Arc<DashboardState>>) -> Json<super::DashboardSummary> {
    let events = state.recorder.recent(EventQuery::default());
    Json(build_summary(&events, &state.runtime))
}

async fn events(
    State(state): State<Arc<DashboardState>>,
    Query(query): Query<EventsQuery>,
) -> Json<Vec<super::OperationEvent>> {
    Json(state.recorder.recent(EventQuery {
        limit: query.limit,
        kind: query.kind,
        status: query.status,
        namespace: query.namespace,
    }))
}

async fn event_detail(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let events = state.recorder.recent(EventQuery::default());
    match events.iter().find(|event| event.id == id) {
        Some(event) => Json(project_event_detail(event)).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "operation event not found" })),
        )
            .into_response(),
    }
}

async fn operation_log_history(
    State(state): State<Arc<DashboardState>>,
    Query(query): Query<OperationLogHttpQuery>,
) -> impl IntoResponse {
    let Some(store) = &state.operation_log else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "operation log history is not configured" })),
        )
            .into_response();
    };

    let limit = match bounded_operation_log_limit(query.limit) {
        Ok(limit) => limit,
        Err(error) => {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": error }))).into_response();
        }
    };

    match store
        .query_operations(OperationLogQuery {
            namespace: query.namespace,
            operation_kind: query.kind,
            correlation_id: query.correlation_id,
            limit: Some(limit),
            ..Default::default()
        })
        .await
    {
        Ok(entries) => Json(
            entries
                .iter()
                .map(project_from_log_entry)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

fn bounded_operation_log_limit(limit: Option<usize>) -> Result<usize, String> {
    let limit = limit.unwrap_or(OPERATION_LOG_HISTORY_DEFAULT_LIMIT);
    if limit > OPERATION_LOG_HISTORY_MAX_LIMIT {
        return Err(format!(
            "operation log history limit must be <= {OPERATION_LOG_HISTORY_MAX_LIMIT}"
        ));
    }
    Ok(limit)
}

async fn operation_log_detail(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let Some(store) = &state.operation_log else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "operation log history is not configured" })),
        )
            .into_response();
    };

    match store
        .query_operations(OperationLogQuery {
            operation_id: Some(id),
            limit: Some(1),
            ..Default::default()
        })
        .await
    {
        Ok(entries) => match entries.first() {
            Some(entry) => Json(project_from_log_entry(entry)).into_response(),
            None => (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "operation log entry not found" })),
            )
                .into_response(),
        },
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error.to_string() })),
        )
            .into_response(),
    }
}

async fn event_stream(
    State(state): State<Arc<DashboardState>>,
) -> Sse<impl Stream<Item = std::result::Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(state.recorder.subscribe()).filter_map(|event| match event {
        Ok(event) => Some(Ok(Event::default().json_data(event).unwrap_or_else(|_| {
            Event::default().data(r#"{"error":"failed to serialize dashboard event"}"#)
        }))),
        Err(_) => None,
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "status": "ok", "read_only": true }))
}

// Task requirements are owner data, never exposed through the legacy observer.
#[derive(Debug, Deserialize)]
struct TasksQuery {
    namespace: String,
}

fn task_read_error(error: AppError) -> axum::response::Response {
    let (status, message) = match error {
        AppError::InvalidParams(_) => (StatusCode::BAD_REQUEST, "invalid task query"),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "task records unavailable"),
    };
    // Never return database/provider errors that could contain task text or paths.
    (status, Json(json!({ "error": message }))).into_response()
}

async fn task_list(
    State(state): State<Arc<DashboardState>>,
    Query(query): Query<TasksQuery>,
) -> axum::response::Response {
    let Some(store) = state
        .operation_log
        .as_ref()
        .filter(|_| state.task_read_enabled)
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "task records unavailable" })),
        )
            .into_response();
    };
    match list_tasks(
        store,
        ListTasksRequest {
            namespace: query.namespace,
            limit: 100,
            after_id: None,
        },
    )
    .await
    {
        Ok(tasks) => {
            Json(json!({ "tasks": tasks, "limit": 100, "read_only": true })).into_response()
        }
        Err(error) => task_read_error(error),
    }
}

async fn task_snapshot(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
    Query(query): Query<TasksQuery>,
) -> axum::response::Response {
    let Some(store) = state
        .operation_log
        .as_ref()
        .filter(|_| state.task_read_enabled)
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "task records unavailable" })),
        )
            .into_response();
    };
    match get_task_snapshot(store, &query.namespace, &id).await {
        Ok(Some(snapshot)) => {
            Json(json!({ "snapshot": snapshot, "read_only": true })).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "task not found" })),
        )
            .into_response(),
        Err(error) => task_read_error(error),
    }
}

async fn task_experience_uses(
    State(state): State<Arc<DashboardState>>,
    Path(id): Path<String>,
    Query(query): Query<TasksQuery>,
) -> axum::response::Response {
    let Some(store) = state
        .operation_log
        .as_ref()
        .filter(|_| state.task_read_enabled)
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "experience use records unavailable" })),
        )
            .into_response();
    };
    match list_experience_uses(
        store,
        ListExperienceUsesRequest {
            namespace: query.namespace,
            task_id: id,
            limit: 20,
            after_id: None,
        },
    )
    .await
    {
        Ok(uses) => Json(json!({ "uses": uses, "limit": 20, "read_only": true })).into_response(),
        Err(error) => {
            let (status, message) = match error {
                AppError::InvalidParams(_) => {
                    (StatusCode::BAD_REQUEST, "invalid experience use query")
                }
                _ => (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "experience use records unavailable",
                ),
            };
            (status, Json(json!({ "error": message }))).into_response()
        }
    }
}

async fn task_workspaces(State(state): State<Arc<DashboardState>>) -> axum::response::Response {
    let Some(store) = state
        .operation_log
        .as_ref()
        .filter(|_| state.task_read_enabled)
    else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "task workspaces unavailable" })),
        )
            .into_response();
    };
    match store.owner_task_namespaces().await {
        Ok(mut namespaces) => {
            let has_more = namespaces.len() > 100;
            namespaces.truncate(100);
            Json(json!({ "namespaces": namespaces, "limit": 100, "has_more": has_more, "read_only": true })).into_response()
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "task workspaces unavailable" })),
        )
            .into_response(),
    }
}
