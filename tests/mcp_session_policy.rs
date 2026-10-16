//! Pure boundary attacks: no database, network, credentials, or model runtime.
use agent_llm_mm::{
    domain::experience_use::DeriveExperienceRequest,
    interfaces::mcp::{dto::IngestInteractionParams, session_policy::*},
    support::workspace_registry::SessionGrant,
};
use serde_json::{Map, Value, json};

const PROJECT: &str = "project/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const PERSONAL: &str = "user/default";
const FOREIGN: &str = "project/bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const WRITES: &[&str] = &[
    "create_task",
    "revise_task",
    "record_task_checkpoint",
    "ingest_interaction",
    "supersede_memory",
    "record_episode",
    "record_experience_use",
    "derive_experience_from_use",
    "propose_experience_candidate",
    "set_experience_candidate_status",
    "revise_experience_candidate",
    "rollback_experience_candidate",
    "propose_feedback_candidate",
    "validate_feedback_candidate",
    "reject_feedback_candidate",
    "commit_feedback_candidate",
];
const DENIED: &[&str] = &[
    "build_self_snapshot",
    "decide_with_snapshot",
    "run_reflection",
    "get_self_model_versions",
    "get_self_model_history",
    "inspect_retrieval_index",
    "rebuild_retrieval_index",
];

fn grant() -> SessionGrant {
    SessionGrant {
        workspace_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".into(),
        project_namespace: PROJECT.into(),
        readable_namespaces: vec![PROJECT.into(), PERSONAL.into()],
        writable_namespaces: vec![PROJECT.into()],
    }
}
fn object(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}
fn fixture(name: &str) -> Map<String, Value> {
    object(match name {
        "create_task"
        | "revise_task"
        | "record_task_checkpoint"
        | "get_task"
        | "build_task_resume_context" => json!({"task_id":"synthetic"}),
        "list_tasks" => json!({}),
        "list_experience_uses" => json!({"task_id":"synthetic"}),
        "derive_experience_from_use" => json!({
            "request_id":"synthetic-request", "use_id":"synthetic-use",
            "episode_id":"synthetic-episode", "candidate_id":"synthetic-candidate",
            "episode_content": {
                "title":"synthetic", "objective":"test", "actions":[], "observations":["synthetic"],
                "outcome":"test", "lesson":"test", "limitations":[], "source_event_refs":["event:synthetic"]
            },
            "candidate_content": {
                "kind":"semantic", "title":"synthetic", "statement":"test", "steps":[],
                "limitations":[], "source_episode_ids":["synthetic-episode"]
            }
        }),
        "record_experience_use" => json!({
            "request_id":"synthetic-request", "use_id":"synthetic-use",
            "task_id":"synthetic", "task_version":1, "expected_task_version":1,
            "candidate_id":"synthetic", "candidate_version":1, "expected_candidate_version":1,
            "decision":"proposed", "reason":"synthetic report",
            "conditions":[{"condition":"synthetic applicability", "assessment":"unknown"}],
            "action_event_refs":[], "outcome_event_refs":[], "verification":"not_performed",
            "evaluation_method":"caller report", "limitations":["not verified"], "related_use_id":null
        }),
        "ingest_interaction" => json!({
            "event":{"owner":"World","kind":"Conversation","summary":"synthetic"},
            "claim_drafts":[{"owner":"World","subject":"fixture","predicate":"is","object":"synthetic","mode":"Said"}],
            "episode_reference":null,"request_id":"synthetic-request"
        }),
        "recall_memory" | "build_task_context" | "recall_experience_candidates" => {
            json!({"query":"synthetic"})
        }
        "search_memory"
        | "list_episode_details"
        | "list_experience_candidates"
        | "export_memory" => json!({}),
        "get_memory" => json!({"id":"event:synthetic","record_type":"Event"}),
        "get_reflection_history" => json!({"claim_reference":"claim:synthetic"}),
        "get_evidence_relation" => json!({"trigger_window_event_ids":["event:synthetic"]}),
        "supersede_memory" => json!({
            "claim_reference":"claim:synthetic","request_id":"synthetic-request",
            "replacement_claim":{"owner":"World","subject":"fixture","predicate":"is","object":"corrected","mode":"Said"},
            "replacement_evidence_event_ids":["event:synthetic"],"summary":"synthetic correction"
        }),
        "record_episode" => {
            json!({"request_id":"synthetic-request","episode_id":"synthetic","content":{
                "title":"synthetic","objective":"test","actions":[],"observations":["synthetic"],
                "outcome":"test","lesson":"test","limitations":[],"source_event_refs":["event:synthetic"]
            }})
        }
        "get_episode_detail" => json!({"episode_id":"synthetic"}),
        "propose_experience_candidate" | "revise_experience_candidate" => {
            let mut value = json!({"request_id":"synthetic-request","candidate_id":"synthetic","content":{
                "kind":"semantic","title":"synthetic","statement":"test","steps":[],"limitations":[],"source_episode_ids":["synthetic"]
            }});
            if name == "revise_experience_candidate" {
                value["expected_version"] = json!(1);
            }
            value
        }
        "get_experience_candidate" | "get_feedback_candidate" => {
            json!({"candidate_id":"synthetic"})
        }
        "set_experience_candidate_status" => {
            json!({"request_id":"synthetic-request","candidate_id":"synthetic","expected_version":1,"status":"active"})
        }
        "rollback_experience_candidate" => {
            json!({"request_id":"synthetic-request","candidate_id":"synthetic","expected_version":2,"target_version":1})
        }
        "get_feedback_target_version" => json!({"target_claim_reference":"claim:synthetic"}),
        "propose_feedback_candidate" => {
            json!({"request_id":"synthetic-request","target_claim_reference":"claim:synthetic","expected_target_version":"synthetic","replacement_object":"synthetic","evidence_event_ids":["event:synthetic"],"summary":"synthetic"})
        }
        "validate_feedback_candidate" | "commit_feedback_candidate" => {
            json!({"request_id":"synthetic-request","candidate_id":"synthetic"})
        }
        "reject_feedback_candidate" => {
            json!({"request_id":"synthetic-request","candidate_id":"synthetic","reason":"synthetic"})
        }
        _ => panic!("fixture must cover every allowed tool"),
    })
}
fn with_scope(name: &str, scope: &str) -> Map<String, Value> {
    let mut args = fixture(name);
    args.insert("namespace".into(), json!(scope));
    let owner = if scope.starts_with("user/") {
        "User"
    } else {
        "World"
    };
    if name == "ingest_interaction" {
        args["event"]["owner"] = json!(owner);
        args["claim_drafts"][0]["owner"] = json!(owner);
    }
    if name == "supersede_memory" {
        args["replacement_claim"]["owner"] = json!(owner);
    }
    args
}

