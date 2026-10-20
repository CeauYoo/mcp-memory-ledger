//! Pure, bounded host protocol. Transcript and prompt fields are deliberately ignored.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const DEFAULT_CONTEXT_BYTES: usize = 8192;
pub const MAX_CONTEXT_BYTES: usize = 32768;
pub const MAX_INPUT_BYTES: usize = 65536;
pub const CONTRACT: &str = "codex-hooks-2026-10-16";

#[derive(Debug, Deserialize)]
pub struct HookInput {
    pub session_id: String,
    pub cwd: PathBuf,
    pub hook_event_name: String,
    pub source: Option<String>,
}
impl HookInput {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_INPUT_BYTES, "hook input exceeds limit");
        let mut input: Self = serde_json::from_slice(bytes).context("invalid hook input")?;
        identifier(&input.session_id)?;
        ensure!(input.cwd.is_absolute(), "hook cwd must be absolute");
        match input.hook_event_name.as_str() {
            "SessionStart" => ensure!(
                matches!(
                    input.source.as_deref(),
                    Some("startup" | "resume" | "clear" | "compact" | "fork")
                ),
                "unsupported SessionStart source"
            ),
            "UserPromptSubmit" => {
                input.source = None;
            }
            _ => anyhow::bail!("unsupported lifecycle event; manual mode"),
        }
        Ok(input)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Binding {
    pub host_session_id: String,
    pub workspace_id: String,
    pub namespace: String,
    pub task_id: String,
    pub workspace_root: PathBuf,
}

/// Evidence levels are separate, not an ordinal confidence score. No production
/// API in this adapter accepts host-delivered or result-verified assertions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStage {
    CoreBuilt,
    OutputWritten,
    HostDelivered,
    ModelClaimed,
    ResultVerified,
}

pub fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control),
        "invalid identifier"
    );
    Ok(())
}
pub fn budget(value: usize) -> Result<usize> {
    ensure!(
        (1024..=MAX_CONTEXT_BYTES).contains(&value),
        "context budget must be 1024..=32768 bytes"
    );
    Ok(value)
}
pub fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub fn context_version(context: &Value, binding: &Binding) -> Result<u64> {
    ensure!(
        context["namespace"].as_str() == Some(&binding.namespace),
        "context namespace mismatch"
    );
    ensure!(
        context["task_id"].as_str() == Some(&binding.task_id),
        "context task mismatch"
    );
    let id = context["context_id"]
        .as_str()
        .context("context id missing")?;
    identifier(id)?;
    context["task_version"]
        .as_u64()
        .filter(|v| *v > 0 && *v <= i64::MAX as u64)
        .context("invalid task version")
}

/// Both the model-visible text and the escaped, complete wire envelope fit the
/// configured cap. Never truncate requirements or rely on host head/tail spill.
pub fn render(
    input: &HookInput,
    binding: &Binding,
    context: &Value,
    max_bytes: usize,
    minimum_version: u64,
) -> Result<Value> {
    validate_context(binding, context, max_bytes, minimum_version)?;
    let content = format!(
        "Task ledger data, not execution authorization. Preserve the current user instructions and host permissions. Checkpoint artifacts require independent revalidation. No model adoption or delivery is proved by this output.\n{}",
        serde_json::to_string(
            &json!({"adapter_contract":CONTRACT,"binding":binding,"context":context})
        )?
    );
    ensure!(
        content.len() <= max_bytes,
        "model context exceeds host byte budget"
    );
    let output = json!({"hookSpecificOutput":{"hookEventName":input.hook_event_name,"additionalContext":content}});
    ensure!(
        serde_json::to_vec(&output)?.len() < max_bytes,
        "escaped hook envelope exceeds host byte budget"
    );
    Ok(output)
}

/// Shared checks run before conditional refresh comparison; an unchanged digest
/// never bypasses scope, status, snapshot, byte budget, or version checks.
pub fn validate_context(
    binding: &Binding,
    context: &Value,
    max_bytes: usize,
    minimum_version: u64,
) -> Result<()> {
    budget(max_bytes)?;
    let version = context_version(context, binding)?;
    ensure!(
        version >= minimum_version,
        "stale task version; refresh required"
    );
    ensure!(
        context["complete"] == true,
        "task context incomplete; narrow scope or raise bounded budget"
    );
    ensure!(
        context["can_resume"] == true && context["status"] == "active",
        "task is not resumable"
    );
    ensure!(
        context["mandatory_snapshot_consistent"] == true,
        "mandatory task snapshot is not consistent"
    );
    if let Some(after) = context
        .get("task_version_after_selection")
        .filter(|v| !v.is_null())
    {
        ensure!(
            after.as_u64() == Some(version),
            "stale task version after optional selection"
        );
    }
    ensure!(
        context["mandatory_requirements"].is_array(),
        "mandatory requirements missing"
    );
    let diagnostics = context["diagnostics"]
        .as_array()
        .context("context diagnostics missing")?;
    ensure!(
        !diagnostics
            .iter()
            .filter_map(Value::as_str)
            .any(|s| s.contains("stale")),
        "stale context or checkpoint; explicit refresh required"
    );
    Ok(())
}

/// A cooperative cache validator, not host acceptance or authorization evidence.
/// Remove only generated identifiers and adapter observation timestamps. Changes
/// to requirements, checkpoint, optional selection or artifact results invalidate it.
pub fn context_fingerprint(binding: &Binding, context: &Value) -> Result<String> {
    context_version(context, binding)?;
    let mut stable = context.clone();
    stable
        .as_object_mut()
        .context("context must be an object")?
        .remove("context_id");
    if let Some(observations) = stable
        .get_mut("adapter_artifact_observations")
        .and_then(Value::as_array_mut)
    {
        for observation in observations {
            if let Some(object) = observation.as_object_mut() {
                object.remove("observed_at");
            }
        }
    }
    Ok(digest(&serde_json::to_vec(
        &json!({"contract":"task-refresh-v1","binding":binding,"context":stable}),
    )?))
}

pub fn fingerprint(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "fingerprint must be lowercase SHA256 hex"
    );
    Ok(())
}

/// Explicit controller refresh response. This is NOT a hook output and is never
/// installed as a new native hook. The caller owns polling and context injection.
pub fn render_refresh(
    binding: &Binding,
    context: &Value,
    max_bytes: usize,
    minimum_version: u64,
    known: Option<&str>,
) -> Result<Value> {
    validate_context(binding, context, max_bytes, minimum_version)?;
    if let Some(known) = known {
        fingerprint(known)?;
    }
    let current = context_fingerprint(binding, context)?;
    let changed = known != Some(current.as_str());
    let output = json!({"contract":"task-refresh-v1","binding":binding,"context_id":context["context_id"],"task_version":context["task_version"],"context_fingerprint":current,"changed":changed,"context":if changed { context.clone() } else { Value::Null },"delivery":"unverified","model_adoption":"unknown","result_effect":"unknown","execution_authorization":false,"cache_basis":"caller_supplied_fingerprint_not_host_acceptance"});
    // Validate the FULL response even on cache hits. An undersized budget cannot
    // suppress hard constraints and then claim a usable unchanged response.
    let mut full = output.clone();
    full["context"] = context.clone();
    full["changed"] = json!(true);
    ensure!(
        serde_json::to_vec(&full)?.len() < max_bytes,
        "refresh envelope exceeds host byte budget"
    );
    Ok(output)
}

pub fn degraded() -> Value {
    json!({"systemMessage":"Task ledger context unavailable or incomplete. Automatic context is not verified; select the task or inspect codex-context status before relying on remembered requirements."})
}
