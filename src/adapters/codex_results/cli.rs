use super::{
    protocol::{Input, MAX_INPUT, token},
    runtime::{self, Bridge},
    state::{Record, State},
};
use crate::{
    adapters::codex_context::{
        protocol::{Binding, digest},
        state as context_state,
    },
    support::workspace_registry::resolve_workspace,
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};
pub async fn try_run_cli(args: &[String]) -> Result<bool> {
    if args.first().map(String::as_str) != Some("codex-results") {
        return Ok(false);
    }
    let action = args.get(1).map(String::as_str).unwrap_or("help");
    // Never print arbitrary errors from filesystem, serde, SQLite or remote replies.
    let result = run(action, &args[2.min(args.len())..]).await;
    if action == "capture" || action == "link" {
        if result.is_err() {
            eprintln!(
                "codex-results: capture/link/delivery incomplete; inspect the metadata outbox; origin and outcome remain unverified"
            );
        }
        println!("{{}}");
        return Ok(true);
    }
    result.map_err(|_|anyhow::anyhow!("codex-results operation incomplete; check explicit binding, private state, scope and retry status"))?;
    Ok(true)
}
fn take(flags: &mut BTreeMap<String, String>, name: &str) -> Result<String> {
    flags.remove(name).context("missing required flag")
}
fn abs(s: String) -> Result<PathBuf> {
    let p = PathBuf::from(s);
    ensure!(p.is_absolute(), "absolute path required");
    Ok(p)
}
fn parse(args: &[String]) -> Result<BTreeMap<String, String>> {
    ensure!(args.len().is_multiple_of(2), "flags need values");
    let mut f = BTreeMap::new();
    for pair in args.chunks(2) {
        ensure!(
            pair[0].starts_with("--") && f.insert(pair[0].clone(), pair[1].clone()).is_none(),
            "invalid flags"
        );
    }
    Ok(f)
}
fn read(event: &str) -> Result<Input> {
    let mut b = Vec::new();
    std::io::stdin()
        .take((MAX_INPUT + 1) as u64)
        .read_to_end(&mut b)?;
    Input::parse(&b, event)
}
fn summary(r: &Record) -> Value {
    json!({"id":r.link.key(),"session_id":r.link.binding.host_session_id,"tool_use_id":r.link.tool_id,"linked_use_id":r.link.use_id,"task_version":r.link.task_version,"candidate_version":r.link.candidate_version,"captured":r.observation.is_some(),"event_id":r.event_id,"use_attempt":r.use_attempt,"use_request_prepared":r.use_request.is_some(),"confirmed_event_rejected":r.event_rejected,"confirmed_rejected":r.rejected,"completed":r.completed,"origin":"unverified","outcome":"unknown","independently_verified":false,"execution_permission":false})
}
async fn verify(
    state: &Path,
    registry: &Path,
    session: &str,
    cwd: Option<&Path>,
) -> Result<(Binding, u64)> {
    let (b, minimum) = context_state::State::open(state)
        .await?
        .binding(session)
        .await?;
    ensure!(
        b.workspace_root.canonicalize()? == b.workspace_root,
        "root changed"
    );
    if let Some(cwd) = cwd {
        ensure!(
            cwd.canonicalize()?.starts_with(&b.workspace_root),
            "cwd outside binding"
        );
    }
    let g = resolve_workspace(registry, &b.workspace_root)?;
    ensure!(
        g.workspace_id == b.workspace_id && g.project_namespace == b.namespace,
        "binding revoked or changed"
    );
    Ok((b, minimum))
}
async fn run(action: &str, args: &[String]) -> Result<()> {
    if action == "help" {
        println!(
            "codex-results init --outbox ABS\ncodex-results preview --outbox ABS --state ABS --registry ABS --socket ABS --use-id ID\ncodex-results link (same flags; explicit PreToolUse JSON on stdin)\ncodex-results capture --outbox ABS --state ABS --registry ABS --socket ABS (PostToolUse JSON on stdin)\ncodex-results inspect --outbox ABS --session-id ID --tool-use-id ID\ncodex-results retry --outbox ABS --state ABS --registry ABS --socket ABS --session-id ID --tool-use-id ID [--retry-rejected true]\nPreview only: patch matcher, POSIX command shell. No installation or permissions."
        );
        return Ok(());
    }
    let mut flags = parse(args)?;
    let outbox = abs(take(&mut flags, "--outbox")?)?;
    if action == "init" {
        ensure!(flags.is_empty(), "unexpected flag");
        State::initialize(&outbox).await?;
        println!(
            "{}",
            json!({"initialized":true,"schema_version":1,"credentials_created":false,"installed":false})
        );
        return Ok(());
    }
    if action == "inspect" {
        let session = take(&mut flags, "--session-id")?;
        let tool = take(&mut flags, "--tool-use-id")?;
        token(&session)?;
        token(&tool)?;
        ensure!(flags.is_empty(), "unexpected flag");
        let id = digest(&serde_json::to_vec(&(&session, &tool))?);
        println!("{}", summary(&State::open(&outbox).await?.load(&id).await?));
        return Ok(());
    }
    let state = abs(take(&mut flags, "--state")?)?;
    let registry = abs(take(&mut flags, "--registry")?)?;
    let socket = abs(take(&mut flags, "--socket")?)?;
    let core = Bridge {
        executable: std::env::current_exe()?,
        socket: socket.clone(),
    };
    if action == "preview" {
        let use_id = take(&mut flags, "--use-id")?;
        token(&use_id)?;
        ensure!(flags.is_empty(), "unexpected flag");
        ensure!(!cfg!(windows), "Windows command preview unsupported");
        let base = format!("{} codex-results", quote(&core.executable)?);
        let common = format!(
            "--outbox {} --state {} --registry {} --socket {}",
            quote(&outbox)?,
            quote(&state)?,
            quote(&registry)?,
            quote(&socket)?
        );
        println!(
            "{}",
            json!({"candidate_only":true,"installed":false,"host_version":"0.159.2","host_delivery":"unverified","matcher_scope":"apply_patch only","requires_explicit_current_use":use_id,"execution_permission":false,"hooks":{"PreToolUse":[{"matcher":"^apply_patch$","hooks":[{"type":"command","command":format!("{base} link {common} --use-id '{use_id}'"),"timeout":30}]}],"PostToolUse":[{"matcher":"^apply_patch$","hooks":[{"type":"command","command":format!("{base} capture {common}"),"timeout":30}]}]}})
        );
        return Ok(());
    }
    let store = State::open(&outbox).await?;
    match action {
        "link" | "capture" => {
            let use_id = if action == "link" {
                Some(take(&mut flags, "--use-id")?)
            } else {
                None
            };
            ensure!(flags.is_empty(), "unexpected flag");
            let input = read(if action == "link" {
                "PreToolUse"
            } else {
                "PostToolUse"
            })?;
            let _lock = context_state::lock_session(&outbox, &input.session_id)?;
            let (binding, minimum) =
                verify(&state, &registry, &input.session_id, Some(&input.cwd)).await?;
            if let Some(use_id) = use_id {
                let link = runtime::make_link(&core, binding, &input, &use_id).await?;
                ensure!(
                    link.task_version as u64 >= minimum,
                    "task older than session watermark"
                );
                store.link(link).await?;
            } else {
                let id = digest(&serde_json::to_vec(&(
                    &input.session_id,
                    &input.tool_use_id,
                ))?);
                let r = store.load(&id).await?;
                ensure!(r.link.binding == binding, "binding changed");
                let r = runtime::capture(&store, r, &input).await?;
                runtime::deliver(&core, &store, r, false).await?;
            }
        }
        "retry" => {
            let session = take(&mut flags, "--session-id")?;
            let tool = take(&mut flags, "--tool-use-id")?;
            token(&session)?;
            token(&tool)?;
            let retry = flags.remove("--retry-rejected");
            ensure!(
                retry.is_none() || retry.as_deref() == Some("true"),
                "invalid retry flag"
            );
            ensure!(flags.is_empty(), "unexpected flag");
            let _lock = context_state::lock_session(&outbox, &session)?;
            let (binding, _) = verify(&state, &registry, &session, None).await?;
            let id = digest(&serde_json::to_vec(&(&session, &tool))?);
            let r = store.load(&id).await?;
            ensure!(r.link.binding == binding, "binding changed");
            let r = runtime::deliver(&core, &store, r, retry.is_some()).await?;
            println!("{}", summary(&r));
        }
        _ => anyhow::bail!("unsupported command"),
    }
    Ok(())
}
fn quote(p: &Path) -> Result<String> {
    let s = p.to_str().context("non UTF8 command path")?;
    ensure!(!s.chars().any(char::is_control), "control in command path");
    Ok(format!("'{}'", s.replace('\'', "'\\''")))
}