#[test]
fn explicit_inventory_covers_all_41_existing_tools_and_defaults_are_materialized() {
    assert_eq!(allowed_tool_names().len() + DENIED.len(), 41);
    let mut unique = allowed_tool_names().to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 34);
    for name in allowed_tool_names() {
        let args = fixture(name);
        let before = args.clone();
        let approved = authorize_tool_call(&grant(), name, &args).unwrap();
        assert_eq!(approved.namespace, PROJECT, "{name}");
        assert_eq!(
            approved.access,
            if WRITES.contains(name) {
                ToolAccess::Write
            } else {
                ToolAccess::Read
            },
            "{name}"
        );
        assert_eq!(args, before, "policy must be pure: {name}");
        if *name == "ingest_interaction" {
            assert_eq!(approved.args["event"]["namespace"], PROJECT);
            assert_eq!(approved.args["claim_drafts"][0]["namespace"], PROJECT);
            assert!(!approved.args.contains_key("namespace"));
            let dto: IngestInteractionParams =
                serde_json::from_value(Value::Object(approved.args)).unwrap();
            assert_eq!(dto.event.namespace.as_deref(), Some(PROJECT));
            assert_eq!(dto.claim_drafts[0].namespace.as_deref(), Some(PROJECT));
        } else {
            assert_eq!(approved.args["namespace"], PROJECT, "{name}");
            if *name == "supersede_memory" {
                assert_eq!(approved.args["replacement_claim"]["namespace"], PROJECT);
            }
        }
    }
}

#[test]
fn all_writes_including_feedback_validation_and_experience_transitions_require_write_grant() {
    for name in allowed_tool_names() {
        let result = authorize_tool_call(&grant(), name, &with_scope(name, PERSONAL));
        if WRITES.contains(name) {
            assert_eq!(result.unwrap_err(), PolicyError::ScopeDenied, "{name}");
        } else {
            assert_eq!(result.unwrap().access, ToolAccess::Read, "{name}");
        }
    }
}

