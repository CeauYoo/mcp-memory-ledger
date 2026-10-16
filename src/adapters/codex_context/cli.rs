use super::{
    protocol::{self, Binding, HookInput},
    state::State,
    transport,
};
use crate::support::workspace_registry::resolve_workspace;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub async fn try_run_cli(args: &[String]) -> Result<bool> {
    if args.first().map(String::as_str) != Some("codex-context") {
        return Ok(false);
    }
    let action = args.get(1).map(String::as_str).unwrap_or("help");
    if action == "help" {
        println!(
            "codex-context init --state ABS_PATH\ncodex-context bind --state ABS_PATH --registry ABS_PATH --socket ABS_PATH --root ABS_ROOT --workspace-id ID --task-id ID --session-id ID\ncodex-context select-default (same flags as bind, without --session-id)\ncodex-context inspect --state ABS_PATH --session-id ID\ncodex-context preview --state ABS_PATH --registry ABS_PATH --socket ABS_PATH [--max-bytes 8192]\ncodex-context hook (same flags as preview; public hook JSON on stdin)\ncodex-context capabilities (reports installed CLI version only; does not enable hooks)"
        );
        return Ok(true);
    }
    let mut flags = parse_flags(&args[2..])?;
    if action == "capabilities" {
        ensure!(flags.is_empty(), "capabilities takes no flags");
        // A version probe does not inspect credentials, transcripts, or global config.
        let report = tokio::task::spawn_blocking(capabilities).await??;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(true);
    }
    let state_path = absolute(take(&mut flags, "--state")?)?;
    match action {
        "init" => {
            ensure!(flags.is_empty(), "unexpected init flag");
            State::initialize(&state_path).await?;
            println!("{}", json!({"initialized":state_path,"schema_version":1}));
        }
        "inspect" => {
            let session = take(&mut flags, "--session-id")?;
            ensure!(flags.is_empty(), "unexpected inspect flag");
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &State::open(&state_path).await?.inspect(&session).await?
                )?
            );
        }
        "bind" | "select-default" => {
            let registry = absolute(take(&mut flags, "--registry")?)?;
            let socket = absolute(take(&mut flags, "--socket")?)?;
            let root = absolute(take(&mut flags, "--root")?)?.canonicalize()?;
            let workspace_id = take(&mut flags, "--workspace-id")?;
            let task_id = take(&mut flags, "--task-id")?;
            let session = if action == "bind" {
                take(&mut flags, "--session-id")?
            } else {
                "explicit-default-selection".into()
            };
            ensure!(flags.is_empty(), "unexpected binding flag");
            protocol::identifier(&task_id)?;
            protocol::identifier(&session)?;
            let grant = resolve_workspace(&registry, &root)?;
            ensure!(
                grant.workspace_id == workspace_id,
                "workspace root/id mismatch"
            );
            let binding = Binding {
                host_session_id: session,
                workspace_id,
                namespace: grant.project_namespace,
                task_id,
                workspace_root: root,
            };
            let context = fetch(&socket, &binding, protocol::DEFAULT_CONTEXT_BYTES).await?;
            protocol::context_version(&context, &binding)?;
            ensure!(
                context["status"] == "active",
                "only active tasks can be selected"
            );
            let state = State::open(&state_path).await?;
            if action == "bind" {
                state.bind(&binding).await?;
            } else {
                state.select_default(&binding).await?;
            }
            println!(
                "{}",
                json!({"selected":binding,"scope":action,"delivery":"unverified","global_config_changed":false})
            );
        }
        "preview" | "hook" => {
            let registry = absolute(take(&mut flags, "--registry")?)?;
            let socket = absolute(take(&mut flags, "--socket")?)?;
            let max_bytes = protocol::budget(
                flags
                    .remove("--max-bytes")
                    .map(|s| s.parse())
                    .transpose()?
                    .unwrap_or(protocol::DEFAULT_CONTEXT_BYTES),
            )?;
            ensure!(flags.is_empty(), "unexpected adapter flag");
            if action == "preview" {
                let preview = preview(
                    &std::env::current_exe()?,
                    &state_path,
                    &registry,
                    &socket,
                    max_bytes,
                )?;
                println!("{}", serde_json::to_string_pretty(&preview)?);
            } else {
                // Failure only reports degraded context. It does not claim to gate
                // tools or change native host approval/sandbox behavior.
                if let Err(error) = run_hook(&state_path, &registry, &socket, max_bytes).await {
                    if error.is::<OutputError>() {
                        return Err(error);
                    }
                    eprintln!("codex-context degraded: {error:#}");
                    write_output(&protocol::degraded())?;
                }
            }
        }
        _ => anyhow::bail!("unknown codex-context action"),
    }
    Ok(true)
}

