//! Additional, restrictive checks. Neither task data nor these decisions grant
//! execution authority. Same-OS-user configuration tampering is out of scope.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const CONTRACT: &str = "codex-actions-2026-10-16-v1";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorGate {
    Enabled,
    Paused,
    Revoked,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckMode {
    PatchOnly,
    StrictLocal,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskVersionMode {
    #[default]
    LatestActive,
    Pinned,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Add,
    Update,
    Delete,
    Move,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorPolicy {
    pub schema_version: u32,
    pub contract: String,
    pub policy_id: String,
    pub epoch: u64,
    pub workspace_id: String,
    pub namespace: String,
    pub canonical_root: PathBuf,
    pub task_id: String,
    /// Minimum version in latest_active mode; exact version only in pinned mode.
    pub expected_task_version: u64,
    #[serde(default)]
    pub task_version_mode: TaskVersionMode,
    pub gate: OperatorGate,
    pub mode: CheckMode,
    pub allowed_operations: Vec<Operation>,
    #[serde(default)]
    pub forbidden_roots: Vec<PathBuf>,
    #[serde(default)]
    pub expires_at: Option<i64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    PassThrough,
    Deny(Reason),
    NeedsConfirmation(Reason),
    Unsupported(Reason),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    InvalidPolicy,
    BindingMismatch,
    OperatorPaused,
    OperatorRevoked,
    PolicyExpired,
    TaskInactive,
    TaskVersionChanged,
    StaleTask,
    UnclassifiedTool,
    OperationNotConfigured,
    InvalidInput,
    CheckUnavailable,
    PathRejected,
    SnapshotChanged,
    ReceiptUnavailable,
}
impl Decision {
    pub fn label(self) -> &'static str {
        match self {
            Self::PassThrough => "pass_through",
            Self::Deny(_) => "deny",
            Self::NeedsConfirmation(_) => "needs_confirmation",
            Self::Unsupported(_) => "unsupported",
        }
    }
    pub fn reason(self) -> Option<Reason> {
        match self {
            Self::PassThrough => None,
            Self::Deny(r) | Self::NeedsConfirmation(r) | Self::Unsupported(r) => Some(r),
        }
    }
}
impl Reason {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidPolicy => "Operator policy is invalid; review local configuration.",
            Self::BindingMismatch => {
                "Workspace or task binding changed; select and review the task."
            }
            Self::OperatorPaused => "Operator policy is paused; task memory cannot resume it.",
            Self::OperatorRevoked => "Operator policy is revoked; review a new policy revision.",
            Self::PolicyExpired => "Operator policy expired; review configuration before retrying.",
            Self::TaskInactive => "Task is not active; no new task action was passed through.",
            Self::TaskVersionChanged => {
                "Pinned task version changed; review the explicitly strict policy binding."
            }
            Self::StaleTask => {
                "Task snapshot is older than the configured baseline; refresh and retry."
            }
            Self::UnclassifiedTool => {
                "This tool is unsupported by scoped action checks; no execution approval was issued."
            }
            Self::OperationNotConfigured => {
                "This patch operation needs operator review; model approval fields cannot authorize it."
            }
            Self::InvalidInput => "Hook input or patch format is unsupported or invalid.",
            Self::CheckUnavailable => {
                "Action check unavailable; current call denied if this hook is running. Host bypass paths remain uncovered."
            }
            Self::PathRejected => "A patch target could not be verified within configured paths.",
            Self::SnapshotChanged => {
                "Task or operator policy changed during checking; refresh before retrying."
            }
            Self::ReceiptUnavailable => {
                "Action observation could not be recorded or conflicts with an earlier event; current call denied."
            }
        }
    }
}
pub struct TaskObservation<'a> {
    pub workspace_id: &'a str,
    pub namespace: &'a str,
    pub task_id: &'a str,
    pub root: &'a std::path::Path,
    pub version: u64,
    pub active: bool,
}
pub fn validate_policy(p: &OperatorPolicy) -> bool {
    let id = |s: &str| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control);
    p.schema_version == 1
        && p.contract == CONTRACT
        && id(&p.policy_id)
        && id(&p.workspace_id)
        && id(&p.namespace)
        && id(&p.task_id)
        && p.epoch > 0
        && p.epoch <= i64::MAX as u64
        && p.expected_task_version > 0
        && p.expected_task_version <= i64::MAX as u64
        && p.canonical_root.is_absolute()
        && p.allowed_operations.len() <= 4
        && p.forbidden_roots.len() <= 64
        && p.forbidden_roots.iter().all(|v| v.is_absolute())
        && p.allowed_operations
            .iter()
            .enumerate()
            .all(|(i, v)| !p.allowed_operations[..i].contains(v))
}
/// Evaluate observed state only; source labels and approval claims are absent.
pub fn evaluate(
    p: &OperatorPolicy,
    t: &TaskObservation<'_>,
    tool: &str,
    ops: &[Operation],
    now: i64,
) -> Decision {
    use Decision::*;
    use Reason::*;
    if !validate_policy(p) {
        return Deny(InvalidPolicy);
    }
    if p.workspace_id != t.workspace_id
        || p.namespace != t.namespace
        || p.task_id != t.task_id
        || p.canonical_root != t.root
    {
        return Deny(BindingMismatch);
    }
    match p.gate {
        OperatorGate::Paused => return Deny(OperatorPaused),
        OperatorGate::Revoked => return Deny(OperatorRevoked),
        OperatorGate::Enabled => {}
    }
    if p.expires_at.is_some_and(|expiry| now >= expiry) {
        return Deny(PolicyExpired);
    }
    if !t.active {
        return Deny(TaskInactive);
    }
    if t.version < p.expected_task_version {
        return Deny(StaleTask);
    }
    if p.task_version_mode == TaskVersionMode::Pinned && p.expected_task_version != t.version {
        return NeedsConfirmation(TaskVersionChanged);
    }
    if tool != "apply_patch" {
        return Unsupported(UnclassifiedTool);
    }
    if ops.is_empty() {
        return Unsupported(InvalidInput);
    }
    if ops.iter().any(|op| !p.allowed_operations.contains(op)) {
        return NeedsConfirmation(OperationNotConfigured);
    }
    PassThrough
}
#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> OperatorPolicy {
        OperatorPolicy {
            schema_version: 1,
            contract: CONTRACT.into(),
            policy_id: "policy".into(),
            epoch: 7,
            workspace_id: "workspace".into(),
            namespace: "project/a".into(),
            canonical_root: std::env::temp_dir(),
            task_id: "task".into(),
            expected_task_version: 3,
            task_version_mode: TaskVersionMode::Pinned,
            gate: OperatorGate::Enabled,
            mode: CheckMode::PatchOnly,
            allowed_operations: vec![Operation::Update],
            forbidden_roots: vec![],
            expires_at: None,
        }
    }
    #[test]
    fn versions_and_gates_are_independent() {
        let mut p = policy();
        let root = p.canonical_root.clone();
        let mut t = TaskObservation {
            workspace_id: "workspace",
            namespace: "project/a",
            task_id: "task",
            root: &root,
            version: 3,
            active: true,
        };
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::PassThrough
        );
        t.version = 7;
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::NeedsConfirmation(Reason::TaskVersionChanged)
        );
        t.version = 3;
        p.gate = OperatorGate::Paused;
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::Deny(Reason::OperatorPaused)
        );
        p.gate = OperatorGate::Revoked;
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::Deny(Reason::OperatorRevoked)
        );
    }
    #[test]
    fn shell_unknown_scope_expiry_and_ops_never_pass() {
        let mut p = policy();
        let root = p.canonical_root.clone();
        let mut t = TaskObservation {
            workspace_id: "workspace",
            namespace: "project/a",
            task_id: "task",
            root: &root,
            version: 3,
            active: true,
        };
        assert_eq!(
            evaluate(&p, &t, "Bash", &[Operation::Update], 0),
            Decision::Unsupported(Reason::UnclassifiedTool)
        );
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Delete], 0),
            Decision::NeedsConfirmation(Reason::OperationNotConfigured)
        );
        t.active = false;
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::Deny(Reason::TaskInactive)
        );
        t.active = true;
        p.expires_at = Some(2);
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 2),
            Decision::Deny(Reason::PolicyExpired)
        );
        t.workspace_id = "other";
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::Deny(Reason::BindingMismatch)
        );
    }
    #[test]
    fn ordinary_task_revisions_refresh_without_permission_changes() {
        let mut p = policy();
        p.task_version_mode = TaskVersionMode::LatestActive;
        let root = p.canonical_root.clone();
        let mut t = TaskObservation {
            workspace_id: "workspace",
            namespace: "project/a",
            task_id: "task",
            root: &root,
            version: 99,
            active: true,
        };
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::PassThrough
        );
        assert_eq!(p.epoch, 7);
        assert_eq!(p.allowed_operations, vec![Operation::Update]);
        t.version = 2;
        assert_eq!(
            evaluate(&p, &t, "apply_patch", &[Operation::Update], 0),
            Decision::Deny(Reason::StaleTask)
        );
    }
    #[test]
    fn approval_and_source_fields_cannot_create_authority() {
        let mut v = serde_json::to_value(policy()).unwrap();
        v["approval"] = serde_json::json!(true);
        assert!(serde_json::from_value::<OperatorPolicy>(v).is_err());
        let mut v = serde_json::to_value(policy()).unwrap();
        v["source"] = serde_json::json!("user");
        assert!(serde_json::from_value::<OperatorPolicy>(v).is_err());
    }
}