#[test]
fn personal_access_is_explicit_and_revocation_applies_to_the_next_call() {
    let mut permissions = grant();
    permissions.writable_namespaces.push(PERSONAL.into());
    for name in allowed_tool_names() {
        assert!(
            authorize_tool_call(&permissions, name, &with_scope(name, PERSONAL)).is_ok(),
            "{name}"
        );
    }
    permissions.writable_namespaces.retain(|ns| ns != PERSONAL);
    assert_eq!(
        authorize_tool_call(
            &permissions,
            "validate_feedback_candidate",
            &with_scope("validate_feedback_candidate", PERSONAL)
        )
        .unwrap_err(),
        PolicyError::ScopeDenied
    );
    assert!(
        authorize_tool_call(
            &permissions,
            "get_feedback_candidate",
            &with_scope("get_feedback_candidate", PERSONAL)
        )
        .is_ok()
    );
    permissions.readable_namespaces.retain(|ns| ns != PERSONAL);
    assert_eq!(
        authorize_tool_call(
            &permissions,
            "get_feedback_candidate",
            &with_scope("get_feedback_candidate", PERSONAL)
        )
        .unwrap_err(),
        PolicyError::ScopeDenied
    );
    assert!(authorize_tool_call(&permissions, "get_memory", &fixture("get_memory")).is_ok());
}

#[test]
fn write_grants_cannot_disclose_records_without_read_permission() {
    let mut permissions = grant();
    permissions.readable_namespaces.clear();
    assert_eq!(
        authorize_tool_call(
            &permissions,
            "supersede_memory",
            &fixture("supersede_memory")
        )
        .unwrap_err(),
        PolicyError::ScopeDenied
    );
}

#[test]
fn explicit_nested_scopes_are_never_overwritten_even_when_both_scopes_are_granted() {
    let mut permissions = grant();
    permissions.writable_namespaces.push(PERSONAL.into());
    let mut args = with_scope("ingest_interaction", PROJECT);
    args["event"]["namespace"] = json!(PERSONAL);
    assert_eq!(
        authorize_tool_call(&permissions, "ingest_interaction", &args).unwrap_err(),
        PolicyError::MixedScope
    );
    args["event"]["namespace"] = json!(PROJECT);
    args["claim_drafts"][0]["namespace"] = json!(PERSONAL);
    assert_eq!(
        authorize_tool_call(&permissions, "ingest_interaction", &args).unwrap_err(),
        PolicyError::MixedScope
    );
    let mut correction = with_scope("supersede_memory", PROJECT);
    correction["replacement_claim"]["namespace"] = json!(PERSONAL);
    assert_eq!(
        authorize_tool_call(&permissions, "supersede_memory", &correction).unwrap_err(),
        PolicyError::MixedScope
    );
}

#[test]
fn every_claim_is_checked_and_a_late_mixed_scope_denies_the_whole_ingest() {
    let mut args = fixture("ingest_interaction");
    let mut last = args["claim_drafts"][0].clone();
    last["namespace"] = json!(FOREIGN);
    args["claim_drafts"].as_array_mut().unwrap().push(last);
    let before = args.clone();
    assert_eq!(
        authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
        PolicyError::MixedScope
    );
    assert_eq!(args, before);
}

#[test]
fn explicit_event_scope_selects_personal_only_with_a_host_write_grant() {
    let mut args = with_scope("ingest_interaction", PERSONAL);
    args.remove("namespace");
    args["event"]["namespace"] = json!(PERSONAL);
    assert_eq!(
        authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
        PolicyError::ScopeDenied
    );
    let mut permissions = grant();
    permissions.writable_namespaces.push(PERSONAL.into());
    let approved = authorize_tool_call(&permissions, "ingest_interaction", &args).unwrap();
    assert_eq!(approved.namespace, PERSONAL);
    assert_eq!(approved.args["claim_drafts"][0]["namespace"], PERSONAL);
}

