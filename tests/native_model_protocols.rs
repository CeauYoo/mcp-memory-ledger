//! Offline native-protocol contract tests. All HTTP traffic stays on loopback.
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use agent_llm_mm::{
    adapters::model::native::{NativeModel, NativeProtocol},
    application::decide_with_snapshot::{self, DecideWithSnapshotInput},
    domain::{
        commitment::Commitment,
        self_revision::{SelfRevisionProposal, SelfRevisionRequest, TriggerType},
        snapshot::SelfSnapshot,
        types::{Namespace, Owner},
    },
    error::AppError,
    ports::{CommitmentStore, ModelDecision, ModelDecisionRequest, ModelPort},
    support::config::NativeModelConfig,
};
use async_trait::async_trait;
use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    routing::post,
};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

const KEY: &str = "native-secret-never-echo-this";

struct Captured {
    uri: String,
    headers: HeaderMap,
    body: Value,
}
#[derive(Clone)]
struct StubState {
    requests: Arc<Mutex<Vec<Captured>>>,
    status: StatusCode,
    body: String,
    delay: Duration,
}
struct Stub {
    base: String,
    state: StubState,
    task: JoinHandle<()>,
}
impl Stub {
    async fn json(body: Value) -> Self {
        Self::raw(200, body.to_string(), 0).await
    }
    async fn raw(status: u16, body: String, delay_ms: u64) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1/", listener.local_addr().unwrap());
        let state = StubState {
            requests: Arc::new(Mutex::new(Vec::new())),
            status: StatusCode::from_u16(status).unwrap(),
            body,
            delay: Duration::from_millis(delay_ms),
        };
        let app = Router::new()
            .fallback(post(respond))
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { base, state, task }
    }
    fn model(&self, protocol: NativeProtocol) -> NativeModel {
        NativeModel::new(self.config(), protocol).unwrap()
    }
    fn config(&self) -> NativeModelConfig {
        NativeModelConfig {
            base_url: self.base.clone(),
            api_key: KEY.into(),
            model: "offline-model".into(),
            timeout_ms: 2000,
            max_tokens: 512,
            temperature: Some(0.25),
        }
    }
    fn count(&self) -> usize {
        self.state.requests.lock().unwrap().len()
    }
}
impl Drop for Stub {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn respond(
    State(state): State<StubState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, String) {
    state.requests.lock().unwrap().push(Captured {
        uri: uri.to_string(),
        headers,
        body: serde_json::from_slice(&body).unwrap(),
    });
    tokio::time::sleep(state.delay).await;
    (state.status, state.body)
}
fn protocols() -> [NativeProtocol; 2] {
    [NativeProtocol::OpenAiResponses, NativeProtocol::Anthropic]
}
fn snapshot() -> SelfSnapshot {
    SelfSnapshot {
        identity: vec!["identity:self=architect".into()],
        commitments: vec![],
        claims: vec!["self.role is architect".into()],
        evidence: vec!["event:evt-1".into()],
        episodes: vec![],
    }
}
fn request() -> ModelDecisionRequest {
    ModelDecisionRequest::new(
        "summarize current memory".into(),
        "read_identity_core".into(),
        snapshot(),
    )
}
fn revision_request() -> SelfRevisionRequest {
    SelfRevisionRequest::new(
        TriggerType::Conflict,
        Namespace::self_(),
        snapshot(),
        vec!["evt-1".into()],
        vec!["claim conflict".into()],
    )
}
fn response(protocol: NativeProtocol, text: &str) -> Value {
    match protocol {
        NativeProtocol::OpenAiResponses => {
            json!({"id":"resp-test","object":"response","status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text}]}]})
        }
        NativeProtocol::Anthropic => {
            json!({"id":"msg-test","type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]})
        }
    }
}
fn user_prompt(protocol: NativeProtocol, body: &Value) -> &str {
    match protocol {
        NativeProtocol::OpenAiResponses => body["input"][0]["content"][0]["text"].as_str().unwrap(),
        NativeProtocol::Anthropic => body["messages"][0]["content"].as_str().unwrap(),
    }
}

#[tokio::test]
async fn native_requests_have_protocol_specific_paths_headers_and_bodies() {
    for protocol in protocols() {
        let stub = Stub::json(response(protocol, "  summarize_memory_state  ")).await;
        let decision = stub.model(protocol).decide(request()).await.unwrap();
        assert_eq!(decision.action, "summarize_memory_state");
        assert_eq!(stub.count(), 1);
        let requests = stub.state.requests.lock().unwrap();
        let captured = &requests[0];
        let body = &captured.body;
        assert_eq!(body["model"], "offline-model");
        assert_eq!(body["temperature"], 0.25);
        assert!(!body.to_string().contains(KEY));
        let prompt = user_prompt(protocol, body);
        assert!(prompt.contains("summarize current memory"));
        assert!(prompt.contains("identity:self=architect"));
        match protocol {
            NativeProtocol::OpenAiResponses => {
                assert_eq!(captured.uri, "/v1/responses");
                assert_eq!(captured.headers["authorization"], format!("Bearer {KEY}"));
                assert!(!captured.headers.contains_key("x-api-key"));
                assert_eq!(body["store"], false);
                assert_eq!(body["max_output_tokens"], 512);
                assert!(body["instructions"].as_str().is_some_and(|s| !s.is_empty()));
                assert_eq!(body["input"][0]["role"], "user");
                assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
                assert!(body.get("messages").is_none());
            }
            NativeProtocol::Anthropic => {
                assert_eq!(captured.uri, "/v1/messages");
                assert_eq!(captured.headers["x-api-key"], KEY);
                assert_eq!(captured.headers["anthropic-version"], "2023-06-01");
                assert!(!captured.headers.contains_key("authorization"));
                assert_eq!(body["max_tokens"], 512);
                assert!(body["system"].as_str().is_some_and(|s| !s.is_empty()));
                assert_eq!(body["messages"][0]["role"], "user");
                assert!(body.get("input").is_none());
            }
        }
    }
}

#[tokio::test]
async fn native_temperature_is_omitted_when_unset_and_root_base_url_works() {
    for protocol in protocols() {
        let stub = Stub::json(response(protocol, "read_identity_core")).await;
        let mut config = stub.config();
        config.temperature = None;
        config.base_url = stub.base.trim_end_matches("/v1/").to_owned();
        NativeModel::new(config, protocol)
            .unwrap()
            .decide(request())
            .await
            .unwrap();
        let requests = stub.state.requests.lock().unwrap();
        assert!(requests[0].body.get("temperature").is_none());
        assert!(matches!(
            requests[0].uri.as_str(),
            "/responses" | "/messages"
        ));
    }
}

#[tokio::test]
async fn native_split_text_ignores_reasoning_and_preserves_revision_json() {
    let proposal = json!({"should_reflect":true,"rationale":"conflict-backed patch","machine_patch":{"identity_patch":{"canonical_claims":["identity:self=mentor"]},"commitment_patch":null},"proposed_evidence_event_ids":["evt-1"],"confidence":"medium"}).to_string();
    let split = proposal.len() / 2;
    for protocol in protocols() {
        let mut body = response(protocol, "unused");
        match protocol {
            NativeProtocol::OpenAiResponses => {
                body["output"][0]["content"] = json!([{"type":"output_text","text":&proposal[..split]},{"type":"output_text","text":&proposal[split..]}]);
                body["output"].as_array_mut().unwrap().insert(0, json!({"type":"reasoning","summary":[{"type":"summary_text","text":"must not become an action"}]}));
            }
            NativeProtocol::Anthropic => {
                body["content"] = json!([{"type":"thinking","thinking":"must not become an action","signature":"fake"},{"type":"redacted_thinking","data":"opaque"},{"type":"text","text":&proposal[..split]},{"type":"text","text":&proposal[split..]}])
            }
        }
        let stub = Stub::json(body).await;
        let result = stub
            .model(protocol)
            .propose_self_revision(revision_request())
            .await
            .unwrap();
        assert!(result.should_reflect);
        assert_eq!(
            result
                .machine_patch
                .identity_patch
                .unwrap()
                .canonical_claims,
            ["identity:self=mentor"]
        );
        assert_eq!(result.proposed_evidence_event_ids, ["evt-1"]);
        assert_eq!(result.confidence.as_deref(), Some("medium"));
        let requests = stub.state.requests.lock().unwrap();
        let prompt = user_prompt(protocol, &requests[0].body);
        assert!(prompt.contains("claim conflict"));
        assert!(prompt.contains("evt-1"));
        assert!(prompt.contains("trigger_type"));
    }
}

#[tokio::test]
async fn responses_rejects_incomplete_refusal_tools_and_malformed_envelopes() {
    let valid = response(NativeProtocol::OpenAiResponses, "read_identity_core");
    let mut cases = vec![json!({}), json!({"status":"completed","output":[]})];
    for status in ["incomplete", "failed", "in_progress", "cancelled"] {
        let mut body = valid.clone();
        body["status"] = json!(status);
        cases.push(body);
    }
    let mut body = valid.clone();
    body.as_object_mut().unwrap().remove("status");
    cases.push(body);
    for status in ["incomplete", "in_progress"] {
        let mut body = valid.clone();
        body["output"][0]["status"] = json!(status);
        cases.push(body);
    }
    let mut body = valid.clone();
    body["output"][0]["role"] = json!("user");
    cases.push(body);
    for content in [
        json!([]),
        json!([{"type":"output_text","text":"  "}]),
        json!([{"type":"refusal","refusal":"declined"}]),
        json!([{"type":"output_text","text":4}]),
        json!([{"type":"output_text","text":"safe"},{"type":"refusal","refusal":"declined"}]),
    ] {
        let mut body = valid.clone();
        body["output"][0]["content"] = content;
        cases.push(body);
    }
    let tool = json!({"type":"function_call","name":"dangerous","arguments":"{}","call_id":"x"});
    let mut body = valid.clone();
    body["output"] = json!([tool.clone()]);
    cases.push(body);
    let mut body = valid.clone();
    body["output"].as_array_mut().unwrap().push(tool);
    cases.push(body);
    let mut body = valid;
    body["error"] = json!({"message":"upstream error"});
    cases.push(body);
    for body in cases {
        assert_rejected(NativeProtocol::OpenAiResponses, body).await;
    }
}

#[tokio::test]
async fn anthropic_rejects_refusal_truncation_tools_and_malformed_envelopes() {
    let valid = response(NativeProtocol::Anthropic, "read_identity_core");
    let mut cases = vec![json!({})];
    for stop in [
        json!("max_tokens"),
        json!("tool_use"),
        json!("pause_turn"),
        json!("refusal"),
        Value::Null,
    ] {
        let mut body = valid.clone();
        body["stop_reason"] = stop;
        cases.push(body);
    }
    let mut body = valid.clone();
    body["stop_details"] = json!({"type":"refusal"});
    cases.push(body);
    let mut body = valid.clone();
    body["role"] = json!("user");
    cases.push(body);
    let mut body = valid.clone();
    body["type"] = json!("error");
    cases.push(body);
    for content in [
        json!([]),
        json!([{"type":"text","text":"  "}]),
        json!([{"type":"text","text":4}]),
        json!([{"type":"thinking","thinking":"not an answer"}]),
        json!([{"type":"tool_use","id":"x","name":"dangerous","input":{}}]),
        json!([{"type":"text","text":"safe"},{"type":"tool_use","id":"x","name":"dangerous","input":{}}]),
    ] {
        let mut body = valid.clone();
        body["content"] = content;
        cases.push(body);
    }
    for body in cases {
        assert_rejected(NativeProtocol::Anthropic, body).await;
    }
}
async fn assert_rejected(protocol: NativeProtocol, body: Value) {
    let stub = Stub::json(body.clone()).await;
    let result = stub.model(protocol).decide(request()).await;
    assert!(result.is_err(), "must reject envelope: {body}");
    assert_eq!(stub.count(), 1, "semantic failures must not retry");
}

#[tokio::test]
async fn native_http_errors_and_bad_json_do_not_retry_or_leak_keys() {
    for protocol in protocols() {
        for (status, body) in [
            (401, format!("{{\"error\":\"{KEY}\"}}")),
            (429, format!("rate limited: {KEY}")),
            (503, format!("outage: {KEY}")),
            (200, "not json {{{".into()),
        ] {
            let stub = Stub::raw(status, body, 0).await;
            let error = stub.model(protocol).decide(request()).await.unwrap_err();
            assert!(!format!("{error:?} {error}").contains(KEY));
            assert_eq!(stub.count(), 1);
        }
    }
}

#[tokio::test]
async fn native_timeout_is_bounded_without_retry_or_key_leak() {
    for protocol in protocols() {
        let stub = Stub::raw(
            200,
            response(protocol, "read_identity_core").to_string(),
            1000,
        )
        .await;
        let mut config = stub.config();
        config.timeout_ms = 100;
        let model = NativeModel::new(config, protocol).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(2), model.decide(request()))
            .await
            .expect("adapter must respect timeout");
        let error = result.unwrap_err();
        assert!(!format!("{error:?} {error}").contains(KEY));
        assert_eq!(stub.count(), 1);
    }
}

#[tokio::test]
async fn native_revision_rejects_malformed_proposals_and_defaults_optional_patch() {
    for protocol in protocols() {
        let stub = Stub::json(response(protocol, "not a JSON proposal")).await;
        assert!(
            stub.model(protocol)
                .propose_self_revision(revision_request())
                .await
                .is_err()
        );
        let stub = Stub::json(response(
            protocol,
            "```json\n{\"should_reflect\":false,\"rationale\":\"insufficient evidence\"}\n```",
        ))
        .await;
        let proposal = stub
            .model(protocol)
            .propose_self_revision(revision_request())
            .await
            .unwrap();
        assert!(!proposal.should_reflect);
        assert!(proposal.machine_patch.identity_patch.is_none());
        assert!(proposal.machine_patch.commitment_patch.is_none());
    }
}

struct DecisionDeps {
    model: NativeModel,
}
#[async_trait]
impl CommitmentStore for DecisionDeps {
    async fn list_commitments(&self) -> Result<Vec<Commitment>, AppError> {
        Ok(vec![Commitment::new(
            Owner::Self_,
            "forbid:write_identity_core_directly",
        )])
    }
}
#[async_trait]
impl ModelPort for DecisionDeps {
    async fn decide(&self, request: ModelDecisionRequest) -> Result<ModelDecision, AppError> {
        self.model.decide(request).await
    }
    async fn propose_self_revision(
        &self,
        request: SelfRevisionRequest,
    ) -> Result<SelfRevisionProposal, AppError> {
        self.model.propose_self_revision(request).await
    }
}
#[tokio::test]
async fn native_decisions_pass_through_server_commitment_gate() {
    for protocol in protocols() {
        let stub = Stub::json(response(protocol, "write_identity_core_directly")).await;
        let deps = DecisionDeps {
            model: stub.model(protocol),
        };
        let mut input = DecideWithSnapshotInput {
            task: "read memory".into(),
            action: "write_identity_core_directly".into(),
            snapshot: snapshot(),
        };
        let blocked = decide_with_snapshot::execute(&deps, input.clone())
            .await
            .unwrap();
        assert!(blocked.blocked);
        assert_eq!(stub.count(), 0, "pre-model gate must prevent HTTP calls");
        input.action = "read_identity_core".into();
        let result = decide_with_snapshot::execute(&deps, input).await.unwrap();
        assert!(result.blocked);
        assert_eq!(
            result.reason.as_deref(),
            Some("commitment_gate_blocked_selected_action")
        );
        assert_eq!(stub.count(), 1);
        let requests = stub.state.requests.lock().unwrap();
        assert!(
            user_prompt(protocol, &requests[0].body)
                .contains("forbid:write_identity_core_directly")
        );
    }
}

#[tokio::test]
async fn anthropic_accepts_completed_stop_sequence() {
    let mut body = response(NativeProtocol::Anthropic, "read_identity_core");
    body["stop_reason"] = json!("stop_sequence");
    body["stop_sequence"] = json!("END");
    let stub = Stub::json(body).await;
    assert_eq!(
        stub.model(NativeProtocol::Anthropic)
            .decide(request())
            .await
            .unwrap()
            .action,
        "read_identity_core"
    );
}

#[tokio::test]
async fn native_redirects_are_not_followed_or_given_credentials() {
    for protocol in protocols() {
        let destination = Stub::json(response(protocol, "redirected_action")).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_url = format!("http://{}", listener.local_addr().unwrap());
        let location = destination.base.clone();
        let app = Router::new().fallback(post(move || async move {
            (
                StatusCode::TEMPORARY_REDIRECT,
                [(axum::http::header::LOCATION, location)],
            )
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut config = destination.config();
        config.base_url = redirect_url;
        let result = NativeModel::new(config, protocol)
            .unwrap()
            .decide(request())
            .await;
        server.abort();
        assert!(result.is_err(), "redirect must be rejected");
        assert_eq!(
            destination.count(),
            0,
            "credentials must never reach redirect destination"
        );
    }
}

// Exercise actual config selection, MCP runtime, native HTTP, trigger policy,
// reflection transaction, and SQLite persistence without a paid provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_mcp_decision_triggers_evidence_backed_reflection_in_sqlite() {
    use agent_llm_mm::support::config::{CONFIG_PATH_ENV_VAR, DATABASE_URL_ENV_VAR};
    for protocol in protocols() {
        let proposal = json!({"should_reflect":true,"rationale":"Conflict suggests tighter commitment hygiene.","machine_patch":{"identity_patch":null,"commitment_patch":{"commitments":["prefer:confirm_conflicting_commitment_updates_before_overwrite"]}}}).to_string();
        // The protocol fixture serves a proposal for both requests, as in the
        // existing MCP auto-reflection regression: first action, then proposal.
        let stub = Stub::json(response(protocol, &proposal)).await;
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!(
            "sqlite://{}",
            directory
                .path()
                .join("native.sqlite")
                .to_string_lossy()
                .replace('\\', "/")
        );
        let store = agent_llm_mm::adapters::sqlite::SqliteStore::bootstrap(&database_url)
            .await
            .unwrap();
        drop(store);
        let config_path = directory.path().join("config.toml");
        let (provider, section) = match protocol {
            NativeProtocol::OpenAiResponses => ("openai-responses", "openai_responses"),
            NativeProtocol::Anthropic => ("anthropic", "anthropic"),
        };
        std::fs::write(&config_path, format!("transport = \"stdio\"\ndatabase_url = {database_url:?}\n[model]\nprovider = {provider:?}\n[model.{section}]\nbase_url = {:?}\napi_key = {KEY:?}\nmodel = \"offline-model\"\ntimeout_ms = 2000\nmax_tokens = 512\n", stub.base)).unwrap();
        let config_path = config_path.to_string_lossy().into_owned();
        let db_for_child = database_url.clone();
        let expected_action = proposal.clone();
        tokio::task::spawn_blocking(move || {
            let mut client = McpClient::spawn(&[(CONFIG_PATH_ENV_VAR, config_path), (DATABASE_URL_ENV_VAR, db_for_child)]);
            client.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"native-offline-test","version":"1"}}}));
            assert!(client.read()["result"].is_object());
            client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
            let ingest = client.call("ingest_interaction", json!({"event":{"owner":"World","namespace":"world","kind":"Conversation","summary":"Seed evidence before resolving a conflicting commitment update."},"claim_drafts":[],"episode_reference":"episode:native-reflection"}));
            assert!(ingest.get("error").is_none(), "{ingest}");
            let snapshot = client.call("build_self_snapshot", json!({"budget":4,"namespace":"world"}));
            let snapshot = snapshot["result"]["structuredContent"]["snapshot"].clone();
            assert!(snapshot.is_object());
            let decision = client.call("decide_with_snapshot", json!({"task":"resolve a conflicting commitment update","action":"overwrite_commitment","snapshot":snapshot,"auto_reflect_namespace":"world","trigger_hints":["conflict","commitment"]}));
            assert!(decision.get("error").is_none(), "{decision}");
            assert_eq!(decision["result"]["structuredContent"]["blocked"], false);
            assert_eq!(decision["result"]["structuredContent"]["decision"]["action"], expected_action);
        }).await.unwrap();
        let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();
        let reflections: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM reflections")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(reflections, 1);
        let status: String = sqlx::query_scalar(
            "SELECT status FROM reflection_trigger_ledger WHERE namespace = 'world'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(status, "handled");
        let commitments: Vec<String> = sqlx::query_scalar("SELECT description FROM commitments")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(
            commitments
                .iter()
                .any(|value| value
                    == "prefer:confirm_conflicting_commitment_updates_before_overwrite")
        );
        assert_eq!(
            stub.count(),
            2,
            "decision and self-revision must each use native transport once"
        );
        pool.close().await;
    }
}

struct McpClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    messages: std::sync::mpsc::Receiver<String>,
}
impl McpClient {
    fn spawn(envs: &[(&str, String)]) -> Self {
        use std::{
            io::BufRead,
            process::{Command, Stdio},
        };
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"));
        for (key, value) in envs {
            command.env(key, value);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, messages) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            messages,
        }
    }
    fn send(&mut self, value: Value) {
        use std::io::Write;
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn read(&self) -> Value {
        loop {
            let line = self
                .messages
                .recv_timeout(Duration::from_secs(15))
                .expect("MCP response within deadline");
            if let Ok(value) = serde_json::from_str::<Value>(&line)
                && value.get("id").is_some()
            {
                return value;
            }
        }
    }
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":name,"arguments":arguments}}));
        self.read()
    }
}
impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
