#[cfg(feature = "release-tools")]
use agent_llm_mm::support::product_wording::{
    ClaimGateState, ProductClaimGuardInput, check_product_claims,
};
use agent_llm_mm::{
    domain::{
        episode_projection::{
            EpisodeLifecycleStatus, EpisodeProjectionInput, build_episode_summary_projection,
        },
        evidence_relation::{EvidenceRelationInput, build_evidence_relation_report},
        memory_layer_projection::{MemoryLayerProjectionInput, build_memory_layer_projection},
        memory_semantics_projection::{
            MemorySemanticsProjectionInput, build_memory_semantics_projection,
        },
        snapshot::SelfSnapshot,
    },
    error::AppError,
    run_doctor,
    support::remote_team::{
        RemoteTeamCapabilityState, remote_team_capability_inventory,
        remote_team_security_gate_report,
    },
};
use tempfile::tempdir;

#[test]
fn evidence_relation_report_keeps_selection_inside_trigger_window_with_ranking_metadata() {
    let report = build_evidence_relation_report(EvidenceRelationInput {
        trigger_window_event_ids: vec![
            "evt-5".to_string(),
            "evt-4".to_string(),
            "evt-3".to_string(),
        ],
        selected_evidence_event_ids: vec!["evt-4".to_string()],
        selection_basis: Some("query_intersection".to_string()),
    })
    .expect("selected evidence is inside the trigger window");

    assert_eq!(report.protocol_version, 2);
    assert_eq!(report.trigger_window_size, 3);
    assert_eq!(report.selected_count, 1);
    assert_eq!(
        report.no_widening_policy,
        "selected_subset_of_trigger_window"
    );
    assert_eq!(report.relations.len(), 3);
    assert_eq!(report.relations[1].event_id, "evt-4");
    assert_eq!(report.relations[1].window_rank, 2);
    assert!(report.relations[1].selected);
    assert_eq!(
        report.relations[1].selection_basis.as_deref(),
        Some("query_intersection")
    );
    assert!(
        report
            .relations
            .iter()
            .all(|relation| relation.event_id != "evt-outside")
    );
}

#[test]
fn evidence_relation_report_exposes_bounded_selection_weight_and_rejection_metadata() {
    let report = build_evidence_relation_report(EvidenceRelationInput {
        trigger_window_event_ids: vec![
            "evt-new".to_string(),
            "evt-mid".to_string(),
            "evt-old".to_string(),
        ],
        selected_evidence_event_ids: vec!["evt-mid".to_string()],
        selection_basis: Some("recency_and_kind_filter".to_string()),
    })
    .expect("selected evidence is inside the trigger window");

    assert_eq!(report.selected_count, 1);
    assert_eq!(report.rejected_count, 2);
    assert_eq!(report.weight_policy, "bounded_selected_binary_weight");

    let selected = report
        .relations
        .iter()
        .find(|relation| relation.event_id == "evt-mid")
        .expect("selected relation should be present");
    assert_eq!(selected.relation_status, "selected");
    assert_eq!(selected.selection_weight, 100);
    assert_eq!(
        selected.selection_basis.as_deref(),
        Some("recency_and_kind_filter")
    );

    let rejected = report
        .relations
        .iter()
        .filter(|relation| relation.relation_status == "available_not_selected")
        .collect::<Vec<_>>();
    assert_eq!(rejected.len(), 2);
    assert!(
        rejected
            .iter()
            .all(|relation| relation.selection_weight == 0)
    );
    assert!(rejected.iter().all(|relation| {
        relation.rejection_reason.as_deref() == Some("not_selected_by_current_policy")
    }));
    assert_eq!(selected.rejection_reason, None);
    assert!(
        report
            .relations
            .iter()
            .all(|relation| relation.selection_weight <= 100),
        "selection weights must stay bounded metadata, not an unbounded ranking engine"
    );
}

#[test]
fn evidence_relation_report_marks_empty_selection_as_available_not_selected_without_basis() {
    let report = build_evidence_relation_report(EvidenceRelationInput {
        trigger_window_event_ids: vec!["evt-new".to_string(), "evt-old".to_string()],
        selected_evidence_event_ids: Vec::new(),
        selection_basis: Some("query_intersection".to_string()),
    })
    .expect("empty selections are valid relation reports");

    assert_eq!(report.trigger_window_size, 2);
    assert_eq!(report.selected_count, 0);
    assert_eq!(report.rejected_count, 2);
    assert_eq!(report.relations.len(), 2);
    assert!(report.relations.iter().all(|relation| !relation.selected
        && relation.relation_status == "available_not_selected"
        && relation.selection_weight == 0
        && relation.selection_basis.is_none()));
}

