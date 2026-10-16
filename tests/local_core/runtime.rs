use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use agent_llm_mm::{
    adapters::sqlite::initialize_database,
    interfaces::{
        dashboard::EventQuery,
        local_core::{CoreHandle, start_core},
    },
    ports::{OperationLogQuery, OperationLogStore},
    support::{
        config::{
            AppConfig, DaemonConfig, DashboardConfig, ModelConfig, ModelProviderKind, TransportKind,
        },
        workspace_registry::{
            WorkspaceRegistration, bind_workspace_root, register_workspace, set_personal_grant,
            set_workspace_active, unbind_workspace_root,
        },
    },
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::mpsc;

struct Fixture {
    directory: TempDir,
    config: AppConfig,
    socket: PathBuf,
    registry: PathBuf,
    a: WorkspaceRegistration,
    b: WorkspaceRegistration,
}

impl Fixture {
    async fn new() -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/workspace-core");
        fs::create_dir_all(&parent).unwrap();
        let directory = tempfile::Builder::new()
            .prefix("lc-")
            .tempdir_in(parent)
            .unwrap();
        #[cfg(unix)]
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        #[cfg(windows)]
        super::windows_private::private_directory(directory.path());
        let root_a = directory.path().join("a");
        let root_b = directory.path().join("b");
        fs::create_dir(&root_a).unwrap();
        fs::create_dir(&root_b).unwrap();
        let registry = directory.path().join("registry.json");
        let a = register_workspace(&registry, root_a).unwrap();
        let b = register_workspace(&registry, root_b).unwrap();
        let config = AppConfig {
            transport: TransportKind::Stdio,
            database_url: agent_llm_mm::support::sqlite_url::sqlite_file_url(
                &directory.path().join("memory.sqlite"),
            ),
            model_provider: ModelProviderKind::Mock,
            model_config: ModelConfig::Mock,
            dashboard: DashboardConfig::default(),
            daemon: DaemonConfig::default(),
        };
        initialize_database(&config.database_url).await.unwrap();
        let socket = directory.path().join("c.sock");
        Self {
            directory,
            config,
            socket,
            registry,
            a,
            b,
        }
    }

    async fn start(&self) -> CoreHandle {
        start_core(
            self.config.clone(),
            self.socket.clone(),
            self.registry.clone(),
        )
        .await
        .unwrap()
    }

    fn bridge(&self, workspace: &WorkspaceRegistration, label: &str) -> Bridge {
        self.bridge_binding(&workspace.id, &workspace.roots[0], label)
    }

    fn bridge_binding(&self, id: &str, root: &Path, label: &str) -> Bridge {
        Bridge::spawn(&self.socket, id, root, label, self.directory.path())
    }
}

struct Bridge {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: mpsc::UnboundedReceiver<Result<Value, String>>,
    next_id: u64,
    initialized: bool,
}

impl Bridge {
    fn spawn(socket: &Path, id: &str, root: &Path, label: &str, working_directory: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"));
        command
            .args(["bridge", "--socket"])
            .arg(socket)
            .args(["--workspace-id", id, "--workspace-root"])
            .arg(root)
            .args(["--client-label", label]);
        Self::spawn_command(command, working_directory)
    }

    fn from_registry(socket: &Path, registry: &Path, root: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"));
        command
            .args(["bridge", "--socket"])
            .arg(socket)
            .arg("--registry")
            .arg(registry);
        Self::spawn_command(command, root)
    }