#[test]
fn missing_namespace_never_falls_back_to_legacy_user_world_or_self_defaults() {
    for owner in ["User", "Self_", "Unknown"] {
        let mut args = fixture("ingest_interaction");
        args["event"]["owner"] = json!(owner);
        assert_eq!(
            authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
            PolicyError::OwnerMismatch
        );
    }
    let mut args = fixture("ingest_interaction");
    args["event"]["namespace"] = Value::Null;
    args["claim_drafts"][0]["namespace"] = Value::Null;
    let approved = authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap();
    assert_eq!(approved.args["event"]["namespace"], PROJECT);
    assert_eq!(approved.args["claim_drafts"][0]["namespace"], PROJECT);
}

#[test]
fn unknown_or_mismatched_claim_owner_cannot_use_legacy_world_matching() {
    for owner in [
        json!("Unknown"),
        json!("User"),
        json!("Self_"),
        json!(null),
        json!(0),
    ] {
        for name in ["ingest_interaction", "supersede_memory"] {
            let mut args = fixture(name);
            if name == "ingest_interaction" {
                args["claim_drafts"][0]["owner"] = owner.clone();
            } else {
                args["replacement_claim"]["owner"] = owner.clone();
            }
            assert_eq!(
                authorize_tool_call(&grant(), name, &args).unwrap_err(),
                PolicyError::OwnerMismatch
            );
        }
    }
}

#[test]
fn malformed_scope_types_and_records_fail_closed() {
    for value in [
        json!(false),
        json!(12),
        json!([]),
        json!({"namespace":PROJECT}),
    ] {
        let mut args = fixture("recall_memory");
        args.insert("namespace".into(), value.clone());
        assert_eq!(
            authorize_tool_call(&grant(), "recall_memory", &args).unwrap_err(),
            PolicyError::InvalidArguments
        );
        let mut ingest = fixture("ingest_interaction");
        ingest["claim_drafts"][0]["namespace"] = value;
        assert_eq!(
            authorize_tool_call(&grant(), "ingest_interaction", &ingest).unwrap_err(),
            PolicyError::InvalidArguments
        );
    }
    for field in ["event", "claim_drafts"] {
        let mut args = fixture("ingest_interaction");
        args.insert(field.into(), Value::Null);
        assert_eq!(
            authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
            PolicyError::InvalidArguments
        );
    }
    for scope in ["", " project/a", "project/", "*", "user/default ", FOREIGN] {
        assert_eq!(
            authorize_tool_call(&grant(), "get_memory", &with_scope("get_memory", scope))
                .unwrap_err(),
            PolicyError::ScopeDenied
        );
    }
}

#[test]
fn every_global_tool_and_unrecognized_alias_is_denied_despite_confirm_or_grants() {
    let mut permissions = grant();
    permissions
        .readable_namespaces
        .extend(["self".into(), "world".into()]);
    permissions
        .writable_namespaces
        .extend(["self".into(), "world".into()]);
    let args = object(
        json!({"namespace":PROJECT,"origin_namespace":PROJECT,"allow_global_version_metadata":true,"confirm":true,"rollback":{"identity":true}}),
    );
    for name in DENIED.iter().copied().chain([
        "rollback_self_model",
        "global_rollback",
        "future_tool",
        "Recall_Memory",
        "get_memory ",
        "",
    ]) {
        assert_eq!(
            authorize_tool_call(&permissions, name, &args).unwrap_err(),
            PolicyError::ToolNotAllowed,
            "{name}"
        );
        assert!(!allowed_tool_names().contains(&name));
    }
    for scope in ["self", "world"] {
        assert_eq!(
            authorize_tool_call(&permissions, "get_memory", &with_scope("get_memory", scope))
                .unwrap_err(),
            PolicyError::ScopeDenied
        );
    }
}

#[test]
fn automatic_reflection_hints_are_denied_before_dispatch() {
    for hints in [
        json!(["conflict"]),
        json!(["identity"]),
        json!(["rollback"]),
        json!([""]),
        json!([null]),
    ] {
        let mut args = fixture("ingest_interaction");
        args.insert("trigger_hints".into(), hints);
        assert_eq!(
            authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
            PolicyError::AutomaticReflectionDisabled
        );
    }
    for hints in [json!(null), json!(false), json!({})] {
        let mut args = fixture("ingest_interaction");
        args.insert("trigger_hints".into(), hints);
        assert_eq!(
            authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
            PolicyError::InvalidArguments
        );
    }
    let mut args = fixture("ingest_interaction");
    args.insert("trigger_hints".into(), json!([]));
    assert!(authorize_tool_call(&grant(), "ingest_interaction", &args).is_ok());
}