#[test]
fn evidence_relation_report_normalizes_mixed_references_and_keeps_counts_statuses_and_json_consistent()
 {
    let report = build_evidence_relation_report(EvidenceRelationInput {
        trigger_window_event_ids: vec![
            "event:evt-new".to_string(),
            "evt-mid".to_string(),
            "evt-new".to_string(),
            "event:evt-old".to_string(),
        ],
        selected_evidence_event_ids: vec![
            "event:evt-mid".to_string(),
            "evt-mid".to_string(),
            "event:evt-old".to_string(),
        ],
        selection_basis: Some("explicit_model_ids".to_string()),
    })
    .expect("deduped selected evidence remains inside the trigger window");

    assert_eq!(report.trigger_window_size, 3);
    assert_eq!(report.selected_count, 2);
    assert_eq!(report.rejected_count, 1);
    assert_eq!(report.relations.len(), 3);
    assert_eq!(
        report
            .relations
            .iter()
            .map(|relation| relation.event_id.as_str())
            .collect::<Vec<_>>(),
        ["evt-new", "evt-mid", "evt-old"],
        "projection readback keeps ordered raw ids for its *_event_ids compatibility contract"
    );
    assert_eq!(
        report.selected_count + report.rejected_count,
        report.trigger_window_size
    );

    for relation in &report.relations {
        match relation.selected {
            true => {
                assert_eq!(relation.relation_status, "selected");
                assert_eq!(relation.selection_weight, 100);
                assert_eq!(
                    relation.selection_basis.as_deref(),
                    Some("explicit_model_ids")
                );
            }
            false => {
                assert_eq!(relation.relation_status, "available_not_selected");
                assert_eq!(relation.selection_weight, 0);
                assert_eq!(relation.selection_basis, None);
            }
        }
    }

    let serialized = serde_json::to_value(&report).expect("relation report serializes");
    assert_eq!(serialized["rejected_count"], 1);
    assert_eq!(
        serialized["weight_policy"],
        "bounded_selected_binary_weight"
    );
    assert_eq!(serialized["relations"][1]["relation_status"], "selected");
    assert_eq!(serialized["relations"][1]["selection_weight"], 100);
    assert_eq!(
        serialized["relations"][0]["relation_status"],
        "available_not_selected"
    );
    assert_eq!(
        serialized["relations"][0]["rejection_reason"],
        "not_selected_by_current_policy"
    );
    assert_eq!(
        serialized["relations"][1]["rejection_reason"],
        serde_json::Value::Null
    );
    assert!(
        serialized.to_string().contains("explicit_model_ids"),
        "stable selection basis labels may be serialized"
    );
    assert!(
        !serialized.to_string().contains("provider_payload"),
        "relation reports must not carry raw provider payloads"
    );
    assert!(
        !serialized.to_string().contains("event:evt-"),
        "event-id readback remains raw rather than changing the existing JSON shape"
    );
}

#[test]
fn evidence_relation_report_rejects_selected_evidence_outside_trigger_window() {
    let error = build_evidence_relation_report(EvidenceRelationInput {
        trigger_window_event_ids: vec!["evt-2".to_string(), "evt-1".to_string()],
        selected_evidence_event_ids: vec!["event:evt-outside".to_string()],
        selection_basis: Some("model_proposed_ids".to_string()),
    })
    .expect_err("selected evidence outside the current trigger window must be rejected");

    assert!(error.to_string().contains("evt-outside"));
    assert!(error.to_string().contains("outside the trigger window"));
}

#[test]
fn evidence_relation_report_rejects_invalid_event_reference_forms() {
    for invalid_event_id in ["", " ", "event:", "event:event:evt-window"] {
        let error = build_evidence_relation_report(EvidenceRelationInput {
            trigger_window_event_ids: vec![invalid_event_id.to_string()],
            selected_evidence_event_ids: Vec::new(),
            selection_basis: None,
        })
        .expect_err("invalid trigger-window references must fail closed");
        assert!(matches!(
            error,
            AppError::InvalidParams(message) if message == "InvalidEventReference"
        ));
    }
}

