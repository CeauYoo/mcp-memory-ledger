use agent_llm_mm::support::sqlite_url::sqlite_file_url as sqlite_url;
use std::{
    fs,
    io::{self, BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

use agent_llm_mm::{
    adapters::sqlite::initialize_database, domain::event::EventReference, run_doctor,
    support::config::AppConfig,
};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Row, sqlite::SqlitePool};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::oneshot,
};

#[derive(Debug)]
struct DemoArgs {
    output_dir: PathBuf,
    server_bin: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
struct SqliteSummary {
    commitments: Vec<Value>,
    reflection_trigger_ledger: Vec<Value>,
    reflections: Vec<Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    fs::create_dir_all(&args.output_dir)?;

    let database_path = args.output_dir.join("demo.sqlite");
    remove_existing_sqlite_files(&database_path)?;
    let database_url = sqlite_url(&database_path);
    let config_path = args.output_dir.join("agent-llm-mm.demo.toml");

    let stub = DemoStub::spawn().await?;
    fs::write(
        &config_path,
        demo_config(&database_url, stub.base_url.as_str()),
    )?;

    let config = AppConfig::load_from_path(&config_path).map_err(anyhow::Error::msg)?;
    initialize_database(&config.database_url).await?;
    let doctor = run_doctor(config).await?;
    write_json(&args.output_dir.join("doctor.json"), &doctor)?;

    let server_bin = args
        .server_bin
        .unwrap_or_else(|| sibling_binary("agent_llm_mm"));
    let mut client = StdioClient::spawn(&server_bin, &config_path)
        .with_context(|| format!("failed to spawn MCP server at {}", server_bin.display()))?;
    let _ = client.list_all_tools()?;

    let baseline = client.call_tool(
        "ingest_interaction",
        json!({
            "event": {
                "owner": "User",
                "kind": "Conversation",
                "summary": "Baseline demo memory: the agent is acting as an architect."
            },
            "claim_drafts": [{
                "owner": "Self_",
                "subject": "self.role",
                "predicate": "is",
                "object": "architect",
                "mode": "Observed"
            }],
            "episode_reference": "episode:demo-baseline"
        }),
    )?;

    let snapshot_before =
        extract_snapshot(client.call_tool("build_self_snapshot", json!({ "budget": 8 }))?)?;
    write_json(
        &args.output_dir.join("snapshot-before.json"),
        &snapshot_before,
    )?;

    let gate_before = extract_structured(client.call_tool(
        "decide_with_snapshot",
        json!({
            "task": "attempt forbidden direct identity write",
            "action": "write_identity_core_directly",
            "snapshot": snapshot_before
        }),
    )?)?;

    let decision_before = extract_structured(client.call_tool(
        "decide_with_snapshot",
        json!({
            "task": "review update",
            "action": "review_conflicting_commitment_update",
            "snapshot": snapshot_before
        }),
    )?)?;
    write_json(
        &args.output_dir.join("decision-before.json"),
        &decision_before,
    )?;

    let _ = client.call_tool(
        "ingest_interaction",
        json!({
            "event": {
                "owner": "Self_",
                "kind": "Action",
                "summary": "self attempted a conflicting commitment overwrite"
            },
            "claim_drafts": [],
            "episode_reference": "episode:demo-conflict-negative"
        }),
    )?;
    let negative_handled_conflict_rows = count_handled_conflicts(&database_url).await?;

    let _ = client.call_tool(
        "ingest_interaction",
        json!({
            "event": {
                "owner": "Self_",
                "kind": "Action",
                "summary": "self attempted a commitment overwrite that requires confirmation"
            },
            "claim_drafts": [],
            "episode_reference": "episode:demo-conflict-positive",
            "trigger_hints": ["conflict", "commitment"]
        }),
    )?;
    let positive_handled_conflict_rows = count_handled_conflicts(&database_url).await?;

    let snapshot_after =
        extract_snapshot(client.call_tool("build_self_snapshot", json!({ "budget": 8 }))?)?;
    write_json(
        &args.output_dir.join("snapshot-after.json"),
        &snapshot_after,
    )?;

