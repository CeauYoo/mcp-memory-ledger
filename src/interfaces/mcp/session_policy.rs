//! Pure authorization for a session bound by the local host, never by tool arguments.
//!
//! This layer runs before DTO decoding, logging request bodies, or touching storage.
//! It selects exactly one granted scope and materializes every legacy scope default.
//! Store/application checks must still validate referenced records in that scope.
//! The session runtime must independently disable automatic model reflection.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    domain::types::{Namespace, Owner},
    support::workspace_registry::SessionGrant,
};

/// The operation's memory access, suitable for safe monitoring attribution.
/// Read operations may still produce host audit metadata; export remains exempt
/// from persisted request logging under its existing read-only contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAccess {
    Read,
    Write,
}

impl ToolAccess {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthorizedCall {
    pub namespace: String,
    pub access: ToolAccess,
    /// Normalized arguments for dispatch, not for unrestricted audit logging.
    pub args: Map<String, Value>,
}

/// Stable, content-free failures. No rejected scope, request body, or grant list
/// is exposed through Display/Debug or serialized into a public error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PolicyError {
    #[error("tool is not available to this session")]
    ToolNotAllowed,
    #[error("invalid session tool arguments")]
    InvalidArguments,
    #[error("memory scope is not permitted for this session operation")]
    ScopeDenied,
    #[error("one tool call must stay within one memory scope")]
    MixedScope,
    #[error("record owner must match the permitted memory scope")]
    OwnerMismatch,
    #[error("automatic reflection is not available to this session")]
    AutomaticReflectionDisabled,
    #[error("legacy episode links are unavailable; use scoped record_episode")]
    LegacyEpisodeLinkNotAllowed,
}

const ALLOWED_TOOL_NAMES: &[&str] = &[
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

/// Global snapshot, decision, reflection/rollback, self-model history/version,
/// and retrieval-index administration are deliberately absent. New tools remain
/// denied until their access and complete argument contract are reviewed here.
pub fn allowed_tool_names() -> &'static [&'static str] {
    ALLOWED_TOOL_NAMES
}

/// Safe access metadata for known tools, including calls denied before dispatch.
pub fn tool_access(name: &str) -> Option<ToolAccess> {
    rule(name).ok().map(|rule| rule.access)
}

struct ToolRule {
    access: ToolAccess,
    fields: &'static [&'static str],
}