#[test]
fn episode_summary_projection_is_read_only_local_metadata_over_episode_events() {
    let projection = build_episode_summary_projection(EpisodeProjectionInput {
        episode_reference: "episode:task-42".to_string(),
        episode_event_ids: vec![
            "event:evt-objective".to_string(),
            "evt-action".to_string(),
            "event:evt-action".to_string(),
            "evt-outcome".to_string(),
        ],
        objective: Some("stabilize local evidence gate".to_string()),
        outcome: Some("blocked remote claims until auth gates exist".to_string()),
        lesson: None,
        linked_evidence_ids: vec![
            "event:evt-action".to_string(),
            "evt-action".to_string(),
            "event:evt-outcome".to_string(),
        ],
    })
    .expect("episode event projection should build");

    assert_eq!(projection.episode_reference, "episode:task-42");
    assert_eq!(
        projection.objective.as_deref(),
        Some("stabilize local evidence gate")
    );
    assert_eq!(
        projection.linked_evidence_ids,
        ["evt-action", "evt-outcome"]
    );
    assert_eq!(projection.event_count, 3);
    assert_eq!(
        projection.lifecycle_status,
        EpisodeLifecycleStatus::Concluded
    );
    assert!(!projection.writes_performed);
    assert_eq!(projection.durable_self_model_write_path, "run_reflection");
    assert_eq!(
        projection.identity_or_commitment_updates,
        Vec::<String>::new()
    );
    let serialized = serde_json::to_value(&projection).expect("projection JSON");
    assert_eq!(
        serialized["linked_evidence_ids"],
        serde_json::json!(["evt-action", "evt-outcome"])
    );
}

#[test]
fn episode_summary_projection_rejects_invalid_event_reference_forms() {
    for invalid_event_id in ["", " ", "event:", "event:event:evt-action"] {
        let error = build_episode_summary_projection(EpisodeProjectionInput {
            episode_reference: "episode:invalid-reference".to_string(),
            episode_event_ids: vec!["evt-objective".to_string()],
            objective: None,
            outcome: None,
            lesson: None,
            linked_evidence_ids: vec![invalid_event_id.to_string()],
        })
        .expect_err("invalid linked-evidence references must fail closed");
        assert!(matches!(
            error,
            AppError::InvalidParams(message) if message == "InvalidEventReference"
        ));
    }
}

#[test]
fn episode_summary_projection_exposes_lesson_with_goal_outcome_and_evidence() {
    let projection = build_episode_summary_projection(EpisodeProjectionInput {
        episode_reference: "episode:task-lesson".to_string(),
        episode_event_ids: vec![
            "evt-goal".to_string(),
            "evt-outcome".to_string(),
            "evt-lesson".to_string(),
        ],
        objective: Some("keep phase 4 memory work read-only".to_string()),
        outcome: Some("projected richer episode semantics without new writes".to_string()),
        lesson: Some(
            "Bounded lessons stay inspectable only when linked evidence remains visible"
                .to_string(),
        ),
        linked_evidence_ids: vec!["evt-outcome".to_string(), "evt-lesson".to_string()],
    })
    .expect("lesson projection should build from episode metadata");

    assert_eq!(
        projection.objective.as_deref(),
        Some("keep phase 4 memory work read-only")
    );
    assert_eq!(
        projection.outcome.as_deref(),
        Some("projected richer episode semantics without new writes")
    );
    assert_eq!(
        projection.lesson.as_deref(),
        Some("Bounded lessons stay inspectable only when linked evidence remains visible")
    );
    assert_eq!(
        projection.linked_evidence_ids,
        ["evt-outcome", "evt-lesson"]
    );
    assert!(!projection.writes_performed);
    assert_eq!(projection.durable_self_model_write_path, "run_reflection");
    assert_eq!(
        projection.identity_or_commitment_updates,
        Vec::<String>::new()
    );
}

#[test]
fn episode_summary_projection_marks_open_lifecycle_without_outcome() {
    let projection = build_episode_summary_projection(EpisodeProjectionInput {
        episode_reference: "episode:task-open".to_string(),
        episode_event_ids: vec!["evt-goal".to_string(), "evt-action".to_string()],
        objective: Some("track an in-flight local slice".to_string()),
        outcome: None,
        lesson: None,
        linked_evidence_ids: vec!["evt-action".to_string()],
    })
    .expect("open episode projection should build");

    // outcome 缺失时派生为 Open，仍只读、不写 identity/commitments。
    assert_eq!(projection.lifecycle_status, EpisodeLifecycleStatus::Open);
    assert!(!projection.writes_performed);
}

