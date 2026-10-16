use crate::domain::action_check::{Decision, OperatorPolicy, Reason, validate_policy};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
pub const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
#[derive(Debug, Deserialize)]
pub struct ActionInput {
    pub session_id: String,
    pub cwd: PathBuf,
    pub hook_event_name: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: Value,
}
impl ActionInput {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= MAX_INPUT_BYTES, "hook input too large");
        let input: Self = serde_json::from_slice(bytes)?;
        for value in [&input.session_id, &input.tool_name, &input.tool_use_id] {
            super::super::codex_context::protocol::identifier(value)?;
        }
        ensure!(
            input.hook_event_name == "PreToolUse" && input.cwd.is_absolute(),
            "unsupported hook event or cwd"
        );
        Ok(input)
    }
    pub fn args_digest(&self) -> Result<String> {
        Ok(super::super::codex_context::protocol::digest(
            &serde_json::to_vec(
                &json!({"tool":self.tool_name,"cwd":self.cwd,"input":self.tool_input}),
            )?,
        ))
    }
    pub fn patch_command(&self) -> Result<&str> {
        let obj = self
            .tool_input
            .as_object()
            .context("patch input must be object")?;
        ensure!(obj.len() == 1, "unknown patch input fields");
        obj.get("command")
            .and_then(Value::as_str)
            .context("patch command missing")
    }
}
pub fn render(decision: Decision) -> Value {
    match decision.reason() {
        None => json!({}),
        Some(reason) => {
            json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":reason.message()}})
        }
    }
}
pub fn unavailable() -> Value {
    render(Decision::Deny(Reason::CheckUnavailable))
}
/// Reading a private local file does not authenticate a human author. No model
/// API writes this file; OS isolation remains the operator's responsibility.
pub fn load_policy(path: &Path) -> Result<(OperatorPolicy, String)> {
    ensure!(path.is_absolute(), "policy path must be absolute");
    let parent = path.parent().context("policy parent missing")?;
    for ancestor in parent.ancestors() {
        let m = fs::symlink_metadata(ancestor)?;
        ensure!(
            m.is_dir() && !m.file_type().is_symlink(),
            "policy ancestors must not be links"
        );
    }
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 65536,
        "invalid policy file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // SAFETY: geteuid takes no arguments and has no memory preconditions.
        let uid = unsafe { libc::geteuid() };
        ensure!(
            meta.nlink() == 1 && meta.uid() == uid && meta.permissions().mode() & 0o077 == 0,
            "policy must be private current-user file"
        );
        let p = fs::metadata(parent)?;
        ensure!(
            p.uid() == uid && p.permissions().mode() & 0o077 == 0,
            "policy directory must be private current-user directory"
        );
    }
    #[cfg(windows)]
    crate::support::windows_security::validate_private_file(path)?;
    #[cfg(not(any(unix, windows)))]
    anyhow::bail!("private policy unsupported on platform");
    let mut bytes = Vec::new();
    fs::File::open(path)?.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "policy exceeds limit");
    let policy: OperatorPolicy = serde_json::from_slice(&bytes)?;
    ensure!(validate_policy(&policy), "invalid operator policy");
    ensure!(
        !path.starts_with(&policy.canonical_root),
        "policy must be outside checked workspace"
    );
    Ok((
        policy,
        super::super::codex_context::protocol::digest(&bytes),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outputs_never_issue_allow_or_unsupported_ask() {
        assert_eq!(render(Decision::PassThrough), json!({}));
        for d in [
            Decision::Deny(Reason::CheckUnavailable),
            Decision::NeedsConfirmation(Reason::TaskVersionChanged),
            Decision::Unsupported(Reason::UnclassifiedTool),
        ] {
            let v = render(d);
            assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "deny");
            assert!(v.get("continue").is_none());
            assert!(v["hookSpecificOutput"].get("updatedInput").is_none());
        }
    }
    #[test]
    fn patch_cannot_smuggle_approval_and_event_must_match() {
        let mut v = json!({"session_id":"s","cwd":std::env::temp_dir(),"hook_event_name":"PreToolUse","tool_name":"apply_patch","tool_use_id":"t","tool_input":{"command":"x","approval":true}});
        let input = ActionInput::parse(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(input.patch_command().is_err());
        v["hook_event_name"] = json!("PostToolUse");
        assert!(ActionInput::parse(&serde_json::to_vec(&v).unwrap()).is_err());
    }
}