async fn run_hook(
    state_path: &Path,
    registry: &Path,
    socket: &Path,
    max_bytes: usize,
) -> Result<()> {
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((protocol::MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let input = HookInput::parse(&bytes)?;
    drop(bytes);
    let cwd = input.cwd.canonicalize().context("hook cwd unavailable")?;
    let state = State::open(state_path).await?;
    let _session_lock = super::state::lock_session(state_path, &input.session_id)?;
    let existing = state.has_binding(&input.session_id).await?;
    let (binding, minimum) = if existing {
        state.binding(&input.session_id).await?
    } else {
        ensure!(
            input.hook_event_name == "SessionStart" && input.source.as_deref() == Some("startup"),
            "unbound resume/fork/session requires explicit task selection"
        );
        (state.default_for(&input.session_id, &cwd).await?, 0)
    };
    ensure!(
        binding.workspace_root.canonicalize()? == binding.workspace_root,
        "bound root changed"
    );
    ensure!(
        cwd.starts_with(&binding.workspace_root),
        "hook cwd is outside bound workspace"
    );
    let grant = resolve_workspace(registry, &binding.workspace_root)?;
    ensure!(
        grant.workspace_id == binding.workspace_id && grant.project_namespace == binding.namespace,
        "workspace binding revoked or changed"
    );
    let core_budget = max_bytes.saturating_sub(1024) / 2;
    ensure!(
        core_budget >= 1024,
        "host byte budget too small for complete context envelope"
    );
    let mut context = fetch(socket, &binding, core_budget).await?;
    let version = protocol::context_version(&context, &binding)?;
    let core_context_id = context["context_id"].as_str().unwrap().to_owned();
    let core_digest = protocol::digest(&serde_json::to_vec(&context)?);
    // A known session must remember even a paused/incomplete newer revision.
    // Otherwise a later database rollback could resurrect an older active task.
    let early_receipt = if existing {
        Some(
            state
                .built(
                    &binding,
                    &core_context_id,
                    version,
                    &core_digest,
                    &input.hook_event_name,
                    input.source.as_deref(),
                )
                .await?,
        )
    } else {
        None
    };
    // Validate scope, task status, freshness and base budget before any artifact IO.
    protocol::render(&input, &binding, &context, max_bytes, minimum)?;
    let observations = super::artifacts::observe(&binding.workspace_root, &context);
    if !observations.is_empty() {
        context["adapter_artifact_observations"] = serde_json::to_value(observations)?;
    }
    let output = protocol::render(&input, &binding, &context, max_bytes, minimum)?;
    // A default only becomes a session binding after fresh context validation.
    if !existing {
        state.bind(&binding).await?;
    }
    let receipt = match early_receipt {
        Some(receipt) => receipt,
        None => {
            state
                .built(
                    &binding,
                    &core_context_id,
                    version,
                    &core_digest,
                    &input.hook_event_name,
                    input.source.as_deref(),
                )
                .await?
        }
    };
    write_output(&output).map_err(OutputError)?;
    // If persistence fails after stdout, never emit a second JSON document.
    if let Err(error) = state.output_written(&receipt).await {
        eprintln!("codex-context output written, receipt update unconfirmed: {error}");
    }
    Ok(())
}
async fn fetch(socket: &Path, binding: &Binding, max_bytes: usize) -> Result<Value> {
    let executable = std::env::current_exe()?;
    let socket = socket.to_owned();
    let binding = binding.clone();
    tokio::task::spawn_blocking(move || {
        transport::read_context(&executable, &socket, &binding, max_bytes)
    })
    .await?
}
#[derive(Debug, thiserror::Error)]
#[error("hook stdout write failed; delivery unconfirmed: {0}")]
struct OutputError(#[source] anyhow::Error);

fn write_output(value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.flush()?;
    Ok(())
}
fn parse_flags(args: &[String]) -> Result<BTreeMap<String, String>> {
    ensure!(args.len().is_multiple_of(2), "flags require values");
    let mut flags = BTreeMap::new();
    for pair in args.chunks_exact(2) {
        ensure!(
            pair[0].starts_with("--") && flags.insert(pair[0].clone(), pair[1].clone()).is_none(),
            "invalid or duplicate flag"
        );
    }
    Ok(flags)
}
fn take(flags: &mut BTreeMap<String, String>, key: &str) -> Result<String> {
    flags.remove(key).with_context(|| format!("missing {key}"))
}
fn absolute(value: String) -> Result<PathBuf> {
    let path = PathBuf::from(value);
    ensure!(path.is_absolute(), "absolute path required");
    Ok(path)
}

fn preview(
    executable: &Path,
    state: &Path,
    registry: &Path,
    socket: &Path,
    max_bytes: usize,
) -> Result<Value> {
    let args = vec![
        executable
            .to_str()
            .context("executable path is not UTF-8")?
            .to_owned(),
        "codex-context".into(),
        "hook".into(),
        "--state".into(),
        state.to_str().context("state path is not UTF-8")?.into(),
        "--registry".into(),
        registry
            .to_str()
            .context("registry path is not UTF-8")?
            .into(),
        "--socket".into(),
        socket.to_str().context("socket path is not UTF-8")?.into(),
        "--max-bytes".into(),
        max_bytes.to_string(),
    ];
    let posix = args
        .iter()
        .map(|a| format!("'{}'", a.replace('\'', "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");
    // Windows shell interpolation is not approximated: produce an explicit
    // unsupported placeholder rather than a command vulnerable to expansion.
    let mut handler =
        json!({"type":"command","command":posix,"additionalContextLimit":0,"timeout":8});
    if cfg!(windows) {
        handler["command"] = json!("WINDOWS_COMMAND_PREVIEW_UNSUPPORTED");
    }
    Ok(
        json!({"preview_only":true,"contract":protocol::CONTRACT,"max_context_bytes":max_bytes,"argv":args,"platform_preview_supported":!cfg!(windows),"delivery":"unverified","trust_review_required":true,"hooks_json":{"hooks":{"SessionStart":[{"matcher":"^(startup|resume|clear|compact|fork)$","hooks":[handler.clone()]}],"UserPromptSubmit":[{"hooks":[handler]}]}}}),
    )
}
fn capabilities() -> Result<Value> {
    // Only public --version, not configuration/auth inspection. Host event
    // payloads contain no authenticated version; this executable may not be the
    // invoking host. Never report automatic delivery from this probe alone.
    let version = installed_version();
    Ok(
        json!({"adapter_contract":protocol::CONTRACT,"installed_cli_version":version,"reference_schema_version":"codex-cli 0.159.2","installed_version_matches_reference":version.as_deref().map(|v|v=="codex-cli 0.159.2"),"invoking_host_version":"unverified","mode":"manual_until_host_delivery_verified","documented_sources":["startup","resume","clear","compact","fork"],"events":["SessionStart","UserPromptSubmit"],"actual_hook_execution":"unverified","host_delivery":"unverified","model_adoption":"unknown","execution_authorization":false,"transcript_access":false}),
    )
}

fn installed_version() -> Option<String> {
    use std::{
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::Duration,
    };
    let mut child = Command::new("codex")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(4097).read_to_end(&mut bytes).map(|_| bytes);
        let _ = tx.send(result);
    });
    let bytes = rx
        .recv_timeout(Duration::from_secs(2))
        .ok()
        .and_then(Result::ok);
    let status = child.try_wait().ok().flatten();
    let _ = child.kill();
    let _ = child.wait();
    let bytes = bytes.filter(|b| b.len() <= 4096)?;
    if !status.is_some_and(|s| s.success()) {
        return None;
    }
    String::from_utf8(bytes).ok().map(|s| s.trim().to_owned())
}