#[test]
fn remote_team_inventory_keeps_every_remote_or_team_capability_blocked_without_gates() {
    let inventory = remote_team_capability_inventory();

    assert!(inventory.local_only);
    assert!(!inventory.support_bundle_upload_available);
    assert!(
        inventory
            .capabilities
            .iter()
            .filter(|capability| capability.name.starts_with("remote_")
                || capability.name.starts_with("team_"))
            .all(|capability| capability.state == RemoteTeamCapabilityState::Blocked)
    );
    assert!(
        inventory
            .capabilities
            .iter()
            .all(|capability| !capability.write_capable_route_exposed)
    );
}

#[test]
fn remote_team_security_gates_block_remote_writes_until_all_prerequisites_exist() {
    let report = remote_team_security_gate_report();

    assert!(!report.remote_writes_allowed);
    assert!(report.blocked_gate_names().contains(&"auth"));
    assert!(report.blocked_gate_names().contains(&"authorization"));
    assert!(report.blocked_gate_names().contains(&"audit"));
    assert!(report.blocked_gate_names().contains(&"rate_limit"));
    assert!(report.blocked_gate_names().contains(&"tenant_isolation"));
    assert!(report.blocked_gate_names().contains(&"rollback"));
}

#[tokio::test]
async fn doctor_exposes_remote_team_inventory_and_security_gates_as_machine_readable_json() {
    let temp_dir = tempdir().expect("temp dir");
    let report = run_doctor(agent_llm_mm::support::config::AppConfig {
        database_url: format!(
            "sqlite://{}",
            temp_dir
                .path()
                .join("doctor-remote-team.sqlite")
                .to_string_lossy()
        ),
        ..Default::default()
    })
    .await
    .expect("doctor should pass");

    assert!(report.remote_team_capability_inventory.local_only);
    assert!(!report.remote_team_security_gates.remote_writes_allowed);
    assert!(
        report
            .remote_team_capability_inventory
            .capabilities
            .iter()
            .all(|capability| !capability.write_capable_route_exposed)
    );

    let serialized = serde_json::to_value(&report).expect("doctor JSON");
    assert_eq!(
        serialized["remote_team_capability_inventory"]["support_bundle_upload_available"],
        false
    );
    assert_eq!(
        serialized["remote_team_security_gates"]["remote_writes_allowed"],
        false
    );
}

#[tokio::test]
async fn doctor_memory_layer_exposes_read_only_classification_diagnostics() {
    let temp_dir = tempdir().expect("temp dir");
    let report = run_doctor(agent_llm_mm::support::config::AppConfig {
        database_url: format!(
            "sqlite://{}",
            temp_dir
                .path()
                .join("doctor-memory-layer.sqlite")
                .to_string_lossy()
        ),
        ..Default::default()
    })
    .await
    .expect("doctor should pass");

    let memory_layer = report
        .system_layer_report
        .layers
        .iter()
        .find(|layer| layer.name == "memory")
        .expect("memory layer should be present");

    // memory 层保持只读且 partial，不得开放任何写入。
    assert!(!memory_layer.writes_allowed);
    assert_eq!(memory_layer.status, "partial");

    let diagnostic_keys: Vec<&str> = memory_layer
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.key.as_str())
        .collect();
    assert!(diagnostic_keys.contains(&"layered_projection_classification"));
    assert!(diagnostic_keys.contains(&"self_model_durable_writes"));
    let experience = memory_layer
        .diagnostics
        .iter()
        .find(|item| item.key == "experience_candidate_runtime")
        .unwrap();
    assert_eq!(experience.status, "implemented");
    assert!(
        experience
            .detail
            .contains("activation grants no execution or permission")
    );

    let self_model_diagnostic = memory_layer
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.key == "self_model_durable_writes")
        .expect("self_model durable write diagnostic should be present");
    assert_eq!(self_model_diagnostic.status, "blocked");
    assert!(self_model_diagnostic.detail.contains("run_reflection"));
}