    let decision_after = extract_structured(client.call_tool(
        "decide_with_snapshot",
        json!({
            "task": "review update",
            "action": "review_conflicting_commitment_update",
            "snapshot": snapshot_after
        }),
    )?)?;
    write_json(
        &args.output_dir.join("decision-after.json"),
        &decision_after,
    )?;

    let sqlite_summary = query_sqlite_summary(&database_url).await?;
    write_json(
        &args.output_dir.join("sqlite-summary.json"),
        &sqlite_summary,
    )?;

    let baseline_event_reference = EventReference::parse(
        baseline["result"]["structuredContent"]["event_id"]
            .as_str()
            .context("baseline ingest response is missing event_id")?,
    )
    .map_err(|error| anyhow::anyhow!("baseline ingest returned an invalid event_id: {error:?}"))?
    .canonical();
    let timeline = json!({
        "baseline": {
            "event_reference": baseline_event_reference
        },
        "gate_before": gate_before,
        "negative_conflict": {
            "handled_conflict_rows": negative_handled_conflict_rows
        },
        "positive_conflict": {
            "handled_conflict_rows": positive_handled_conflict_rows
        }
    });
    write_json(&args.output_dir.join("timeline.json"), &timeline)?;

    fs::write(
        args.output_dir.join("report.md"),
        render_report(&doctor_value(&args.output_dir)?, &sqlite_summary, &timeline),
    )?;

    Ok(())
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<DemoArgs> {
    let mut output_dir = None;
    let mut server_bin = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--output-dir" => {
                output_dir = args.next().map(PathBuf::from);
            }
            "--server-bin" => {
                server_bin = args.next().map(PathBuf::from);
            }
            _ => anyhow::bail!("unknown argument: {arg}"),
        }
    }

    Ok(DemoArgs {
        output_dir: output_dir.unwrap_or_else(default_output_dir),
        server_bin,
    })
}

fn default_output_dir() -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    PathBuf::from("target")
        .join("reports")
        .join("self-revision-demo")
        .join(timestamp)
}

fn demo_config(database_url: &str, stub_base_url: &str) -> String {
    format!(
        r#"transport = "stdio"
database_url = "{database_url}"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "{stub_base_url}"
api_key = "demo-local-key"
model = "demo-local"
timeout_ms = 30000
"#
    )
}

fn remove_existing_sqlite_files(database_path: &Path) -> Result<()> {
    for path in [
        database_path.to_path_buf(),
        database_path.with_extension("sqlite-shm"),
        database_path.with_extension("sqlite-wal"),
    ] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("remove {}", path.display())),
        }
    }
    Ok(())
}

fn sibling_binary(name: &str) -> PathBuf {
    let mut path = std::env::current_exe().expect("current exe");
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.join(name)
}

fn extract_structured(response: Value) -> Result<Value> {
    if let Some(error) = response.get("error") {
        anyhow::bail!("MCP call failed: {error}");
    }
    response
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .cloned()
        .context("missing structuredContent")
}

fn extract_snapshot(response: Value) -> Result<Value> {
    extract_structured(response)?
        .get("snapshot")
        .cloned()
        .context("missing snapshot")
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn doctor_value(output_dir: &Path) -> Result<Value> {
    Ok(serde_json::from_slice(&fs::read(
        output_dir.join("doctor.json"),
    )?)?)
}

async fn count_handled_conflicts(database_url: &str) -> Result<i64> {
    let pool = SqlitePool::connect(database_url).await?;
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reflection_trigger_ledger WHERE trigger_type = 'conflict' AND status = 'handled'",
    )
    .fetch_one(&pool)
    .await?;
    Ok(count)
}