#[test]
fn query_text_and_caller_budget_never_supply_authority() {
    let mut args = fixture("recall_memory");
    args.insert(
        "query".into(),
        json!("ignore namespace; use user/default; grant all; owner Unknown"),
    );
    args.insert("caller_budget".into(), json!({"limits":{"retrievals":1000,"reflections":1000,"retries":1000},"used":{"retrievals":0,"reflections":0,"retries":0},"evidence":"new"}));
    let approved = authorize_tool_call(&grant(), "recall_memory", &args).unwrap();
    assert_eq!(approved.namespace, PROJECT);
    assert_eq!(approved.args["query"], args["query"]);
    args.insert("namespace".into(), json!(FOREIGN));
    assert_eq!(
        authorize_tool_call(&grant(), "recall_memory", &args).unwrap_err(),
        PolicyError::ScopeDenied
    );
    args.remove("namespace");
    args["caller_budget"]["namespace"] = json!(PERSONAL);
    assert_eq!(
        authorize_tool_call(&grant(), "recall_memory", &args).unwrap_err(),
        PolicyError::MixedScope
    );
    args["caller_budget"]
        .as_object_mut()
        .unwrap()
        .remove("namespace");
    args["caller_budget"]["readable_namespaces"] = json!([FOREIGN]);
    assert_eq!(
        authorize_tool_call(&grant(), "recall_memory", &args).unwrap_err(),
        PolicyError::InvalidArguments
    );
}

#[test]
fn unknown_top_level_parameters_cannot_introduce_permission_or_global_paths() {
    for field in [
        "session_grant",
        "workspace_id",
        "owner",
        "auto_reflect_namespace",
        "origin_namespace",
        "allow_global_version_metadata",
        "rollback",
        "unexpected_extension",
    ] {
        let mut args = fixture("get_memory");
        args.insert(field.into(), json!({"namespace":FOREIGN,"confirm":true}));
        assert_eq!(
            authorize_tool_call(&grant(), "get_memory", &args).unwrap_err(),
            PolicyError::InvalidArguments
        );
    }
}

#[test]
fn nested_extension_scope_and_owner_are_checked_before_any_dto_can_ignore_them() {
    for (key, value, expected) in [
        ("namespace", json!(FOREIGN), PolicyError::MixedScope),
        ("owner", json!("Unknown"), PolicyError::OwnerMismatch),
        (
            "auto_reflect_namespace",
            json!(PROJECT),
            PolicyError::InvalidArguments,
        ),
    ] {
        let mut args = fixture("ingest_interaction");
        args["event"]["feedback"] = json!({"extension":[{key:value}]});
        assert_eq!(
            authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
            expected
        );
    }
}

#[test]
fn nested_scope_walk_is_bounded() {
    let mut value = json!({"namespace":FOREIGN});
    for _ in 0..40 {
        value = json!([value]);
    }
    let mut args = fixture("recall_memory");
    args.insert("caller_budget".into(), value);
    assert_eq!(
        authorize_tool_call(&grant(), "recall_memory", &args).unwrap_err(),
        PolicyError::InvalidArguments
    );
}

#[test]
fn permission_errors_disclose_neither_request_content_nor_available_scopes() {
    let mut args = with_scope("recall_memory", "user/private-sensitive-name");
    args.insert("query".into(), json!("sensitive request body"));
    let error = authorize_tool_call(&grant(), "recall_memory", &args).unwrap_err();
    for rendered in [format!("{error}"), format!("{error:?}")] {
        for forbidden in [
            "private-sensitive-name",
            "sensitive request body",
            PROJECT,
            PERSONAL,
        ] {
            assert!(!rendered.contains(forbidden));
        }
    }
    assert_eq!(ToolAccess::Read.as_str(), "read");
    assert_eq!(ToolAccess::Write.as_str(), "write");
    assert_eq!(serde_json::to_value(ToolAccess::Write).unwrap(), "write");
}

#[test]
fn malformed_default_project_grant_never_reintroduces_global_fallback() {
    let mut permissions = grant();
    for project in ["self", "world", "project/", "user/default"] {
        permissions.project_namespace = project.into();
        permissions.readable_namespaces.push(project.into());
        assert_eq!(
            authorize_tool_call(&permissions, "get_memory", &fixture("get_memory")).unwrap_err(),
            PolicyError::ScopeDenied
        );
    }
}