#[tokio::test]
async fn doctor_exposes_read_only_system_layer_report_with_architecture_blockers() {
    let temp_dir = tempdir().expect("temp dir");
    let report = run_doctor(agent_llm_mm::support::config::AppConfig {
        database_url: format!(
            "sqlite://{}",
            temp_dir
                .path()
                .join("doctor-system-layer.sqlite")
                .to_string_lossy()
        ),
        ..Default::default()
    })
    .await
    .expect("doctor should pass");

    let system_layer_report = &report.system_layer_report;
    assert!(system_layer_report.read_only);
    assert!(!system_layer_report.writes_performed);

    let layer_names: Vec<&str> = system_layer_report
        .layers
        .iter()
        .map(|layer| layer.name.as_str())
        .collect();
    assert_eq!(
        layer_names,
        [
            "substrate",
            "signal",
            "memory",
            "policy",
            "control_loop",
            "actuator",
            "interface",
            "release_boundary",
        ]
    );

    let allowed_statuses = [
        "implemented",
        "partial",
        "simulation-only",
        "planning-gate",
        "not-implemented",
    ];
    assert!(
        system_layer_report
            .layers
            .iter()
            .all(|layer| allowed_statuses.contains(&layer.status.as_str()))
    );

    let blockers = system_layer_report.blockers.join("\n").to_lowercase();
    for expected_blocker in [
        "local alpha",
        "windows parity",
        "fresh-machine",
        "remote/team",
        "daemon writes",
        "security/auth",
        "memory layering",
    ] {
        assert!(
            blockers.contains(expected_blocker),
            "missing blocker: {expected_blocker}"
        );
    }

    let substrate_layer = system_layer_report
        .layers
        .iter()
        .find(|layer| layer.name == "substrate")
        .expect("substrate layer should be present");
    let substrate_details = serde_json::to_string(&substrate_layer.diagnostics)
        .expect("substrate diagnostics should serialize");
    for expected_detail in [
        "config_shape",
        "database_path_shape",
        "platform_entrypoint",
        "data_lifecycle_gate_status",
    ] {
        assert!(
            substrate_details.contains(expected_detail),
            "missing substrate diagnostic detail: {expected_detail}; got {substrate_details}"
        );
    }

    let signal_layer = system_layer_report
        .layers
        .iter()
        .find(|layer| layer.name == "signal")
        .expect("signal layer should be present");
    let signal_diagnostics = serde_json::to_string(&signal_layer.diagnostics)
        .expect("signal diagnostics should serialize");
    for expected_detail in [
        "evidence_relation_report",
        "selected_subset_of_trigger_window",
        "bounded_selected_binary_weight",
    ] {
        assert!(
            signal_diagnostics.contains(expected_detail),
            "missing signal diagnostic detail: {expected_detail}; got {signal_diagnostics}"
        );
    }

    let evidence_contract = &system_layer_report.evidence_relation_contract;
    assert_eq!(evidence_contract.protocol_version, 2);
    assert!(evidence_contract.read_only);
    assert!(!evidence_contract.writes_performed);
    assert!(!evidence_contract.grants_capability);
    assert_eq!(
        evidence_contract.no_widening_policy,
        "selected_subset_of_trigger_window"
    );
    assert_eq!(
        evidence_contract.weight_policy,
        "bounded_selected_binary_weight"
    );
    assert_eq!(
        evidence_contract.relation_statuses,
        ["selected", "available_not_selected"]
    );
    assert_eq!(evidence_contract.selected_weight, 100);
    assert_eq!(evidence_contract.available_not_selected_weight, 0);
    assert!(
        evidence_contract
            .additive_v2_fields
            .contains(&"rejected_count".to_string())
    );
    assert!(
        evidence_contract
            .additive_v2_fields
            .contains(&"relation_status".to_string())
    );
    assert!(
        evidence_contract
            .additive_v2_fields
            .contains(&"selection_weight".to_string())
    );
    assert!(
        evidence_contract
            .additive_v2_fields
            .contains(&"rejection_reason".to_string())
    );
    let serialized_report = serde_json::to_value(system_layer_report).expect("report JSON");
    assert_eq!(
        serialized_report["evidence_relation_contract"]["weight_policy"],
        "bounded_selected_binary_weight"
    );

    let physics_principles: Vec<&str> = system_layer_report
        .physics_principles
        .iter()
        .map(|principle| principle.principle.as_str())
        .collect();
    assert_eq!(
        physics_principles,
        [
            "causality",
            "conservation",
            "arrow_of_time",
            "locality",
            "feedback_control",
            "entropy_increase",
            "energy_budget",
            "boundary_conditions",
        ]
    );
    assert!(
        system_layer_report
            .physics_principles
            .iter()
            .all(|principle| principle.report_only && !principle.grants_capability)
    );

    let dependency_rule_names: Vec<&str> = system_layer_report
        .dependency_rules
        .iter()
        .map(|rule| rule.name.as_str())
        .collect();
    assert_eq!(
        dependency_rule_names,
        [
            "actuator_has_no_dashboard_or_release_dependency",
            "write_capable_interface_requires_run_reflection_or_adr",
            "observe_only_daemon_must_not_call_actuator",
            "release_boundary_cannot_generate_external_evidence",
            "memory_writes_require_migration_and_lifecycle_gates",
        ]
    );
    assert!(
        system_layer_report
            .dependency_rules
            .iter()
            .all(|rule| rule.enforced_as == "read-only-boundary")
    );
    assert!(system_layer_report.dependency_rules.iter().all(|rule| {
        ["enforced", "declared-test-contract"].contains(&rule.status.as_str())
            && !rule.grants_capability
    }));
    assert!(
        system_layer_report
            .dependency_rules
            .iter()
            .all(|rule| !rule.evidence.is_empty()
                && rule.evidence.iter().all(|evidence| evidence.satisfied
                    && ["runtime", "declared_test_contract"].contains(&evidence.source.as_str())))
    );

    let daemon_rule = system_layer_report
        .dependency_rules
        .iter()
        .find(|rule| rule.name == "observe_only_daemon_must_not_call_actuator")
        .expect("daemon dependency rule");
    assert_eq!(daemon_rule.status, "enforced");
    assert!(daemon_rule.evidence.iter().any(|evidence| evidence.key
        == "run_reflection_allowed_from_daemon"
        && evidence.source == "runtime"
        && evidence.observed == "false"
        && evidence.expected == "false"));

    let release_rule = system_layer_report
        .dependency_rules
        .iter()
        .find(|rule| rule.name == "release_boundary_cannot_generate_external_evidence")
        .expect("release dependency rule");
    assert_eq!(release_rule.status, "declared-test-contract");
    for expected_key in [
        "local_refresh_does_not_generate_windows_parity",
        "local_refresh_does_not_generate_real_fresh_machine",
        "release_soak_keeps_human_decision_external",
    ] {
        assert!(
            release_rule
                .evidence
                .iter()
                .any(|evidence| evidence.key == expected_key
                    && evidence.source == "declared_test_contract"
                    && evidence.verification_command.is_some()
                    && evidence.observed == "verification_declared"
                    && evidence.expected == "verification_declared"),
            "missing release-boundary evidence key: {expected_key}"
        );
    }

    let memory_rule = system_layer_report
        .dependency_rules
        .iter()
        .find(|rule| rule.name == "memory_writes_require_migration_and_lifecycle_gates")
        .expect("memory dependency rule");
    assert_eq!(memory_rule.status, "declared-test-contract");
    assert!(memory_rule.evidence.iter().any(|evidence| evidence.key
        == "durable_new_memory_layer_writes_allowed"
        && evidence.source == "declared_test_contract"
        && evidence.verification_command.as_deref()
            == Some("cargo test --test product_completion_read_models -v")
        && evidence.observed == "verification_declared"
        && evidence.expected == "verification_declared"));

    let phase_numbers: Vec<u8> = system_layer_report
        .phase_coverage
        .iter()
        .map(|phase| phase.phase)
        .collect();
    assert_eq!(phase_numbers, [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    let phase_coverage =
        serde_json::to_string(&system_layer_report.phase_coverage).expect("phase coverage json");
    for expected_boundary in [
        "real fresh-machine evidence",
        "Windows runtime parity",
        "human release decision",
        "live provider evidence remains preflight-only",
        "provider gateway behavior",
        "daemon-triggered writes",
        "remote write admin",
        "tenant isolation",
        "installer",
        "Beta/GA claims",
    ] {
        assert!(
            phase_coverage.contains(expected_boundary),
            "missing phase boundary: {expected_boundary}"
        );
    }

    let non_claims = system_layer_report.non_claims.join("\n").to_lowercase();
    for non_claim in [
        "not a physics solver",
        "not a constraint optimizer",
        "not scientific validation evidence",
        "not complete multi-layer cognition",
        "not a remote/team product",
    ] {
        assert!(
            non_claims.contains(non_claim),
            "missing non-claim: {non_claim}"
        );
    }

    let actuator_layer = system_layer_report
        .layers
        .iter()
        .find(|layer| layer.name == "actuator")
        .expect("actuator layer");
    assert!(
        actuator_layer
            .anchors
            .contains(&"run_reflection".to_string())
    );
    assert!(!actuator_layer.writes_allowed);

    let serialized = serde_json::to_value(&report).expect("doctor JSON");
    assert!(serialized.get("system_layer_report").is_some());
    assert_eq!(
        serialized["system_layer_report"]["layers"][5]["writes_allowed"],
        false
    );
    let serialized_text = serde_json::to_string(&report).expect("doctor JSON text");
    assert!(serialized_text.contains("\"system_layer_report\""));
    assert!(serialized_text.contains("\"physics_principles\""));
    assert!(serialized_text.contains("\"dependency_rules\""));
    assert!(serialized_text.contains("\"phase_coverage\""));
    assert!(!serialized_text.contains("\"remote_writes_allowed\":true"));
    assert!(!serialized_text.contains("\"daemon_writes_allowed\":true"));
}

#[test]
fn memory_layer_projection_is_read_only_and_keeps_self_model_durable_writes_blocked() {
    let projection = build_memory_layer_projection(MemoryLayerProjectionInput {
        snapshot: SelfSnapshot {
            identity: vec!["identity:self=technical-demo".to_string()],
            commitments: vec!["forbid:write_identity_core_directly".to_string()],
            claims: vec!["claim:self.role=technical-demo".to_string()],
            evidence: vec!["event:evt-1".to_string()],
            episodes: vec!["episode:task-1".to_string()],
        },
        episode_projection_count: 1,
    });

    assert_eq!(
        projection.capability_scope,
        "legacy_snapshot_projection_only_not_v5_experience_inventory"
    );
    assert!(projection.read_only);
    assert!(!projection.writes_performed);
    assert_eq!(projection.durable_self_model_write_path, "run_reflection");
    assert_eq!(
        projection.layer_status("working").as_deref(),
        Some("partial")
    );
    assert_eq!(
        projection.layer_status("episodic").as_deref(),
        Some("partial")
    );
    assert_eq!(
        projection.layer_status("semantic").as_deref(),
        Some("partial")
    );
    assert_eq!(
        projection.layer_status("procedural").as_deref(),
        Some("not_implemented")
    );
    assert_eq!(
        projection.layer_status("self_model").as_deref(),
        Some("partial")
    );
}

#[test]
fn memory_semantics_projection_reports_richer_semantics_without_new_durable_writes() {
    let projection = build_memory_semantics_projection(MemorySemanticsProjectionInput {
        evidence_relation_count: 3,
        episode_summary_count: 2,
        semantic_claim_count: 4,
        procedural_memory_count: 0,
        self_model_write_migration_present: false,
    });

    assert!(projection.read_only);
    assert!(!projection.writes_performed);
    assert_eq!(projection.durable_self_model_write_path, "run_reflection");
    assert_semantic_capability(&projection, "evidence_relations", "partial");
    assert_semantic_capability(&projection, "episode_summaries", "partial");
    assert_semantic_capability(&projection, "procedural_memory", "not_implemented");
    assert_semantic_capability(&projection, "durable_self_model_writes", "blocked");
    assert!(
        projection
            .non_claims
            .iter()
            .any(|claim| claim.contains("not full ranking engine"))
    );
}

#[cfg(feature = "release-tools")]
#[test]
fn product_wording_guard_blocks_overstated_claims_without_matching_gates() {
    let report = check_product_claims(ProductClaimGuardInput {
        text: "Agent LLM MM is GA, production-ready, supports remote team service, and has complete self-governance.".to_string(),
        gate_state: ClaimGateState::default(),
    });

    assert!(!report.allowed);
    assert_eq!(report.violations.len(), 4);
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.claim == "ga")
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.claim == "production_ready")
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.claim == "remote_team_service")
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.claim == "complete_self_governance")
    );
}

fn assert_semantic_capability(
    projection: &agent_llm_mm::domain::memory_semantics_projection::MemorySemanticsProjection,
    capability: &str,
    status: &str,
) {
    let entry = projection
        .capabilities
        .iter()
        .find(|entry| entry.capability == capability)
        .unwrap_or_else(|| {
            panic!(
                "missing semantic capability {capability}; capabilities={:?}",
                projection.capabilities
            )
        });
    assert_eq!(entry.status, status);
    assert!(!entry.writes_allowed);
}