async fn query_sqlite_summary(database_url: &str) -> Result<SqliteSummary> {
    let pool = SqlitePool::connect(database_url).await?;

    let commitments = sqlx::query("SELECT owner, description FROM commitments ORDER BY rowid")
        .fetch_all(&pool)
        .await?
        .into_iter()
        .map(|row| {
            json!({
                "owner": row.get::<String, _>("owner"),
                "description": row.get::<String, _>("description")
            })
        })
        .collect();

    let reflection_trigger_ledger = sqlx::query(
        r#"
        SELECT trigger_type, namespace, trigger_key, status, reflection_id
        FROM reflection_trigger_ledger
        ORDER BY rowid
        "#,
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|row| {
        json!({
            "trigger_type": row.get::<String, _>("trigger_type"),
            "namespace": row.get::<String, _>("namespace"),
            "trigger_key": row.get::<String, _>("trigger_key"),
            "status": row.get::<String, _>("status"),
            "reflection_id": row.get::<Option<String>, _>("reflection_id")
        })
    })
    .collect();

    let reflections = sqlx::query(
        r#"
        SELECT reflection_id, summary, supporting_evidence_event_ids, requested_commitment_updates
        FROM reflections
        ORDER BY rowid
        "#,
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|row| {
        json!({
            "reflection_id": row.get::<String, _>("reflection_id"),
            "summary": row.get::<String, _>("summary"),
            "supporting_evidence_event_ids": row.get::<Option<String>, _>("supporting_evidence_event_ids"),
            "requested_commitment_updates": row.get::<Option<String>, _>("requested_commitment_updates")
        })
    })
    .collect();

    Ok(SqliteSummary {
        commitments,
        reflection_trigger_ledger,
        reflections,
    })
}

fn render_report(doctor: &Value, sqlite_summary: &SqliteSummary, timeline: &Value) -> String {
    let hooks = doctor["auto_reflection_runtime_hooks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|hook| format!("- `{hook}`"))
        .collect::<Vec<_>>()
        .join("\n");
    let commitments = sqlite_summary
        .commitments
        .iter()
        .filter_map(|value| value["description"].as_str())
        .map(|commitment| format!("- `{commitment}`"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "# Self-Revision Demo Report\n\n\
        ## Environment\n\n\
        - transport: `{}`\n\
        - provider: `{}`\n\
        - durable write path: `{}`\n\
        - status: `{}`\n\n\
        ## Runtime Hooks\n\n{}\n\n\
        ## Timeline\n\n\
        - gate before update blocked: `{}`\n\
        - handled conflict rows before explicit hints: `{}`\n\
        - handled conflict rows after explicit hints: `{}`\n\n\
        ## Current Commitments\n\n{}\n",
        doctor["transport"].as_str().unwrap_or("unknown"),
        doctor["provider"].as_str().unwrap_or("unknown"),
        doctor["self_revision_write_path"]
            .as_str()
            .unwrap_or("unknown"),
        doctor["status"].as_str().unwrap_or("unknown"),
        hooks,
        timeline["gate_before"]["blocked"]
            .as_bool()
            .unwrap_or(false),
        timeline["negative_conflict"]["handled_conflict_rows"]
            .as_i64()
            .unwrap_or_default(),
        timeline["positive_conflict"]["handled_conflict_rows"]
            .as_i64()
            .unwrap_or_default(),
        commitments
    )
}

struct StdioClient {
    child: Child,
    initialized: bool,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl StdioClient {
    fn spawn(server_bin: &Path, config_path: &Path) -> io::Result<Self> {
        let mut child = Command::new(server_bin)
            .env(
                agent_llm_mm::support::config::CONFIG_PATH_ENV_VAR,
                config_path.to_string_lossy().into_owned(),
            )
            .env_remove(agent_llm_mm::support::config::DATABASE_URL_ENV_VAR)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("missing child stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing child stdout"))?;
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut stderr = stderr;
                let _ = io::copy(&mut stderr, &mut io::sink());
            });
        }

        Ok(Self {
            child,
            initialized: false,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn list_all_tools(&mut self) -> io::Result<Value> {
        self.initialize()?;
        self.send(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }))?;
        self.read_message()
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> io::Result<Value> {
        self.initialize()?;
        self.send(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        }))?;
        self.read_message()
    }

    fn initialize(&mut self) -> io::Result<()> {
        if self.initialized {
            return Ok(());
        }

        self.send(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {
                    "name": "self-revision-demo-runner",
                    "version": "0.1.0"
                }
            }
        }))?;
        let _ = self.read_message()?;

        self.send(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))?;
        self.initialized = true;
        Ok(())
    }

    fn send(&mut self, payload: Value) -> io::Result<()> {
        let mut body = serde_json::to_vec(&payload)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        body.push(b'\n');
        self.stdin.write_all(&body)?;
        self.stdin.flush()
    }

    fn read_message(&mut self) -> io::Result<Value> {
        loop {
            let mut line = String::new();
            let bytes_read = self.stdout.read_line(&mut line)?;
            if bytes_read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "child process closed stdout before sending an MCP message",
                ));
            }

            let trimmed = line.trim();
            if !trimmed.starts_with('{') {
                continue;
            }

            return serde_json::from_str(trimmed)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
        }
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct DemoStub {
    base_url: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl DemoStub {
    async fn spawn() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let base_url = format!("http://{address}");
        let (shutdown_tx, mut shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accept = listener.accept() => {
                        if let Ok((mut stream, _)) = accept {
                            tokio::spawn(async move {
                                let _ = handle_stub_connection(&mut stream).await;
                            });
                        }
                    }
                }
            }
        });

        Ok(Self {
            base_url,
            shutdown: Some(shutdown_tx),
        })
    }
}

