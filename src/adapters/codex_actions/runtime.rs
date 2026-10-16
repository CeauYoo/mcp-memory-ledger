//! I/O composition only. Double reads detect some changes, never an atomic lease
//! on future tool execution. A hook that never runs cannot deny anything.
use super::{
    patch,
    protocol::{self, ActionInput},
    receipts,
};
use crate::{
    adapters::codex_context::{protocol::Binding, state, transport},
    domain::action_check::{self, Decision, Operation, Reason, TaskObservation},
    support::workspace_registry::resolve_workspace,
};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::path::{Path, PathBuf};
pub struct Paths {
    pub state: PathBuf,
    pub registry: PathBuf,
    pub socket: PathBuf,
    pub policy: PathBuf,
    pub receipts: PathBuf,
}
pub async fn binding(paths: &Paths, session: &str) -> Result<(Binding, u64)> {
    let state = state::State::open(&paths.state).await?;
    let (binding, minimum) = state.binding(session).await?;
    let grant = resolve_workspace(&paths.registry, &binding.workspace_root)?;
    ensure!(
        grant.workspace_id == binding.workspace_id && grant.project_namespace == binding.namespace,
        "binding revoked or changed"
    );
    Ok((binding, minimum))
}
pub async fn fetch(socket: &Path, binding: &Binding) -> Result<Value> {
    let exe = std::env::current_exe()?;
    let socket = socket.to_owned();
    let binding = binding.clone();
    tokio::task::spawn_blocking(move || transport::read_context(&exe, &socket, &binding, 32768))
        .await?
}
pub async fn check(paths: &Paths, input: &ActionInput) -> Result<Decision> {
    let (policy, digest) = protocol::load_policy(&paths.policy)?;
    let audit = receipts::State::open(&paths.receipts).await?;
    // Persist a newly observed revocation even if task/root/core checks fail.
    audit
        .observe_policy(&policy.policy_id, policy.epoch, &digest)
        .await?;
    ensure!(
        policy.canonical_root.canonicalize()? == policy.canonical_root,
        "policy root changed"
    );
    let _lock = state::lock_session(&paths.state, &input.session_id)?;
    let (binding, minimum) = binding(paths, &input.session_id).await?;
    let cwd = input.cwd.canonicalize()?;
    ensure!(
        cwd == input.cwd && cwd.starts_with(&binding.workspace_root),
        "cwd identity changed or outside workspace"
    );
    // The source of a policy and task are deliberately separate. A task's source
    // field is never inspected here as authority.
    let context = fetch(&paths.socket, &binding).await?;
    let mut version =
        crate::adapters::codex_context::protocol::context_version(&context, &binding)?;
    // Keep even a newer paused/incomplete observation before later validation
    // fails. This provisional denial is replaced only after all checks succeed.
    audit
        .observe(
            &input.session_id,
            &input.tool_use_id,
            &input.args_digest()?,
            &policy.policy_id,
            policy.epoch,
            &digest,
            version,
            "deny",
        )
        .await?;
    ensure!(
        version >= minimum,
        "task snapshot older than session observation"
    );
    ensure!(
        context["mandatory_snapshot_consistent"] == true,
        "task snapshot inconsistent"
    );
    let active = context["status"] == "active"
        && context["can_resume"] == true
        && context["complete"] == true;
    let observation = TaskObservation {
        workspace_id: &binding.workspace_id,
        namespace: &binding.namespace,
        task_id: &binding.task_id,
        root: &binding.workspace_root,
        version,
        active,
    };
    let parsed = if input.tool_name == "apply_patch" {
        input.patch_command().and_then(patch::parse_targets)
    } else {
        Ok(Vec::new())
    };
    let ops: Vec<_> = parsed
        .as_ref()
        .map(|targets| {
            targets
                .iter()
                .map(|t| match t.operation {
                    patch::PatchOperation::Add => Operation::Add,
                    patch::PatchOperation::Update => Operation::Update,
                    patch::PatchOperation::Delete => Operation::Delete,
                    patch::PatchOperation::Move => Operation::Move,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut decision = action_check::evaluate(
        &policy,
        &observation,
        &input.tool_name,
        &ops,
        chrono::Utc::now().timestamp(),
    );
    if decision == Decision::PassThrough {
        let mut forbidden = policy.forbidden_roots.clone();
        forbidden.extend([
            paths.policy.clone(),
            paths.state.clone(),
            paths.registry.clone(),
            paths.receipts.clone(),
            std::env::current_exe()?,
        ]);
        if patch::resolve_targets(
            parsed.as_ref().unwrap(),
            &cwd,
            &binding.workspace_root,
            &forbidden,
        )
        .is_err()
        {
            decision = Decision::Deny(Reason::PathRejected);
        }
    }
    // Re-read without asking the operator on ordinary task revisions. The
    // trusted operation/path policy is unchanged; task text never adds powers.
    if decision == Decision::PassThrough {
        let after = fetch(&paths.socket, &binding).await?;
        let after_version =
            crate::adapters::codex_context::protocol::context_version(&after, &binding)?;
        if after_version < version {
            decision = Decision::Deny(Reason::SnapshotChanged);
        } else {
            version = after_version;
            audit
                .observe(
                    &input.session_id,
                    &input.tool_use_id,
                    &input.args_digest()?,
                    &policy.policy_id,
                    policy.epoch,
                    &digest,
                    version,
                    "deny",
                )
                .await?;
            let latest = TaskObservation {
                version,
                active: after["status"] == "active"
                    && after["can_resume"] == true
                    && after["complete"] == true
                    && after["mandatory_snapshot_consistent"] == true,
                ..observation
            };
            decision = action_check::evaluate(
                &policy,
                &latest,
                &input.tool_name,
                &ops,
                chrono::Utc::now().timestamp(),
            );
        }
        let (after_binding, _) = self::binding(paths, &input.session_id).await?;
        ensure!(after_binding == binding, "binding changed during check");
    }
    // Observe the final policy epoch even when it caused denial, so later
    // rollback cannot resurrect an earlier accepted configuration.
    let (policy_after, digest_after) = protocol::load_policy(&paths.policy)?;
    audit
        .observe_policy(&policy_after.policy_id, policy_after.epoch, &digest_after)
        .await?;
    if digest_after != digest || policy_after.epoch != policy.epoch {
        decision = Decision::Deny(Reason::SnapshotChanged);
    }
    audit
        .observe(
            &input.session_id,
            &input.tool_use_id,
            &input.args_digest()?,
            &policy_after.policy_id,
            policy_after.epoch,
            &digest_after,
            version,
            decision.label(),
        )
        .await
        .context("receipt conflict or unavailable")?;
    Ok(decision)
}