    fn spawn_command(mut command: Command, working_directory: &Path) -> Self {
        // No provider settings or credentials are inherited.
        command.env_clear();
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        let mut child = command
            // A bridge must not parse even an explicitly selected nonexistent
            // provider config. Every successful protocol test checks this.
            .env(
                "AGENT_LLM_MM_CONFIG",
                working_directory.join("synthetic-missing-provider-config.toml"),
            )
            .env("TMPDIR", working_directory)
            .env("TMP", working_directory)
            .env("TEMP", working_directory)
            .env("TOKIO_WORKER_THREADS", "2")
            .current_dir(working_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, responses) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let response = line.map_err(|error| error.to_string()).and_then(|line| {
                    serde_json::from_str(&line).map_err(|error| error.to_string())
                });
                if sender.send(response).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            responses,
            next_id: 1,
            initialized: false,
        }
    }

    fn send(&mut self, value: Value) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        let stdin = self.stdin.as_mut().ok_or("bridge input is closed")?;
        stdin
            .write_all(&bytes)
            .and_then(|()| stdin.flush())
            .map_err(|error| error.to_string())
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))?;
        tokio::time::timeout(Duration::from_secs(8), async {
            while let Some(response) = self.responses.recv().await {
                let response = response?;
                if response.get("id").and_then(Value::as_u64) == Some(id) {
                    return Ok(response);
                }
            }
            Err("bridge closed before response".into())
        })
        .await
        .map_err(|_| "bridge response timed out".to_string())?
    }

    async fn initialize(&mut self) -> Result<(), String> {
        if self.initialized {
            return Ok(());
        }
        let response = self
            .request(
                "initialize",
                json!({
                    "protocolVersion":"2025-03-26", "capabilities":{},
                    "clientInfo":{"name":"local-core-synthetic-test", "version":"1"}
                }),
            )
            .await?;
        if response.get("error").is_some() {
            return Err("MCP initialization rejected".into());
        }
        self.send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))?;
        self.initialized = true;
        Ok(())
    }

    async fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.initialize().await.unwrap();
        self.request("tools/call", json!({"name":tool,"arguments":arguments}))
            .await
            .unwrap()
    }

    async fn tools(&mut self) -> BTreeSet<String> {
        self.initialize().await.unwrap();
        self.request("tools/list", json!({})).await.unwrap()["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    async fn close(&mut self) -> std::process::ExitStatus {
        self.stdin.take();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    return status;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("test bridge must exit after EOF")
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stdin.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn content(response: &Value) -> &Value {
    assert!(response.get("error").is_none(), "tool rejected: {response}");
    assert_ne!(
        response["result"]["isError"], true,
        "tool failed: {response}"
    );
    &response["result"]["structuredContent"]
}

fn denied(response: &Value) {
    assert!(
        response.get("error").is_some() || response["result"]["isError"] == true,
        "expected denial, got {response}"
    );
}

fn ingest(summary: &str) -> Value {
    json!({"event":{"owner":"World", "kind":"Observation", "summary":summary}, "claim_drafts":[]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clients_share_core_without_sharing_project_scopes_or_global_tools() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut a = fixture.bridge(&fixture.a, "client-a");
    let mut a_second = fixture.bridge(&fixture.a, "client-a-second");
    let mut b = fixture.bridge(&fixture.b, "client-b");
    let tools = a.tools().await;
    let expected = [
        "create_task",
        "revise_task",
        "get_task",
        "list_tasks",
        "record_task_checkpoint",
        "build_task_resume_context",
        "ingest_interaction",
        "recall_memory",
        "build_task_context",
        "search_memory",
        "get_memory",
        "get_reflection_history",
        "get_evidence_relation",
        "supersede_memory",
        "record_episode",
        "record_experience_use",
        "derive_experience_from_use",
        "list_experience_uses",
        "get_episode_detail",
        "list_episode_details",
        "propose_experience_candidate",
        "get_experience_candidate",
        "list_experience_candidates",
        "set_experience_candidate_status",
        "revise_experience_candidate",
        "rollback_experience_candidate",
        "recall_experience_candidates",
        "get_feedback_target_version",
        "propose_feedback_candidate",
        "get_feedback_candidate",
        "validate_feedback_candidate",
        "reject_feedback_candidate",
        "commit_feedback_candidate",
        "export_memory",
    ];
    assert_eq!(tools, expected.into_iter().map(str::to_owned).collect());
    for forbidden in [
        "build_self_snapshot",
        "decide_with_snapshot",
        "get_self_model_history",
        "get_self_model_versions",
        "run_reflection",
        "inspect_retrieval_index",
        "rebuild_retrieval_index",
    ] {
        denied(&a.call(forbidden, json!({})).await);
    }
    let written_a = a
        .call("ingest_interaction", ingest("synthetic-alpha memory"))
        .await;
    let event_a = content(&written_a)["event_id"].as_str().unwrap();
    let written_b = b
        .call("ingest_interaction", ingest("synthetic-beta memory"))
        .await;
    let event_b = content(&written_b)["event_id"].as_str().unwrap();
    let recalled = a_second
        .call(
            "recall_memory",
            json!({"query":"synthetic-alpha", "limit":8}),
        )
        .await;
    assert_eq!(content(&recalled)["namespace"], fixture.a.project_namespace);
    assert_eq!(content(&recalled)["records"].as_array().unwrap().len(), 1);
    let own = a_second
        .call("get_memory", json!({"id":format!("event:{event_a}")}))
        .await;
    assert_eq!(content(&own)["record"]["summary"], "synthetic-alpha memory");
    let guessed_foreign_id = a
        .call("get_memory", json!({"id":format!("event:{event_b}")}))
        .await;
    assert!(content(&guessed_foreign_id)["record"].is_null());
    denied(
        &a.call(
            "recall_memory",
            json!({"namespace":fixture.b.project_namespace, "query":"synthetic-beta"}),
        )
        .await,
    );
    let mut forged = ingest("should never be written");
    forged["event"]["namespace"] = json!(fixture.b.project_namespace);
    denied(&a.call("ingest_interaction", forged).await);
    let mut implicit_reflection = ingest("should not invoke model");
    implicit_reflection["trigger_hints"] = json!(["explicit_conflict"]);
    denied(&a.call("ingest_interaction", implicit_reflection).await);
    let no_foreign = b
        .call("recall_memory", json!({"query":"should", "limit":8}))
        .await;
    assert!(
        content(&no_foreign)["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let events = core
        .observer()
        .recorder()
        .unwrap()
        .recent(EventQuery::default());
    assert!(events.iter().any(|event| event.namespace.as_deref()
        == Some(fixture.a.project_namespace.as_str())
        && event.operation == "ingest_interaction"));
    assert!(events.iter().any(|event| event.namespace.as_deref()
        == Some(fixture.b.project_namespace.as_str())
        && event.operation == "ingest_interaction"));
    let client_labels: BTreeSet<_> = events
        .iter()
        .filter_map(|event| event.payload["session"]["client_label"].as_str())
        .collect();
    assert!(client_labels.contains("client-a"));
    assert!(client_labels.contains("client-a-second"));
    assert!(client_labels.contains("client-b"));
    let operations = core
        .store()
        .query_operations(OperationLogQuery {
            namespace: Some(fixture.a.project_namespace.clone()),
            limit: Some(100),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        operations.iter().any(|operation| {
            operation.entrypoint == "recall_memory"
                && operation.status.as_str() == "ok"
                && operation
                    .request_summary_json
                    .as_ref()
                    .is_some_and(|metadata| {
                        let metadata: Value = serde_json::from_str(metadata).unwrap();
                        metadata["workspace_id"] == fixture.a.id
                            && metadata["client_label"] == "client-a-second"
                    })
        }),
        "successful recall must be persisted with session attribution"
    );
    assert!(a.close().await.success());
    assert!(a_second.close().await.success());
    assert!(b.close().await.success());
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn personal_permissions_change_without_reconnecting_and_never_expand_other_sessions() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut a = fixture.bridge(&fixture.a, "personal-a");
    let mut b = fixture.bridge(&fixture.b, "personal-b");
    let recall = json!({"namespace":"user/synthetic-personal", "query":"synthetic"});
    denied(&a.call("recall_memory", recall.clone()).await);
    set_personal_grant(
        &fixture.registry,
        &fixture.a.id,
        "user/synthetic-personal",
        true,
        false,
    )
    .unwrap();
    assert!(
        content(&a.call("recall_memory", recall.clone()).await)["records"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let personal = json!({"namespace":"user/synthetic-personal", "event":{"owner":"User", "kind":"Conversation", "summary":"synthetic personal memory"}, "claim_drafts":[]});
    denied(&a.call("ingest_interaction", personal.clone()).await);
    set_personal_grant(
        &fixture.registry,
        &fixture.a.id,
        "user/synthetic-personal",
        true,
        true,
    )
    .unwrap();
    content(&a.call("ingest_interaction", personal.clone()).await);
    assert_eq!(
        content(&a.call("recall_memory", recall.clone()).await)["records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    denied(&b.call("recall_memory", recall.clone()).await);
    set_personal_grant(
        &fixture.registry,
        &fixture.a.id,
        "user/synthetic-personal",
        true,
        false,
    )
    .unwrap();
    denied(&a.call("ingest_interaction", personal.clone()).await);
    set_personal_grant(
        &fixture.registry,
        &fixture.a.id,
        "user/synthetic-personal",
        false,
        false,
    )
    .unwrap();
    denied(&a.call("recall_memory", recall).await);
    denied(&a.call("ingest_interaction", personal).await);
    assert!(a.close().await.success());
    assert!(b.close().await.success());
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_or_mismatched_workspace_bindings_cannot_initialize() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let unknown_root = fixture.directory.path().join("unregistered");
    fs::create_dir(&unknown_root).unwrap();
    for (id, root) in [
        (uuid::Uuid::new_v4().to_string(), fixture.a.roots[0].clone()),
        (fixture.a.id.clone(), fixture.b.roots[0].clone()),
        (fixture.a.id.clone(), unknown_root),
    ] {
        let mut bridge = fixture.bridge_binding(&id, &root, "rejected");
        assert!(bridge.initialize().await.is_err());
        assert!(!bridge.close().await.success());
    }
    let mut valid = fixture.bridge(&fixture.a, "still-valid");
    assert_eq!(valid.tools().await.len(), 34);
    assert!(valid.close().await.success());
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_one_core_can_own_a_socket_or_database() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    assert!(
        start_core(
            fixture.config.clone(),
            fixture.socket.clone(),
            fixture.registry.clone()
        )
        .await
        .is_err()
    );
    assert!(
        start_core(
            fixture.config.clone(),
            fixture.directory.path().join("other.sock"),
            fixture.registry.clone()
        )
        .await
        .is_err()
    );
    let mut other_database = fixture.config.clone();
    other_database.database_url = agent_llm_mm::support::sqlite_url::sqlite_file_url(
        &fixture.directory.path().join("other.sqlite"),
    );
    initialize_database(&other_database.database_url)
        .await
        .unwrap();
    assert!(
        start_core(
            other_database,
            fixture.socket.clone(),
            fixture.registry.clone()
        )
        .await
        .is_err(),
        "another database must not bypass ownership of the endpoint"
    );
    let mut bridge = fixture.bridge(&fixture.a, "original-core");
    assert_eq!(bridge.tools().await.len(), 34);
    assert!(bridge.close().await.success());
    core.stop().await;
    let restarted = fixture.start().await;
    restarted.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bridge_eof_preserves_core_and_memory_survives_reconnect_and_core_restart() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut first = fixture.bridge(&fixture.a, "first-session");
    let written = first
        .call("ingest_interaction", ingest("synthetic reconnect evidence"))
        .await;
    let event_id = content(&written)["event_id"].as_str().unwrap().to_owned();
    assert!(first.close().await.success());
    let mut reconnected = fixture.bridge(&fixture.a, "second-session");
    let get = reconnected
        .call("get_memory", json!({"id":format!("event:{event_id}")}))
        .await;
    assert_eq!(
        content(&get)["record"]["summary"],
        "synthetic reconnect evidence"
    );
    assert!(reconnected.close().await.success());
    core.stop().await;
    let restarted = fixture.start().await;
    let mut after_restart = fixture.bridge(&fixture.a, "after-core-restart");
    let recall = after_restart
        .call("recall_memory", json!({"query":"reconnect"}))
        .await;
    assert_eq!(content(&recall)["records"].as_array().unwrap().len(), 1);
    assert!(after_restart.close().await.success());
    restarted.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scoped_correction_preserves_sources_and_history_without_cross_scope_reference_writes() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut a = fixture.bridge(&fixture.a, "correction-a");
    let mut b = fixture.bridge(&fixture.b, "correction-b");
    let initial = json!({
        "event":{"owner":"World", "kind":"Observation", "summary":"synthetic source for status"},
        "claim_drafts":[{"owner":"World", "subject":"synthetic.status", "predicate":"is", "object":"old", "mode":"Observed"}]
    });
    let written_a = a.call("ingest_interaction", initial.clone()).await;
    let event_a = content(&written_a)["event_id"].as_str().unwrap();
    let claim_a = format!("claim:{event_a}:claim:0");
    let written_b = b.call("ingest_interaction", initial).await;
    let event_b = content(&written_b)["event_id"].as_str().unwrap();
    let claim_b = format!("claim:{event_b}:claim:0");
    let correction = json!({
        "claim_reference":claim_a,
        "replacement_claim":{"owner":"World", "subject":"synthetic.status", "predicate":"is", "object":"new", "mode":"Observed"},
        "replacement_evidence_event_ids":[format!("event:{event_a}")],
        "summary":"synthetic correction"
    });
    let mut foreign_target = correction.clone();
    foreign_target["claim_reference"] = json!(claim_b);
    denied(&a.call("supersede_memory", foreign_target).await);
    let mut foreign_evidence = correction.clone();
    foreign_evidence["replacement_evidence_event_ids"] = json!([format!("event:{event_b}")]);
    denied(&a.call("supersede_memory", foreign_evidence).await);
    let corrected = a.call("supersede_memory", correction).await;
    let reflection = content(&corrected)["reflection_id"].as_str().unwrap();
    let replacement = content(&corrected)["replacement_claim_id"]
        .as_str()
        .unwrap();
    let old = a
        .call("get_memory", json!({"id":claim_a, "record_type":"Claim"}))
        .await;
    assert_eq!(content(&old)["record"]["status"], "Superseded");
    let new = a
        .call(
            "get_memory",
            json!({"id":replacement, "record_type":"Claim"}),
        )
        .await;
    assert_eq!(content(&new)["record"]["status"], "Active");
    assert_eq!(content(&new)["record"]["object"], "new");
    let source = a
        .call("get_memory", json!({"id":format!("event:{event_a}")}))
        .await;
    assert_eq!(
        content(&source)["record"]["summary"],
        "synthetic source for status"
    );
    let history = a
        .call("get_reflection_history", json!({"claim_reference":claim_a}))
        .await;
    assert_eq!(
        content(&history)["reflections"][0]["reflection_id"],
        reflection
    );
    let foreign = b
        .call("get_memory", json!({"id":claim_b, "record_type":"Claim"}))
        .await;
    assert_eq!(content(&foreign)["record"]["status"], "Active");
    let hidden = b
        .call("get_reflection_history", json!({"claim_reference":claim_a}))
        .await;
    assert!(
        content(&hidden)["reflections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(a.close().await.success());
    assert!(b.close().await.success());
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn core_refuses_implicit_ungated_dashboard_and_daemon_configuration() {
    let fixture = Fixture::new().await;
    let mut dashboard_config = fixture.config.clone();
    dashboard_config.dashboard.enabled = true;
    dashboard_config.dashboard.port = 0;
    assert!(
        start_core(
            dashboard_config,
            fixture.socket.clone(),
            fixture.registry.clone()
        )
        .await
        .is_err()
    );
    let mut daemon_config = fixture.config.clone();
    daemon_config.daemon.enabled = true;
    assert!(
        start_core(
            daemon_config,
            fixture.socket.clone(),
            fixture.registry.clone()
        )
        .await
        .is_err()
    );
    assert!(!fixture.socket.exists());
    let core = fixture.start().await;
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_episode_labels_are_denied_while_rich_episodes_and_exports_remain_scoped() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut a = fixture.bridge(&fixture.a, "episode-a");
    let mut b = fixture.bridge(&fixture.b, "episode-b");
    let empty_a = a.call("export_memory", json!({})).await;
    let empty_b = b.call("export_memory", json!({})).await;
    let mut legacy = ingest("legacy label must not write");
    legacy["episode_reference"] = json!("shared-legacy-label");
    denied(&a.call("ingest_interaction", legacy.clone()).await);
    denied(&b.call("ingest_interaction", legacy).await);
    let unchanged_a = a.call("export_memory", json!({})).await;
    let unchanged_b = b.call("export_memory", json!({})).await;
    assert_eq!(content(&unchanged_a), content(&empty_a));
    assert_eq!(content(&unchanged_b), content(&empty_b));
    assert!(
        content(&unchanged_a)["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        content(&unchanged_b)["events"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let source_a = a
        .call("ingest_interaction", ingest("synthetic episode source A"))
        .await;
    let event_a = content(&source_a)["event_id"].as_str().unwrap();
    let source_b = b
        .call("ingest_interaction", ingest("synthetic episode source B"))
        .await;
    let event_b = content(&source_b)["event_id"].as_str().unwrap();
    let episode = |event_id: &str, title: &str, episode_id: &str| {
        json!({
            "request_id":"synthetic-episode-receipt", "episode_id":episode_id,
            "content":{"title":title, "objective":"verify scoped episodes", "actions":["synthetic action"], "observations":["synthetic observation"], "outcome":"synthetic pass", "lesson":"scope is explicit", "limitations":["synthetic fixture only"], "source_event_refs":[format!("event:{event_id}")]}
        })
    };
    let episode_a_id = uuid::Uuid::new_v4().to_string();
    let episode_b_id = uuid::Uuid::new_v4().to_string();
    let recorded_a = a
        .call(
            "record_episode",
            episode(event_a, "workspace A episode", &episode_a_id),
        )
        .await;
    assert_eq!(
        content(&recorded_a)["record"]["namespace"],
        fixture.a.project_namespace
    );
    let before_other_episode = a.call("export_memory", json!({})).await;
    let collision = b
        .call(
            "record_episode",
            episode(event_b, "duplicate-id-must-fail", &episode_a_id),
        )
        .await;
    denied(&collision);
    assert!(!collision.to_string().contains("workspace A episode"));
    assert!(!collision.to_string().contains(&fixture.a.project_namespace));
    let after_collision = a.call("export_memory", json!({})).await;
    assert_eq!(content(&before_other_episode), content(&after_collision));
    let recorded_b = b
        .call(
            "record_episode",
            episode(event_b, "workspace B episode", &episode_b_id),
        )
        .await;
    assert_eq!(
        content(&recorded_b)["record"]["namespace"],
        fixture.b.project_namespace
    );
    let after_other_episode = a.call("export_memory", json!({})).await;
    assert_eq!(
        content(&before_other_episode),
        content(&after_other_episode),
        "B's scoped rich episode cannot affect A's export"
    );
    assert_eq!(
        content(&after_other_episode)["experience_episodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        content(&after_other_episode)["experience_episodes"][0]["content"]["title"],
        "workspace A episode"
    );
    let own_b = b
        .call("get_episode_detail", json!({"episode_id":episode_b_id}))
        .await;
    assert_eq!(content(&own_b)["content"]["title"], "workspace B episode");
    let hidden = b
        .call("get_episode_detail", json!({"episode_id":episode_a_id}))
        .await;
    assert!(content(&hidden).is_null());
    let mut foreign_source = episode(
        event_b,
        "forbidden foreign source",
        "foreign-source-attempt",
    );
    foreign_source["episode_id"] = json!("foreign-source-attempt");
    foreign_source["request_id"] = json!("foreign-source-attempt");
    denied(&a.call("record_episode", foreign_source).await);
    let final_export = a.call("export_memory", json!({})).await;
    assert_eq!(content(&final_export), content(&after_other_episode));
    assert!(a.close().await.success());
    assert!(b.close().await.success());
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stopping_core_drains_admitted_transaction_before_releasing_database_lock() {
    use sqlx::{Connection, SqliteConnection};
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut client = fixture.bridge(&fixture.a, "shutdown-test");
    client.initialize().await.unwrap();
    let mut writer = SqliteConnection::connect(&fixture.config.database_url)
        .await
        .unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut writer)
        .await
        .unwrap();
    let mut input = ingest("synthetic shutdown barrier event");
    input["request_id"] = json!("shutdown-receipt");
    client.send(json!({"jsonrpc":"2.0","id":900,"method":"tools/call","params":{"name":"ingest_interaction","arguments":input}})).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while core.dispatched_requests() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("tool must pass the final cancellation check before shutdown");
    let stop = tokio::spawn(core.stop());
    // A signal-based barrier confirms stop has begun: the listener is closed and
    // bridges lose their connection, while the actual admitted transaction waits.
    tokio::time::timeout(Duration::from_secs(2), async {
        while client.child.try_wait().unwrap().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("bridge exits promptly even with stdin still open");
    assert!(
        !stop.is_finished(),
        "core must keep lock until admitted DB work ends"
    );
    assert!(
        start_core(
            fixture.config.clone(),
            fixture.directory.path().join("second.sock"),
            fixture.registry.clone()
        )
        .await
        .is_err()
    );
    sqlx::query("ROLLBACK").execute(&mut writer).await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), stop)
        .await
        .unwrap()
        .unwrap();
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE summary = 'synthetic shutdown barrier event'",
    )
    .fetch_one(&mut writer)
    .await
    .unwrap();
    assert_eq!(
        events, 1,
        "admitted write finishes once, without implicit replay"
    );
    let receipts: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM operation_log WHERE actor_id = 'durable_write_receipt_v1'",
    )
    .fetch_one(&mut writer)
    .await
    .unwrap();
    assert_eq!(receipts, 1);
    let next = fixture.start().await;
    next.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn global_bridge_configuration_uses_exact_registered_launch_directory_only() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    for workspace in [&fixture.a, &fixture.b] {
        let mut bridge =
            Bridge::from_registry(&fixture.socket, &fixture.registry, &workspace.roots[0]);
        bridge.initialize().await.unwrap();
        let response = bridge
            .call("recall_memory", json!({"query":"synthetic"}))
            .await;
        assert!(response.get("result").is_some(), "{response}");
        let foreign = if workspace.id == fixture.a.id {
            &fixture.b
        } else {
            &fixture.a
        };
        let denied = bridge
            .call(
                "recall_memory",
                json!({"namespace":foreign.project_namespace,"query":"synthetic"}),
            )
            .await;
        assert!(denied.get("error").is_some());
        assert!(bridge.close().await.success());
    }
    let nested = fixture.a.roots[0].join("unregistered-child");
    fs::create_dir(&nested).unwrap();
    for root in [&nested, fixture.directory.path()] {
        let mut bridge = Bridge::from_registry(&fixture.socket, &fixture.registry, root);
        assert!(bridge.initialize().await.is_err());
        assert!(!bridge.close().await.success());
    }
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancellation_is_connection_local_and_admitted_writes_are_not_rolled_back() {
    use sqlx::{Connection, SqliteConnection};
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut a = fixture.bridge(&fixture.a, "cancel-a");
    let mut b = fixture.bridge(&fixture.b, "cancel-b");
    a.initialize().await.unwrap();
    b.initialize().await.unwrap();
    let mut writer = SqliteConnection::connect(&fixture.config.database_url)
        .await
        .unwrap();
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut writer)
        .await
        .unwrap();
    a.send(json!({"jsonrpc":"2.0","id":900,"method":"tools/call","params":{"name":"ingest_interaction","arguments":ingest("synthetic admitted cancellation")}})).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while core.dispatched_requests() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for _ in 0..100 {
        a.send(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":900}}),
        )
        .unwrap();
    }
    b.next_id = 900;
    let response = b.request("ping", json!({})).await.unwrap();
    assert!(response.get("result").is_some(), "{response}");
    sqlx::query("ROLLBACK").execute(&mut writer).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(5), a.responses.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result["id"], 900);
    assert!(result.get("result").is_some(), "{result}");
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM events WHERE summary='synthetic admitted cancellation'",
    )
    .fetch_one(&mut writer)
    .await
    .unwrap();
    assert_eq!(count, 1);
    a.next_id = 901;
    assert!(
        a.request("ping", json!({}))
            .await
            .unwrap()
            .get("result")
            .is_some()
    );
    assert!(a.close().await.success());
    assert!(b.close().await.success());
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bridge_without_a_core_fails_closed_without_changing_files() {
    let fixture = Fixture::new().await;
    let registry_before = fs::read(&fixture.registry).unwrap();
    let database = fixture.directory.path().join("memory.sqlite");
    let database_before = fs::read(&database).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"));
    command
        .args(["bridge", "--socket"])
        .arg(&fixture.socket)
        .args(["--workspace-id", &fixture.a.id, "--workspace-root"])
        .arg(&fixture.a.roots[0]);
    let mut bridge = Bridge::spawn_command(command, fixture.directory.path());
    assert!(bridge.initialize().await.is_err());
    assert!(!bridge.close().await.success());
    assert_eq!(fs::read(&fixture.registry).unwrap(), registry_before);
    assert_eq!(fs::read(&database).unwrap(), database_before);
    assert!(!fixture.socket.exists());
    assert!(!fixture.registry.with_extension("json.lock").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disable_and_unbind_revoke_live_sessions_before_receipt_replay() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut a = fixture.bridge(&fixture.a, "lifecycle-a");
    let mut b = fixture.bridge(&fixture.b, "lifecycle-b");
    let mut write = ingest("synthetic lifecycle receipt");
    write["request_id"] = json!("synthetic-lifecycle-replay");
    let original = a.call("ingest_interaction", write.clone()).await;
    content(&original);
    set_workspace_active(&fixture.registry, &fixture.a.id, false).unwrap();
    denied(&a.request("tools/list", json!({})).await.unwrap());
    denied(&a.call("ingest_interaction", write.clone()).await);
    assert_eq!(b.tools().await.len(), 34);
    set_workspace_active(&fixture.registry, &fixture.a.id, true).unwrap();
    let replay = a.call("ingest_interaction", write.clone()).await;
    assert_eq!(content(&replay)["event_id"], content(&original)["event_id"]);
    assert_eq!(content(&replay)["replayed"], true);
    unbind_workspace_root(&fixture.registry, &fixture.a.id, &fixture.a.roots[0]).unwrap();
    denied(&a.request("tools/list", json!({})).await.unwrap());
    denied(&a.call("ingest_interaction", write.clone()).await);
    bind_workspace_root(&fixture.registry, &fixture.a.id, &fixture.a.roots[0]).unwrap();
    let replay = a.call("ingest_interaction", write).await;
    assert_eq!(content(&replay)["event_id"], content(&original)["event_id"]);
    assert_eq!(content(&replay)["replayed"], true);
    let recalled = a
        .call(
            "recall_memory",
            json!({"query":"synthetic lifecycle receipt"}),
        )
        .await;
    assert_eq!(content(&recalled)["records"].as_array().unwrap().len(), 1);
    assert!(a.close().await.success());
    assert!(b.close().await.success());
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn task_tools_use_live_scope_binding_and_revoke_before_receipt_replay() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut a = fixture.bridge(&fixture.a, "task-client-a");
    let mut b = fixture.bridge(&fixture.b, "task-client-b");
    let request = json!({"task_id":"scoped-task","request_id":"scoped-task-create","title":"Synthetic task continuity","source":{"kind":"caller_reported","reference":null},"reason":"Test session-bound task creation","requirements":[{"requirement_id":"keep-scope","kind":"prohibition","text":"Do not leave the selected workspace","state":"reported","source":{"kind":"caller_reported","reference":null},"replaces":null}]});
    let created = a.call("create_task", request.clone()).await;
    assert_eq!(content(&created)["record"]["version"], 1);
    let scope = content(&created)["record"]["namespace"].clone();
    let built = a
        .call(
            "build_task_resume_context",
            json!({"task_id":"scoped-task","max_bytes":16384}),
        )
        .await;
    assert_eq!(content(&built)["mandatory_requirement_count"], 1);
    assert_eq!(content(&built)["can_resume"], true);
    let other = b.call("get_task", json!({"task_id":"scoped-task"})).await;
    assert!(content(&other).is_null());
    denied(
        &b.call(
            "get_task",
            json!({"namespace":scope,"task_id":"scoped-task"}),
        )
        .await,
    );
    set_workspace_active(&fixture.registry, &fixture.a.id, false).unwrap();
    for name in [
        "create_task",
        "revise_task",
        "get_task",
        "list_tasks",
        "record_task_checkpoint",
        "build_task_resume_context",
    ] {
        let args = if name == "create_task" {
            request.clone()
        } else {
            json!({"task_id":"scoped-task"})
        };
        denied(&a.call(name, args).await);
    }
    set_workspace_active(&fixture.registry, &fixture.a.id, true).unwrap();
    assert_eq!(
        content(&a.call("create_task", request).await)["replayed"],
        true
    );
    assert!(a.close().await.success());
    assert!(b.close().await.success());
    core.stop().await;
}

/// Real native adapter -> spawned stdio bridge -> live scoped core. This does not
/// assert that Codex itself received or understood stdout, and uses no model.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn native_context_adapter_reads_live_task_and_stops_output_after_pause_or_revocation() {
    let fixture = Fixture::new().await;
    let core = fixture.start().await;
    let mut bridge = fixture.bridge(&fixture.a, "adapter-task-seed");
    let created = bridge.call("create_task", json!({
        "task_id":"adapter-task", "request_id":"adapter-task-create",
        "title":"Synthetic adapter continuity", "reason":"Exercise real native transport",
        "source":{"kind":"caller_reported","reference":null},
        "requirements":[{"requirement_id":"required-scope","kind":"prohibition",
            "text":"SYNTHETIC_REQUIRED_CONSTRAINT: retain the selected workspace scope",
            "state":"reported","source":{"kind":"caller_reported","reference":null},"replaces":null}]
    })).await;
    let namespace = content(&created)["record"]["namespace"].clone();
    let state = fixture.directory.path().join("adapter-state.sqlite");
    let root = fixture.a.roots[0].canonicalize().unwrap();
    // Follow the harness's existing child-process isolation. A deliberately absent
    // provider config also checks that this path remains independent of model setup.
    let adapter = |action: &str, extra: &[(&str, &std::ffi::OsStr)], input: Option<&Value>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"));
        command.env_clear();
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        command
            .args(["codex-context", action, "--state"])
            .arg(&state)
            .env(
                "AGENT_LLM_MM_CONFIG",
                fixture.directory.path().join("synthetic-no-provider.toml"),
            )
            .env("TMPDIR", fixture.directory.path())
            .env("TMP", fixture.directory.path())
            .env("TEMP", fixture.directory.path())
            .env("TOKIO_WORKER_THREADS", "2")
            .current_dir(&root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, value) in extra {
            command.arg(name).arg(value);
        }
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        if let Some(input) = input {
            stdin
                .write_all(&serde_json::to_vec(input).unwrap())
                .unwrap();
        }
        drop(stdin);
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "native adapter {action} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap_or_else(|error| {
            panic!("native adapter {action} emitted invalid stdout: {error}")
        })
    };
    assert_eq!(adapter("init", &[], None)["schema_version"], 1);
    let selected = adapter(
        "select-default",
        &[
            ("--registry", fixture.registry.as_os_str()),
            ("--socket", fixture.socket.as_os_str()),
            ("--root", root.as_os_str()),
            ("--workspace-id", std::ffi::OsStr::new(&fixture.a.id)),
            ("--task-id", std::ffi::OsStr::new("adapter-task")),
        ],
        None,
    );
    assert_eq!(selected["selected"]["task_id"], "adapter-task");
    assert_eq!(selected["delivery"], "unverified");
    let hook_flags = [
        ("--registry", fixture.registry.as_os_str()),
        ("--socket", fixture.socket.as_os_str()),
    ];
    let input = json!({"session_id":"synthetic-native-session","cwd":root,"hook_event_name":"SessionStart","source":"startup"});
    let output = adapter("hook", &hook_flags, Some(&input));
    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    let text = output["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(text.contains("SYNTHETIC_REQUIRED_CONSTRAINT"));
    let envelope: Value = serde_json::from_str(text.split_once('\n').unwrap().1).unwrap();
    assert_eq!(
        envelope["binding"]["host_session_id"],
        "synthetic-native-session"
    );
    assert_eq!(envelope["binding"]["workspace_id"], fixture.a.id);
    let context = &envelope["context"];
    assert_eq!(context["namespace"], namespace);
    assert_eq!(context["task_id"], "adapter-task");
    assert_eq!(context["task_version"], 1);
    assert_eq!(context["complete"], true);
    assert_eq!(context["can_resume"], true);
    assert_eq!(context["mandatory_snapshot_consistent"], true);
    assert_eq!(context["mandatory_requirement_count"], 1);
    assert_eq!(
        context["mandatory_requirements"][0]["requirement_id"],
        "required-scope"
    );
    let inspect_flags = [(
        "--session-id",
        std::ffi::OsStr::new("synthetic-native-session"),
    )];
    let receipt = adapter("inspect", &inspect_flags, None);
    assert_eq!(receipt["delivery"], "unverified");
    assert_eq!(receipt["model_adoption"], "unknown");
    assert_eq!(receipt["result_effect"], "unknown");
    assert_eq!(receipt["receipts"].as_array().unwrap().len(), 1);
    assert_eq!(receipt["receipts"][0]["stage"], "output_written");
    assert_eq!(receipt["receipts"][0]["context_id"], context["context_id"]);
    assert_eq!(
        receipt["receipts"][0]["task_version"],
        context["task_version"]
    );
    let paused = bridge.call("revise_task", json!({"task_id":"adapter-task","request_id":"pause-adapter-task",
        "expected_version":1,"changes":[],"status":"paused","source":{"kind":"caller_reported","reference":null},"reason":"Pause before resuming"})).await;
    assert_eq!(content(&paused)["record"]["status"], "paused");
    let resume = json!({"session_id":"synthetic-native-session","cwd":root,"hook_event_name":"SessionStart","source":"resume"});
    let paused_output = adapter("hook", &hook_flags, Some(&resume));
    assert!(paused_output.get("systemMessage").is_some());
    assert!(paused_output.get("hookSpecificOutput").is_none());
    assert!(
        !paused_output
            .to_string()
            .contains("SYNTHETIC_REQUIRED_CONSTRAINT")
    );
    let paused_receipt = adapter("inspect", &inspect_flags, None);
    assert_eq!(paused_receipt["minimum_task_version_seen"], 2);
    let paused_receipts = paused_receipt["receipts"].as_array().unwrap();
    assert_eq!(paused_receipts.len(), 2);
    let observed_pause = paused_receipts
        .iter()
        .find(|r| r["task_version"] == 2)
        .unwrap();
    assert_eq!(observed_pause["stage"], "core_built");
    assert!(
        paused_receipts
            .iter()
            .all(|r| r["task_version"] == 1 || r["stage"] != "output_written")
    );
    let active = bridge.call("revise_task", json!({"task_id":"adapter-task","request_id":"reactivate-adapter-task",
        "expected_version":2,"changes":[],"status":"active","source":{"kind":"caller_reported","reference":null},"reason":"Test independent registry revocation"})).await;
    assert_eq!(content(&active)["record"]["status"], "active");
    set_workspace_active(&fixture.registry, &fixture.a.id, false).unwrap();
    let revoked_output = adapter("hook", &hook_flags, Some(&resume));
    assert!(revoked_output.get("systemMessage").is_some());
    assert!(revoked_output.get("hookSpecificOutput").is_none());
    assert!(
        !revoked_output
            .to_string()
            .contains("SYNTHETIC_REQUIRED_CONSTRAINT")
    );
    // A received pause raises the version floor but is never marked output-written.
    // Registry revocation occurs before core access and adds no further receipt.
    let final_receipt = adapter("inspect", &inspect_flags, None);
    assert_eq!(final_receipt["receipts"].as_array().unwrap().len(), 2);
    assert_eq!(final_receipt["minimum_task_version_seen"], 2);
    assert_eq!(final_receipt["receipts"][0]["stage"], "core_built");
    assert_eq!(final_receipt["delivery"], "unverified");
    assert!(bridge.close().await.success());
    core.stop().await;
}
