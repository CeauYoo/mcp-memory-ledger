use serde::Serialize;

use crate::{
    adapters::sqlite::{
        DatabaseLifecycleReport, SqliteStore, initialize_database, inspect_database,
        migrate_database, open_read_only_current_database,
    },
    domain::{
        evidence_relation::{
            EVIDENCE_RELATION_AVAILABLE_NOT_SELECTED_STATUS, EVIDENCE_RELATION_NO_WIDENING_POLICY,
            EVIDENCE_RELATION_PROTOCOL_VERSION, EVIDENCE_RELATION_SELECTED_STATUS,
            EVIDENCE_RELATION_WEIGHT_POLICY, SELECTED_EVIDENCE_WEIGHT, UNSELECTED_EVIDENCE_WEIGHT,
        },
        operation_log::{OperationLogKind, OperationLogStatus},
    },
    interfaces,
    ports::{OperationLogQuery, OperationLogStore},
    support::config::{AppConfig, ModelProviderKind, ProviderMatrixEntry, TransportKind},
};

use super::remote_team::{
    RemoteTeamCapabilityInventory, RemoteTeamSecurityGateReport, remote_team_capability_inventory,
    remote_team_security_gate_report,
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorReport {
    pub transport: TransportKind,
    pub database_url: String,
    pub database_lifecycle: DatabaseLifecycleReport,
    pub retrieval_index: Option<crate::adapters::sqlite::RetrievalIndexReport>,
    pub provider: ModelProviderKind,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub provider_matrix: Vec<DoctorProviderMatrixEntry>,
    pub dashboard_enabled: bool,
    pub dashboard_host: String,
    pub dashboard_port: u16,
    pub dashboard_base_path: String,
    pub dashboard_required: bool,
    pub daemon_enabled: bool,
    pub daemon_poll_interval_ms: u64,
    pub daemon_max_concurrent_tasks: u32,
    pub daemon_observe_only: DaemonObserveOnlyDiagnostics,
    pub remote_team_capability_inventory: RemoteTeamCapabilityInventory,
    pub remote_team_security_gates: RemoteTeamSecurityGateReport,
    pub system_layer_report: SystemLayerReport,
    pub auto_reflection_runtime_hooks: Vec<String>,
    pub self_revision_write_path: &'static str,
    pub status: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderSupportState {
    Supported,
    PlannedOnly,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorProviderMatrixEntry {
    pub provider: &'static str,
    pub support_state: ProviderSupportState,
    pub configurable: bool,
    pub adapter: &'static str,
    pub missing_implementation: &'static str,
    pub selected: bool,
    // 只读派生标记：该行是否可被运行时实际选用（必须受支持且可配置）。
    // planned-only 行恒为 false，避免读者误以为未实现的行可投入配置。
    pub selectable_for_runtime: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DaemonObserveOnlyDiagnostics {
    pub mode: &'static str,
    pub local_only: bool,
    pub write_gate_approved: bool,
    pub writes_allowed: bool,
    pub remote_listener_enabled: bool,
    pub write_blockers: Vec<String>,
    pub remote_blockers: Vec<String>,
    pub data_sources: Vec<String>,
    pub candidate_read_data_source: &'static str,
    pub candidate_read_operation_kinds: Vec<&'static str>,
    pub candidate_read_statuses: Vec<&'static str>,
    pub candidate_read_limit_per_kind_status: usize,
    pub candidate_reads_are_read_only: bool,
    pub trigger_candidates_observed: usize,
    pub trigger_candidates_suppressed: usize,
    pub cooldown_status: &'static str,
    pub cooldown_diagnostics: &'static str,
    pub suppression_diagnostics: &'static str,
    pub clean_shutdown_status: &'static str,
    pub lifecycle_regression_status: &'static str,
    pub semantic_writes_allowed: bool,
    pub run_reflection_allowed_from_daemon: bool,
    pub write_capable_daemon_gate_status: &'static str,
    pub background_autonomy_enabled: bool,
    pub daemon_loop_connected: bool,
    pub daemon_started_by_doctor: bool,
    pub in_flight_task_count: usize,
    pub read_errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemLayerReport {
    pub read_only: bool,
    pub writes_performed: bool,
    pub layers: Vec<SystemLayerEntry>,
    pub evidence_relation_contract: EvidenceRelationContract,
    pub physics_principles: Vec<PhysicsPrincipleMapping>,
    pub dependency_rules: Vec<SystemDependencyRule>,
    pub phase_coverage: Vec<SystemPhaseCoverage>,
    pub blockers: Vec<String>,
    pub non_claims: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EvidenceRelationContract {
    pub protocol_version: u32,
    pub read_only: bool,
    pub writes_performed: bool,
    pub grants_capability: bool,
    pub no_widening_policy: &'static str,
    pub weight_policy: &'static str,
    pub relation_statuses: Vec<String>,
    pub selected_weight: u8,
    pub available_not_selected_weight: u8,
    pub additive_v2_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemLayerEntry {
    pub name: String,
    pub status: String,
    pub responsibility: String,
    pub anchors: Vec<String>,
    pub writes_allowed: bool,
    pub blockers: Vec<String>,
    pub diagnostics: Vec<SystemLayerDiagnostic>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemLayerDiagnostic {
    pub key: String,
    pub status: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PhysicsPrincipleMapping {
    pub principle: String,
    pub engineering_interpretation: String,
    pub structural_rule: String,
    pub implementation_anchor: String,
    pub report_only: bool,
    pub grants_capability: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemDependencyRule {
    pub name: String,
    pub rule: String,
    pub enforced_as: String,
    pub status: String,
    pub grants_capability: bool,
    pub evidence: Vec<SystemDependencyEvidence>,
    pub verification: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemDependencyEvidence {
    pub key: String,
    pub source: String,
    pub verification_command: Option<String>,
    pub observed: String,
    pub expected: String,
    pub satisfied: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SystemPhaseCoverage {
    pub phase: u8,
    pub name: String,
    pub status: String,
    pub implementation_mode: String,
    pub current_anchor: Vec<String>,
    pub blocked_items: Vec<String>,
    pub verification: Vec<String>,
}

pub async fn run_doctor(config: AppConfig) -> anyhow::Result<DoctorReport> {
    run_doctor_with_bootstrap(config, false).await
}

pub async fn run_doctor_allow_bootstrap(config: AppConfig) -> anyhow::Result<DoctorReport> {
    run_doctor_with_bootstrap(config, true).await
}

async fn run_doctor_with_bootstrap(
    config: AppConfig,
    allow_bootstrap: bool,
) -> anyhow::Result<DoctorReport> {
    config.validate().map_err(anyhow::Error::msg)?;

    let base_url = config
        .doctor_base_url()
        .map(|base_url| provider_base_url_shape(&base_url));
    let model = config.doctor_model();

    let mut database_lifecycle = inspect_database(&config.database_url).await?;
    if allow_bootstrap {
        database_lifecycle = match database_lifecycle.status.as_str() {
            "missing" => initialize_database(&config.database_url).await?,
            "current" => database_lifecycle,
            _ => migrate_database(&config.database_url).await?,
        };
        database_lifecycle.operation = "doctor_allow_bootstrap".to_string();
        if !database_lifecycle.bootstrap_performed {
            database_lifecycle.message =
                "database already current; bootstrap permission was not used".to_string();
        }
    }
    let operation_log = match config.transport {
        TransportKind::Stdio => open_read_only_current_database(&config.database_url).await?,
    };
    let retrieval_index = match operation_log.as_ref() {
        Some(store) => Some(store.inspect_retrieval_index().await?),
        None => None,
    };
    let daemon_observe_only =
        build_daemon_observe_only_diagnostics(&config, operation_log.as_ref()).await;
    let provider_matrix = build_provider_matrix(&config);
    let remote_team_capability_inventory = remote_team_capability_inventory();
    let remote_team_security_gates = remote_team_security_gate_report();
    let system_layer_report =
        build_system_layer_report(&daemon_observe_only, &remote_team_security_gates);

    Ok(DoctorReport {
        transport: config.transport,
        database_url: config.database_url,
        status: if database_lifecycle.is_current()
            && retrieval_index
                .as_ref()
                .is_none_or(|index| index.is_usable())
        {
            "ok"
        } else {
            "attention_required"
        },
        database_lifecycle,
        retrieval_index,
        provider: config.model_provider,
        base_url,
        model,
        provider_matrix,
        dashboard_enabled: config.dashboard.enabled,
        dashboard_host: config.dashboard.host,
        dashboard_port: config.dashboard.port,
        dashboard_base_path: config.dashboard.base_path,
        dashboard_required: config.dashboard.required,
        daemon_enabled: config.daemon.enabled,
        daemon_poll_interval_ms: config.daemon.poll_interval_ms,
        daemon_max_concurrent_tasks: config.daemon.max_concurrent_tasks,
        daemon_observe_only,
        remote_team_capability_inventory,
        remote_team_security_gates,
        system_layer_report,
        auto_reflection_runtime_hooks: interfaces::mcp::server::AUTO_REFLECTION_RUNTIME_HOOKS
            .iter()
            .map(|hook| hook.to_string())
            .collect(),
        self_revision_write_path: interfaces::mcp::server::SELF_REVISION_WRITE_PATH,
    })
}

fn build_system_layer_report(
    daemon_observe_only: &DaemonObserveOnlyDiagnostics,
    remote_team_security_gates: &RemoteTeamSecurityGateReport,
) -> SystemLayerReport {
    let security_gate_names = remote_team_security_gates.blocked_gate_names().join(", ");
    let remote_writes_blocker = if remote_team_security_gates.remote_writes_allowed {
        "remote/team writes remain outside this read-only report".to_string()
    } else {
        format!("security/auth gates block remote/team writes: {security_gate_names}")
    };
    let daemon_writes_blocker = if daemon_observe_only.writes_allowed {
        "daemon writes remain outside this read-only report".to_string()
    } else {
        "daemon writes are blocked; observe-only diagnostics do not create write authority"
            .to_string()
    };

    SystemLayerReport {
        read_only: true,
        writes_performed: false,
        layers: vec![
            system_layer_entry(
                "substrate",
                "partial",
                "local process, config, SQLite storage, migrations, and platform entrypoints",
                ["scripts/agent-llm-mm.sh", "scripts/agent-llm-mm.ps1"],
                [
                    "Local Alpha candidate evidence must be refreshed per candidate before product claims",
                    "real fresh-machine evidence is not proven by local simulation",
                    "Windows parity runtime evidence is still missing",
                    "human release decision evidence is still missing",
                ],
                [
                    system_layer_diagnostic(
                        "config_shape",
                        "implemented",
                        "doctor reports provider, dashboard, daemon, and transport configuration shape without exposing secrets",
                    ),
                    system_layer_diagnostic(
                        "database_path_shape",
                        "implemented",
                        "doctor reports the configured database_url shape; support bundles must not export raw SQLite files",
                    ),
                    system_layer_diagnostic(
                        "platform_entrypoint",
                        "implemented",
                        "macOS uses scripts/agent-llm-mm.sh and Windows uses scripts/agent-llm-mm.ps1",
                    ),
                    system_layer_diagnostic(
                        "data_lifecycle_gate_status",
                        "partial",
                        "local backup/restore gates exist, but real fresh-machine and Windows runtime evidence remain open",
                    ),
                ],
            ),
            system_layer_entry(
                "signal",
                "implemented",
                "raw local observations, operation log metadata, trigger candidates, and bounded evidence handles",
                ["ingest_interaction", "operation_log", "evidence_relation_report"],
                ["richer signal ranking and widening remain outside this report"],
                [
                    system_layer_diagnostic(
                        "evidence_relation_report",
                        "implemented",
                        "read-only evidence relation report exposes selected / available-not-selected rows inside selected_subset_of_trigger_window",
                    ),
                    system_layer_diagnostic(
                        "evidence_relation_weight_policy",
                        "implemented",
                        "bounded_selected_binary_weight is binary selection metadata, not a full scoring or ranking engine",
                    ),
                ],
            ),
            system_layer_entry(
                "memory",
                "partial",
                "derived claims, snapshots, episode projections, and read-only memory-layer labels",
                ["build_self_snapshot", "memory_layer_projection"],
                [
                    "memory layering is partial; legacy snapshot projection does not include the separate v5 inert experience-candidate runtime",
                    "future memory layers need migration, lifecycle, and evidence-link gates",
                ],
                [
                    system_layer_diagnostic(
                        "layered_projection_classification",
                        "partial",
                        "legacy snapshot-only working / episodic / semantic / self_model labels report partial with evidence; its procedural label does not describe the separate v5 experience candidate API",
                    ),
                    system_layer_diagnostic(
                        "experience_candidate_runtime",
                        "implemented",
                        "schema-v5 explicit Episode and semantic/procedural candidate tools support scoped evidence, versioned reject/revise/rollback and active-only recall; activation grants no execution or permission",
                    ),
                    system_layer_diagnostic(
                        "self_model_durable_writes",
                        "blocked",
                        "self_model layer stays read-only; run_reflection is the only durable identity / commitment write path",
                    ),
                ],
            ),
            system_layer_entry(
                "policy",
                "implemented",
                "local governance rules, product wording guardrails, commitment gate, and remote/team security gates",
                ["decide_with_snapshot", "product_wording_guard"],
                [remote_writes_blocker.as_str()],
                [],
            ),
            system_layer_entry(
                "control_loop",
                "partial",
                "local feedback hooks that observe, decide, suppress, cool down, and propose",
                ["auto_reflect_if_needed", "trigger_ledger"],
                [daemon_writes_blocker.as_str()],
                [],
            ),
            system_layer_entry(
                "actuator",
                "implemented",
                "durable semantic write path for governed identity, commitment, and reflection updates",
                [interfaces::mcp::server::SELF_REVISION_WRITE_PATH],
                [
                    "this read-only report does not allow writes",
                    "future daemon writes require a separate architecture decision, migration, and rollback gate",
                ],
                [],
            ),
            system_layer_entry(
                "interface",
                "partial",
                "MCP stdio, local dashboard, support bundle, doctor output, and release scripts",
                ["mcp_stdio", "local_dashboard", "support_bundle", "doctor"],
                [
                    "remote/team interfaces are blocked until product and security gates exist",
                    "write-capable interface behavior must pass through run_reflection or a later approved write-path decision",
                ],
                [],
            ),
            system_layer_entry(
                "release_boundary",
                "partial",
                "release evidence, platform parity, compatibility, product wording, and human release decision gates",
                [
                    "local_alpha_evidence_summary",
                    "release_gate",
                    "status_sync_check",
                ],
                [
                    "Local Alpha is not complete without fresh evidence and a human release decision",
                    "Windows parity and fresh-machine evidence remain open",
                ],
                [],
            ),
        ],
        evidence_relation_contract: evidence_relation_contract(),
        physics_principles: physics_principle_mappings(),
        dependency_rules: system_dependency_rules(daemon_observe_only, remote_team_security_gates),
        phase_coverage: system_phase_coverage(),
        blockers: vec![
            "Local Alpha full release gate still needs fresh evidence and human decision".to_string(),
            "Windows parity runtime evidence is missing".to_string(),
            "fresh-machine evidence is missing; local simulation is not enough".to_string(),
            "remote/team behavior is blocked and not implemented".to_string(),
            daemon_writes_blocker,
            remote_writes_blocker,
            "memory layering remains partial; v5 experience candidates are inert and separate from legacy snapshot projections".to_string(),
        ],
        non_claims: vec![
            "not a physics solver".to_string(),
            "not a constraint optimizer".to_string(),
            "not a physical controller".to_string(),
            "not scientific validation evidence".to_string(),
            "not complete multi-layer cognition".to_string(),
            "not a remote/team product".to_string(),
            "not Local Alpha, Beta, GA, or production-ready certification".to_string(),
        ],
    }
}

fn evidence_relation_contract() -> EvidenceRelationContract {
    EvidenceRelationContract {
        protocol_version: EVIDENCE_RELATION_PROTOCOL_VERSION,
        read_only: true,
        writes_performed: false,
        grants_capability: false,
        no_widening_policy: EVIDENCE_RELATION_NO_WIDENING_POLICY,
        weight_policy: EVIDENCE_RELATION_WEIGHT_POLICY,
        relation_statuses: vec![
            EVIDENCE_RELATION_SELECTED_STATUS.to_string(),
            EVIDENCE_RELATION_AVAILABLE_NOT_SELECTED_STATUS.to_string(),
        ],
        selected_weight: SELECTED_EVIDENCE_WEIGHT,
        available_not_selected_weight: UNSELECTED_EVIDENCE_WEIGHT,
        additive_v2_fields: [
            "rejected_count",
            "weight_policy",
            "relation_status",
            "selection_weight",
            "rejection_reason",
        ]
        .iter()
        .map(|field| field.to_string())
        .collect(),
    }
}

fn system_layer_entry<const A: usize, const B: usize>(
    name: &str,
    status: &str,
    responsibility: &str,
    anchors: [&str; A],
    blockers: [&str; B],
    diagnostics: impl IntoIterator<Item = SystemLayerDiagnostic>,
) -> SystemLayerEntry {
    SystemLayerEntry {
        name: name.to_string(),
        status: status.to_string(),
        responsibility: responsibility.to_string(),
        anchors: anchors.iter().map(|anchor| anchor.to_string()).collect(),
        writes_allowed: false,
        blockers: blockers.iter().map(|blocker| blocker.to_string()).collect(),
        diagnostics: diagnostics.into_iter().collect(),
    }
}

fn system_layer_diagnostic(key: &str, status: &str, detail: &str) -> SystemLayerDiagnostic {
    SystemLayerDiagnostic {
        key: key.to_string(),
        status: status.to_string(),
        detail: detail.to_string(),
    }
}

fn physics_principle_mappings() -> Vec<PhysicsPrincipleMapping> {
    vec![
        physics_principle_mapping(
            "causality",
            "effects must have traceable causes",
            "semantic writes, gate status changes, and release claims must link to evidence or a human decision",
            "run_reflection audit and release evidence gates",
        ),
        physics_principle_mapping(
            "conservation",
            "state cannot appear from nowhere",
            "planned or simulated capabilities stay blocked until code, tests, docs, and fresh evidence exist",
            "follow-up reality gates and status-sync-check",
        ),
        physics_principle_mapping(
            "arrow_of_time",
            "release evidence is ordered and dated",
            "stale MVP evidence cannot certify later Local Alpha, Beta, or GA claims",
            "local_alpha_evidence_summary and release decision artifacts",
        ),
        physics_principle_mapping(
            "locality",
            "interactions happen through bounded local interfaces",
            "the MCP stdio core stays local and new surfaces must remain bounded",
            "MCP stdio, local dashboard, support bundle, and doctor",
        ),
        physics_principle_mapping(
            "feedback_control",
            "stable systems separate sensors, controllers, and actuators",
            "observe-only daemon diagnostics must not become write authority",
            "daemon_observe_only and run_reflection boundary",
        ),
        physics_principle_mapping(
            "entropy_increase",
            "complexity grows unless bounded",
            "status labels, drift checks, and small slices prevent plan and reality divergence",
            "status_sync_check and product wording guard",
        ),
        physics_principle_mapping(
            "energy_budget",
            "work is constrained by available evidence and verification budget",
            "prefer read-only reports and diagnostics before increasing write power or distribution scope",
            "system_layer_report",
        ),
        physics_principle_mapping(
            "boundary_conditions",
            "system behavior depends on external constraints",
            "remote/team and security-sensitive work stays blocked until auth, authorization, audit, rate limit, tenant isolation, rollback, and threat-model gates exist",
            "remote_team_security_gates",
        ),
    ]
}

fn physics_principle_mapping(
    principle: &str,
    engineering_interpretation: &str,
    structural_rule: &str,
    implementation_anchor: &str,
) -> PhysicsPrincipleMapping {
    PhysicsPrincipleMapping {
        principle: principle.to_string(),
        engineering_interpretation: engineering_interpretation.to_string(),
        structural_rule: structural_rule.to_string(),
        implementation_anchor: implementation_anchor.to_string(),
        report_only: true,
        grants_capability: false,
    }
}

fn system_dependency_rules(
    daemon_observe_only: &DaemonObserveOnlyDiagnostics,
    remote_team_security_gates: &RemoteTeamSecurityGateReport,
) -> Vec<SystemDependencyRule> {
    vec![
        system_dependency_rule(
            "actuator_has_no_dashboard_or_release_dependency",
            "Actuator must not depend on dashboard or release tooling.",
            [
                runtime_dependency_evidence(
                    "actuator_write_path",
                    interfaces::mcp::server::SELF_REVISION_WRITE_PATH,
                    "run_reflection",
                ),
                declared_test_contract_dependency_evidence(
                    "dashboard_or_release_grants_actuator",
                    "cargo test --test product_completion_read_models -v",
                ),
            ],
            ["cargo test --test product_completion_read_models -v"],
        ),
        system_dependency_rule(
            "write_capable_interface_requires_run_reflection_or_adr",
            "Write-capable interfaces must pass through run_reflection or a later approved write-path ADR.",
            [
                runtime_dependency_evidence(
                    "self_revision_write_path",
                    interfaces::mcp::server::SELF_REVISION_WRITE_PATH,
                    "run_reflection",
                ),
                runtime_dependency_evidence(
                    "remote_writes_allowed",
                    bool_text(remote_team_security_gates.remote_writes_allowed),
                    "false",
                ),
            ],
            ["./scripts/agent-llm-mm.sh doctor"],
        ),
        system_dependency_rule(
            "observe_only_daemon_must_not_call_actuator",
            "Observe-only daemon diagnostics may read candidates but must not call the actuator.",
            [
                runtime_dependency_evidence(
                    "run_reflection_allowed_from_daemon",
                    bool_text(daemon_observe_only.run_reflection_allowed_from_daemon),
                    "false",
                ),
                runtime_dependency_evidence(
                    "daemon_writes_allowed",
                    bool_text(daemon_observe_only.writes_allowed),
                    "false",
                ),
                runtime_dependency_evidence(
                    "candidate_reads_are_read_only",
                    bool_text(daemon_observe_only.candidate_reads_are_read_only),
                    "true",
                ),
            ],
            ["cargo test --test daemon_config -v"],
        ),
        system_dependency_rule(
            "release_boundary_cannot_generate_external_evidence",
            "Release tooling can summarize evidence but cannot fabricate Windows runtime parity, real fresh-machine evidence, remote/team readiness, or a human release decision.",
            [
                declared_test_contract_dependency_evidence(
                    "local_refresh_does_not_generate_windows_parity",
                    "cargo test --test local_alpha_release_evidence release_gate_refresh -v",
                ),
                declared_test_contract_dependency_evidence(
                    "local_refresh_does_not_generate_real_fresh_machine",
                    "cargo test --test local_alpha_release_evidence release_gate_refresh -v",
                ),
                declared_test_contract_dependency_evidence(
                    "release_soak_keeps_remote_team_external",
                    "cargo test --test local_alpha_release_evidence release_soak -v",
                ),
                declared_test_contract_dependency_evidence(
                    "release_soak_keeps_human_decision_external",
                    "cargo test --test local_alpha_release_evidence release_soak -v",
                ),
            ],
            [
                "cargo test --test local_alpha_release_evidence release_gate_refresh -v",
                "cargo test --test local_alpha_release_evidence release_soak -v",
            ],
        ),
        system_dependency_rule(
            "memory_writes_require_migration_and_lifecycle_gates",
            "Future durable memory layers require migration, lifecycle, evidence-link, rollback, and write-path review gates.",
            [
                declared_test_contract_dependency_evidence(
                    "durable_new_memory_layer_writes_allowed",
                    "cargo test --test product_completion_read_models -v",
                ),
                runtime_dependency_evidence(
                    "durable_self_model_write_path",
                    interfaces::mcp::server::SELF_REVISION_WRITE_PATH,
                    "run_reflection",
                ),
            ],
            ["cargo test --test sqlite_backup_restore -v"],
        ),
    ]
}

fn system_dependency_rule<const E: usize, const V: usize>(
    name: &str,
    rule: &str,
    evidence: [SystemDependencyEvidence; E],
    verification: [&str; V],
) -> SystemDependencyRule {
    let evidence = evidence.to_vec();
    let status = if evidence.iter().any(|item| !item.satisfied) {
        "open"
    } else if evidence
        .iter()
        .any(|item| item.source == "declared_test_contract")
    {
        "declared-test-contract"
    } else {
        "enforced"
    };

    SystemDependencyRule {
        name: name.to_string(),
        rule: rule.to_string(),
        enforced_as: "read-only-boundary".to_string(),
        status: status.to_string(),
        grants_capability: false,
        evidence,
        verification: verification
            .iter()
            .map(|command| command.to_string())
            .collect(),
    }
}

fn runtime_dependency_evidence(
    key: &str,
    observed: impl Into<String>,
    expected: impl Into<String>,
) -> SystemDependencyEvidence {
    system_dependency_evidence(key, "runtime", None, observed, expected)
}

fn declared_test_contract_dependency_evidence(
    key: &str,
    verification: &str,
) -> SystemDependencyEvidence {
    system_dependency_evidence(
        key,
        "declared_test_contract",
        Some(verification.to_string()),
        "verification_declared",
        "verification_declared",
    )
}

fn system_dependency_evidence(
    key: &str,
    source: &str,
    verification_command: Option<String>,
    observed: impl Into<String>,
    expected: impl Into<String>,
) -> SystemDependencyEvidence {
    let observed = observed.into();
    let expected = expected.into();
    let satisfied = observed == expected;
    SystemDependencyEvidence {
        key: key.to_string(),
        source: source.to_string(),
        verification_command,
        observed,
        expected,
        satisfied,
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn system_phase_coverage() -> Vec<SystemPhaseCoverage> {
    vec![
        system_phase(
            0,
            "Preserve Baseline And Review Gates",
            "implemented",
            "local read-only drift and boundary checks",
            ["status_sync_check", "doctor", "product_wording_guard"],
            ["fresh evidence must still be rerun before new release claims"],
            [
                "git diff --check",
                "./scripts/status-sync-check.sh",
                "./scripts/agent-llm-mm.sh doctor",
            ],
        ),
        system_phase(
            1,
            "Read-Only Architecture Reports",
            "implemented",
            "doctor.system_layer_report exposes layer, law, dependency, phase, and non-claim boundaries",
            ["doctor.system_layer_report"],
            [
                "does not grant daemon writes, remote/team behavior, provider adapters, or durable memory layer writes",
            ],
            [
                "cargo test --test product_completion_read_models -v",
                "./scripts/agent-llm-mm.sh doctor",
            ],
        ),
        system_phase(
            2,
            "Local Alpha Evidence Completion",
            "partial",
            "local evidence refresh and source-only decision templates",
            [
                "local_alpha_evidence_summary",
                "product_readiness_check",
                "release_decision_local",
            ],
            [
                "real fresh-machine evidence is missing",
                "Windows runtime parity evidence is missing",
                "human release decision is missing",
            ],
            [
                "./scripts/local-alpha-release-gate-refresh.sh",
                "./scripts/product-readiness-check.sh <candidate-name>",
            ],
        ),
        system_phase(
            3,
            "Signal And Evidence Semantics",
            "partial",
            "bounded no-widening read-only relation projection",
            ["evidence_relation_report", "failure_modes"],
            ["full ranking and weighting engine is not implemented"],
            [
                "cargo test --test evidence_query_dto -v",
                "cargo test --test failure_modes -v",
            ],
        ),
        system_phase(
            4,
            "Memory Layering, Still Local And Gated",
            "partial",
            "read-only episode and layered memory projections",
            ["episode_summary_projection", "memory_layer_projection"],
            [
                "procedural execution is not implemented; v5 procedural candidates remain inert data",
                "durable self-model writes require migration, lifecycle, and rollback gates",
            ],
            [
                "cargo test --test product_completion_read_models -v",
                "cargo test --test sqlite_backup_restore -v",
            ],
        ),
        system_phase(
            5,
            "Observe-Only Daemon Stabilization",
            "partial",
            "observe-only diagnostics and config-gated local lifecycle with daemon writes and remote listener closed",
            ["daemon_observe_only"],
            [
                "daemon-triggered writes are blocked",
                "write-capable daemon loop is blocked",
                "all-entry automatic self-reflection is blocked",
            ],
            [
                "cargo test --test daemon_config -v",
                "cargo test --test mcp_stdio serve_starts_observe_only_daemon_when_enabled_without_semantic_writes -v",
                "./scripts/agent-llm-mm.sh doctor",
            ],
        ),
        system_phase(
            6,
            "Provider Expansion",
            "partial",
            "mock, openai-compatible, and openrouter are locally supported; future provider rows remain rejected",
            ["provider_matrix"],
            [
                "Azure OpenAI and local providers remain planned-only",
                "live provider evidence remains preflight-only and does not certify provider quality",
                "provider gateway behavior is not implemented",
            ],
            [
                "cargo test --test provider_config -v",
                "./scripts/agent-llm-mm.sh doctor",
            ],
        ),
        system_phase(
            7,
            "Remote/Team/Security Foundation",
            "planning-gate",
            "blocked inventory and unsatisfied security gate contracts",
            [
                "remote_team_capability_inventory",
                "remote_team_security_gates",
            ],
            [
                "remote write admin is blocked",
                "team shared memory is blocked",
                "tenant isolation is not implemented",
                "auth, authorization, audit, rate limit, and rollback are not implemented",
            ],
            [
                "cargo test --test product_completion_read_models -v",
                "./scripts/product-readiness-check.sh <candidate-name>",
            ],
        ),
        system_phase(
            8,
            "Release Engineering And Beta/GA Readiness",
            "partial",
            "local source-only soak and candidate evidence diagnostics",
            ["release_soak_local", "product_readiness_check"],
            [
                "installer is not implemented",
                "binary package and service manager are not implemented",
                "compatibility matrix automation is not implemented",
                "Beta/GA claims are blocked",
            ],
            [
                "bash -n scripts/release-soak-local.sh",
                "cargo test --test local_alpha_release_evidence release_soak -v",
            ],
        ),
    ]
}

fn system_phase<const A: usize, const B: usize, const V: usize>(
    phase: u8,
    name: &str,
    status: &str,
    implementation_mode: &str,
    current_anchor: [&str; A],
    blocked_items: [&str; B],
    verification: [&str; V],
) -> SystemPhaseCoverage {
    SystemPhaseCoverage {
        phase,
        name: name.to_string(),
        status: status.to_string(),
        implementation_mode: implementation_mode.to_string(),
        current_anchor: current_anchor
            .iter()
            .map(|anchor| anchor.to_string())
            .collect(),
        blocked_items: blocked_items.iter().map(|item| item.to_string()).collect(),
        verification: verification
            .iter()
            .map(|command| command.to_string())
            .collect(),
    }
}

fn build_provider_matrix(config: &AppConfig) -> Vec<DoctorProviderMatrixEntry> {
    AppConfig::provider_matrix()
        .into_iter()
        .map(|entry| doctor_provider_matrix_entry(config, entry))
        .collect()
}

fn doctor_provider_matrix_entry(
    config: &AppConfig,
    entry: ProviderMatrixEntry,
) -> DoctorProviderMatrixEntry {
    let support_state = match entry.state {
        "supported" => ProviderSupportState::Supported,
        _ => ProviderSupportState::PlannedOnly,
    };
    DoctorProviderMatrixEntry {
        provider: entry.provider,
        support_state,
        configurable: entry.configurable,
        adapter: entry.adapter,
        missing_implementation: entry.missing_implementation,
        selected: entry.provider == config.model_provider.as_str(),
        selectable_for_runtime: support_state == ProviderSupportState::Supported
            && entry.configurable,
    }
}

fn provider_base_url_shape(base_url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(base_url) else {
        return "<redacted>".to_string();
    };
    let Some(host) = parsed.host_str() else {
        return "<redacted>".to_string();
    };

    let mut shaped = format!("{}://{}", parsed.scheme(), host);
    if let Some(port) = parsed.port() {
        shaped.push(':');
        shaped.push_str(&port.to_string());
    }
    if parsed.path() != "/" {
        shaped.push_str("/<redacted-path>");
    }
    shaped
}

async fn build_daemon_observe_only_diagnostics(
    config: &AppConfig,
    operation_log: Option<&SqliteStore>,
) -> DaemonObserveOnlyDiagnostics {
    let mut read_errors = Vec::new();
    let (trigger_candidates_observed, trigger_candidates_suppressed) = if config.daemon.enabled {
        if let Some(operation_log) = operation_log {
            (
                count_trigger_candidates(
                    operation_log,
                    OperationLogStatus::Failed,
                    &mut read_errors,
                )
                .await,
                count_trigger_candidates(
                    operation_log,
                    OperationLogStatus::Suppressed,
                    &mut read_errors,
                )
                .await,
            )
        } else {
            read_errors.push(
                "operation_log:unavailable:database is not current; read-only doctor did not bootstrap"
                    .to_string(),
            );
            (0, 0)
        }
    } else {
        (0, 0)
    };

    DaemonObserveOnlyDiagnostics {
        mode: "observe_only",
        local_only: true,
        write_gate_approved: false,
        writes_allowed: false,
        remote_listener_enabled: false,
        write_blockers: vec![
            "daemon write gate is not approved; run_reflection remains the only durable write path"
                .to_string(),
            "observe-only diagnostics must not write identity, commitments, claims, events, or reflections".to_string(),
            "write-capable daemon gate is blocked; observe-only mode cannot call run_reflection"
                .to_string(),
        ],
        remote_blockers: vec![
            "remote listener is blocked until auth, authorization, audit, rollback, and tenant isolation gates exist".to_string(),
            "remote/team mode is not implemented in the local MVP".to_string(),
        ],
        data_sources: vec!["daemon_config".to_string(), "operation_log".to_string()],
        candidate_read_data_source: "operation_log",
        candidate_read_operation_kinds: vec![
            OperationLogKind::Tool.as_str(),
            OperationLogKind::Trigger.as_str(),
        ],
        candidate_read_statuses: vec![
            OperationLogStatus::Failed.as_str(),
            OperationLogStatus::Suppressed.as_str(),
        ],
        candidate_read_limit_per_kind_status: 25,
        candidate_reads_are_read_only: true,
        trigger_candidates_observed,
        trigger_candidates_suppressed,
        cooldown_status: "observe_only",
        cooldown_diagnostics: "diagnostic_only_no_scheduling",
        suppression_diagnostics: "read_only_status_count",
        clean_shutdown_status: "not_started_by_doctor",
        lifecycle_regression_status: "verified_by_handle_stop_test",
        semantic_writes_allowed: false,
        run_reflection_allowed_from_daemon: false,
        write_capable_daemon_gate_status: "blocked",
        background_autonomy_enabled: false,
        daemon_loop_connected: false,
        daemon_started_by_doctor: false,
        in_flight_task_count: 0,
        read_errors,
    }
}

async fn count_trigger_candidates(
    operation_log: &impl OperationLogStore,
    status: OperationLogStatus,
    read_errors: &mut Vec<String>,
) -> usize {
    let mut count = 0;
    for kind in [OperationLogKind::Tool, OperationLogKind::Trigger] {
        match operation_log
            .query_operations(OperationLogQuery {
                operation_kind: Some(kind.as_str().to_string()),
                status: Some(status.as_str().to_string()),
                limit: Some(25),
                ..Default::default()
            })
            .await
        {
            Ok(entries) => {
                count += entries.len();
            }
            Err(error) => read_errors.push(format!("operation_log:{}:{}", kind.as_str(), error)),
        }
    }
    count
}