fn rule(name: &str) -> Result<ToolRule, PolicyError> {
    use ToolAccess::{Read, Write};
    let (access, fields): (_, &[&str]) = match name {
        "create_task" => (
            Write,
            &[
                "namespace",
                "task_id",
                "request_id",
                "title",
                "requirements",
                "source",
                "reason",
            ],
        ),
        "revise_task" => (
            Write,
            &[
                "namespace",
                "task_id",
                "request_id",
                "expected_version",
                "changes",
                "status",
                "source",
                "reason",
            ],
        ),
        "get_task" => (Read, &["namespace", "task_id", "version"]),
        "list_tasks" => (Read, &["namespace", "limit", "after_id"]),
        "record_task_checkpoint" => (
            Write,
            &[
                "namespace",
                "task_id",
                "request_id",
                "checkpoint_id",
                "expected_version",
                "expected_checkpoint_sequence",
                "steps",
                "blockers",
                "next_steps",
                "environment",
                "repository_revision",
                "artifacts",
                "source",
            ],
        ),
        "build_task_resume_context" => (Read, &["namespace", "task_id", "max_bytes"]),
        "ingest_interaction" => (
            Write,
            &[
                "namespace",
                "observed_at",
                "request_id",
                "event",
                "claim_drafts",
                "episode_reference",
                "trigger_hints",
            ],
        ),
        "recall_memory" => (Read, &["namespace", "query", "limit", "caller_budget"]),
        "build_task_context" => (
            Read,
            &["namespace", "query", "limit", "max_bytes", "caller_budget"],
        ),
        "search_memory" => (
            Read,
            &[
                "namespace",
                "record_type",
                "record_types",
                "event_reference",
                "kind",
                "limit",
                "recorded_after",
                "recorded_before",
                "claim_reference",
                "claim_status",
                "mode",
                "episode_reference",
                "reflection_reference",
            ],
        ),
        "get_memory" => (Read, &["namespace", "id", "record_type"]),
        "get_reflection_history" => (Read, &["namespace", "claim_reference", "limit"]),
        "get_evidence_relation" => (
            Read,
            &[
                "namespace",
                "trigger_window_event_ids",
                "selected_evidence_event_ids",
                "selection_basis",
            ],
        ),
        "supersede_memory" => (
            Write,
            &[
                "namespace",
                "request_id",
                "claim_reference",
                "replacement_claim",
                "replacement_evidence_event_ids",
                "summary",
            ],
        ),
        "derive_experience_from_use" => (
            Write,
            &[
                "namespace",
                "request_id",
                "use_id",
                "episode_id",
                "candidate_id",
                "episode_content",
                "candidate_content",
            ],
        ),
        "record_experience_use" => (
            Write,
            &[
                "namespace",
                "request_id",
                "use_id",
                "task_id",
                "task_version",
                "expected_task_version",
                "candidate_id",
                "candidate_version",
                "expected_candidate_version",
                "decision",
                "reason",
                "conditions",
                "action_event_refs",
                "outcome_event_refs",
                "verification",
                "evaluation_method",
                "limitations",
                "related_use_id",
            ],
        ),
        "list_experience_uses" => (Read, &["namespace", "task_id", "limit", "after_id"]),
        "record_episode" => (Write, &["namespace", "request_id", "episode_id", "content"]),
        "get_episode_detail" => (Read, &["namespace", "episode_id"]),
        "list_episode_details" => (Read, &["namespace", "limit", "after_id"]),
        "propose_experience_candidate" => (
            Write,
            &["namespace", "request_id", "candidate_id", "content"],
        ),
        "get_experience_candidate" => (Read, &["namespace", "candidate_id", "version"]),
        "list_experience_candidates" => (Read, &["namespace", "limit", "after_id", "status"]),
        "set_experience_candidate_status" => (
            Write,
            &[
                "namespace",
                "request_id",
                "candidate_id",
                "expected_version",
                "status",
            ],
        ),
        "revise_experience_candidate" => (
            Write,
            &[
                "namespace",
                "request_id",
                "candidate_id",
                "expected_version",
                "content",
            ],
        ),
        // This rollback only creates a pending version of a scoped experience.
        "rollback_experience_candidate" => (
            Write,
            &[
                "namespace",
                "request_id",
                "candidate_id",
                "expected_version",
                "target_version",
            ],
        ),
        "recall_experience_candidates" => (Read, &["namespace", "query", "limit", "max_bytes"]),
        "get_feedback_target_version" => (Read, &["namespace", "target_claim_reference"]),
        "propose_feedback_candidate" => (
            Write,
            &[
                "namespace",
                "target_claim_reference",
                "expected_target_version",
                "replacement_object",
                "evidence_event_ids",
                "summary",
                "request_id",
            ],
        ),
        "get_feedback_candidate" => (Read, &["namespace", "candidate_id"]),
        // Validation persists candidate state and a durable receipt.
        "validate_feedback_candidate" | "commit_feedback_candidate" => {
            (Write, &["namespace", "candidate_id", "request_id"])
        }
        "reject_feedback_candidate" => (
            Write,
            &["namespace", "candidate_id", "reason", "request_id"],
        ),
        "export_memory" => (
            Read,
            &["namespace", "max_records", "max_relations", "max_bytes"],
        ),
        _ => return Err(PolicyError::ToolNotAllowed),
    };
    Ok(ToolRule { access, fields })
}