impl Drop for DemoStub {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

async fn handle_stub_connection(stream: &mut TcpStream) -> Result<()> {
    let request = read_http_request(stream).await?;
    let body = request
        .split("\r\n\r\n")
        .nth(1)
        .context("missing http body")?;
    let json_body: Value = serde_json::from_str(body)?;
    let content = classify_demo_response(&json_body);
    let response = json!({
        "id": "chatcmpl-demo",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": content
            }
        }]
    });
    let response_text = response.to_string();
    let http = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        response_text.len(),
        response_text
    );
    stream.write_all(http.as_bytes()).await?;
    Ok(())
}

async fn read_http_request(stream: &mut TcpStream) -> Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end = None;
    let mut content_length = None;

    loop {
        let bytes_read = stream.read(&mut chunk).await?;
        if bytes_read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..bytes_read]);

        if header_end.is_none() {
            header_end = find_header_end(&buffer);
            if let Some(end) = header_end {
                let headers = String::from_utf8_lossy(&buffer[..end]);
                content_length = parse_content_length(&headers);
            }
        }

        if let (Some(end), Some(length)) = (header_end, content_length)
            && buffer.len() >= end + 4 + length
        {
            break;
        }
    }

    String::from_utf8(buffer).context("request was not valid utf-8")
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> Option<usize> {
    headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    })
}

fn classify_demo_response(request: &Value) -> String {
    let system = request["messages"][0]["content"]
        .as_str()
        .unwrap_or_default();
    let user = request["messages"][1]["content"]
        .as_str()
        .unwrap_or_default();

    if system.contains("Return only the next action name") && user.contains("Task:") {
        let action =
            if user.contains("prefer:confirm_conflicting_commitment_updates_before_overwrite") {
                "confirm_conflicting_commitment_updates_before_overwrite"
            } else {
                "apply_commitment_update_now"
            };
        return action.to_string();
    }

    if system.contains("Return only a JSON self-revision proposal")
        && user.contains("Self revision request:")
    {
        return json!({
            "should_reflect": true,
            "rationale": "Conflict evidence suggests tighter commitment hygiene.",
            "machine_patch": {
                "identity_patch": null,
                "commitment_patch": {
                    "commitments": [
                        "prefer:confirm_conflicting_commitment_updates_before_overwrite"
                    ]
                }
            },
            "proposed_evidence_event_ids": [],
            "proposed_evidence_query": {
                "owner": "Self_",
                "kind": "Action",
                "limit": 1
            },
            "confidence": "high"
        })
        .to_string();
    }

    "unsupported_demo_request".to_string()
}