#[test]
fn global_legacy_episode_labels_cannot_link_workspace_ingest_records() {
    for reference in [
        json!("shared-label"),
        json!(""),
        json!(true),
        json!(1),
        json!([]),
        json!({}),
    ] {
        let mut args = fixture("ingest_interaction");
        args.insert("episode_reference".into(), reference);
        assert_eq!(
            authorize_tool_call(&grant(), "ingest_interaction", &args).unwrap_err(),
            PolicyError::LegacyEpisodeLinkNotAllowed
        );
    }
    let mut args = fixture("ingest_interaction");
    args.remove("episode_reference");
    assert!(authorize_tool_call(&grant(), "ingest_interaction", &args).is_ok());
    args.insert("episode_reference".into(), Value::Null);
    assert!(authorize_tool_call(&grant(), "ingest_interaction", &args).is_ok());
    assert!(authorize_tool_call(&grant(), "record_episode", &fixture("record_episode")).is_ok());
}

#[test]
fn task_tools_are_bound_to_project_and_revocation_is_checked_on_each_call() {
    for name in [
        "create_task",
        "revise_task",
        "get_task",
        "list_tasks",
        "record_task_checkpoint",
        "build_task_resume_context",
    ] {
        assert!(authorize_tool_call(&grant(), name, &fixture(name)).is_ok());
        assert_eq!(
            authorize_tool_call(&grant(), name, &with_scope(name, FOREIGN)).unwrap_err(),
            PolicyError::ScopeDenied
        );
        let mut revoked = grant();
        revoked.readable_namespaces.clear();
        revoked.writable_namespaces.clear();
        assert_eq!(
            authorize_tool_call(&revoked, name, &fixture(name)).unwrap_err(),
            PolicyError::ScopeDenied
        );
    }
}

#[test]
fn experience_use_contract_accepts_only_reviewed_fields_and_rechecks_scope() {
    use agent_llm_mm::domain::experience_use::{
        ListExperienceUsesRequest, RecordExperienceUseRequest,
    };
    let mut permissions = grant();
    let approved = authorize_tool_call(
        &permissions,
        "derive_experience_from_use",
        &fixture("derive_experience_from_use"),
    )
    .unwrap();
    let request: DeriveExperienceRequest =
        serde_json::from_value(Value::Object(approved.args)).unwrap();
    request.validate().unwrap();
    let approved = authorize_tool_call(
        &permissions,
        "record_experience_use",
        &fixture("record_experience_use"),
    )
    .unwrap();
    let request: RecordExperienceUseRequest =
        serde_json::from_value(Value::Object(approved.args)).unwrap();
    request.validate().unwrap();
    let approved = authorize_tool_call(
        &permissions,
        "list_experience_uses",
        &fixture("list_experience_uses"),
    )
    .unwrap();
    let request: ListExperienceUsesRequest =
        serde_json::from_value(Value::Object(approved.args)).unwrap();
    request.validate().unwrap();
    for name in [
        "record_experience_use",
        "list_experience_uses",
        "derive_experience_from_use",
    ] {
        let mut args = fixture(name);
        args.insert("trusted_host".into(), json!(true));
        assert_eq!(
            authorize_tool_call(&permissions, name, &args).unwrap_err(),
            PolicyError::InvalidArguments
        );
        assert_eq!(
            authorize_tool_call(&permissions, name, &with_scope(name, FOREIGN)).unwrap_err(),
            PolicyError::ScopeDenied
        );
    }
    permissions.writable_namespaces.clear();
    assert_eq!(
        authorize_tool_call(
            &permissions,
            "record_experience_use",
            &fixture("record_experience_use")
        )
        .unwrap_err(),
        PolicyError::ScopeDenied
    );
    assert!(
        authorize_tool_call(
            &permissions,
            "list_experience_uses",
            &fixture("list_experience_uses")
        )
        .is_ok()
    );
    permissions.readable_namespaces.clear();
    assert_eq!(
        authorize_tool_call(
            &permissions,
            "list_experience_uses",
            &fixture("list_experience_uses")
        )
        .unwrap_err(),
        PolicyError::ScopeDenied
    );
}