/// Authorize against a freshly resolved host grant. The host must resolve the
/// grant again for each call to make changes/revocation effective in live sessions.
/// A write also needs read access because mutation results expose scoped records.
/// Only absent/null namespaces are filled; explicit conflicting values are denied.
pub fn authorize_tool_call(
    grant: &SessionGrant,
    name: &str,
    args: &Map<String, Value>,
) -> Result<AuthorizedCall, PolicyError> {
    let rule = rule(name)?;
    if args.keys().any(|key| !rule.fields.contains(&key.as_str())) {
        return Err(PolicyError::InvalidArguments);
    }
    let event = if name == "ingest_interaction" {
        // Legacy episode labels are global and can couple unrelated namespaces.
        // Scoped rich episodes must be linked through record_episode instead.
        if args
            .get("episode_reference")
            .is_some_and(|value| !value.is_null())
        {
            return Err(PolicyError::LegacyEpisodeLinkNotAllowed);
        }
        match args.get("trigger_hints") {
            None => (),
            Some(Value::Array(hints)) if hints.is_empty() => (),
            Some(Value::Array(_)) => return Err(PolicyError::AutomaticReflectionDisabled),
            _ => return Err(PolicyError::InvalidArguments),
        }
        Some(object(args.get("event"))?)
    } else {
        None
    };
    let requested = explicit_namespace(args)?;
    let event_scope = event.map(explicit_namespace).transpose()?.flatten();
    let namespace = requested
        .or(event_scope)
        .unwrap_or(grant.project_namespace.as_str());
    // No owner-based legacy fallback and no global self/world access, even if
    // a malformed host grant accidentally contains one of those namespaces.
    if !grant.project_namespace.starts_with("project/")
        || Namespace::parse(grant.project_namespace.clone()).is_err()
        || (namespace != grant.project_namespace && !namespace.starts_with("user/"))
        || !grant.allows_read(namespace)
        || (rule.access == ToolAccess::Write && !grant.allows_write(namespace))
    {
        return Err(PolicyError::ScopeDenied);
    }
    let scope = Namespace::parse(namespace.to_string()).map_err(|_| PolicyError::ScopeDenied)?;
    validate_nested_scope(args, &scope, 0)?;
    let mut normalized = args.clone();
    if name == "ingest_interaction" {
        // A top-level selector is a policy convenience, not an ignored DTO field.
        normalized.remove("namespace");
        normalize_record(normalized.get_mut("event"), &scope)?;
        let drafts = normalized
            .get_mut("claim_drafts")
            .and_then(Value::as_array_mut)
            .ok_or(PolicyError::InvalidArguments)?;
        for draft in drafts {
            normalize_record(Some(draft), &scope)?;
        }
    } else {
        normalized.insert("namespace".into(), Value::String(namespace.to_string()));
        if name == "supersede_memory" {
            normalize_record(normalized.get_mut("replacement_claim"), &scope)?;
        }
    }
    Ok(AuthorizedCall {
        namespace: namespace.to_string(),
        access: rule.access,
        args: normalized,
    })
}

fn object(value: Option<&Value>) -> Result<&Map<String, Value>, PolicyError> {
    value
        .and_then(Value::as_object)
        .ok_or(PolicyError::InvalidArguments)
}

fn explicit_namespace(args: &Map<String, Value>) -> Result<Option<&str>, PolicyError> {
    match args.get("namespace") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(namespace)) => Ok(Some(namespace)),
        _ => Err(PolicyError::InvalidArguments),
    }
}

fn normalize_record(value: Option<&mut Value>, scope: &Namespace) -> Result<(), PolicyError> {
    let record = value
        .and_then(Value::as_object_mut)
        .ok_or(PolicyError::InvalidArguments)?;
    // Existing project scopes derive World; owner is never silently rewritten.
    validate_owner(record.get("owner"), scope)?;
    record.insert(
        "namespace".into(),
        Value::String(scope.as_str().to_string()),
    );
    Ok(())
}

fn validate_owner(value: Option<&Value>, scope: &Namespace) -> Result<(), PolicyError> {
    let owner = match value.and_then(Value::as_str) {
        Some("Self_") => Owner::Self_,
        Some("User") => Owner::User,
        Some("World") => Owner::World,
        // Unknown matches World in the legacy domain helper; deny it explicitly.
        _ => return Err(PolicyError::OwnerMismatch),
    };
    if owner != scope.derived_owner() {
        return Err(PolicyError::OwnerMismatch);
    }
    Ok(())
}

/// Check every structured namespace/owner, including within extension-shaped
/// input before the DTO rejects it. Text is inert; it is never parsed as a grant.
fn validate_nested_scope(
    args: &Map<String, Value>,
    scope: &Namespace,
    depth: usize,
) -> Result<(), PolicyError> {
    if depth > 32 {
        return Err(PolicyError::InvalidArguments);
    }
    if explicit_namespace(args)?.is_some_and(|value| value != scope.as_str()) {
        return Err(PolicyError::MixedScope);
    }
    if let Some(owner) = args.get("owner") {
        validate_owner(Some(owner), scope)?;
    }
    for (key, value) in args {
        if key.ends_with("_namespace") || key.ends_with("_namespaces") {
            return Err(PolicyError::InvalidArguments);
        }
        validate_nested_value(value, scope, depth + 1)?;
    }
    Ok(())
}

fn validate_nested_value(
    value: &Value,
    scope: &Namespace,
    depth: usize,
) -> Result<(), PolicyError> {
    if depth > 32 {
        return Err(PolicyError::InvalidArguments);
    }
    match value {
        Value::Object(object) => validate_nested_scope(object, scope, depth),
        Value::Array(values) => {
            for value in values {
                validate_nested_value(value, scope, depth + 1)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
