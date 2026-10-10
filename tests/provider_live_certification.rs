use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

use agent_llm_mm::support::{
    config::{AppConfig, ModelConfig, ModelProviderKind, OpenAiCompatibleConfig},
    provider_certification::{ProviderCertificationOptions, summarize_provider_certification},
    provider_live_certification::{
        ProviderLiveCertificationMode, ProviderLiveCertificationOptions,
        run_provider_live_certification,
    },
};
use serde_json::Value;
use tempfile::tempdir;

const EVIDENCE_FILES: &[(&str, &str)] = &[
    ("live-decision.json", "live_decision"),
    ("live-self-revision.json", "live_self_revision"),
    ("provider-error-handling.json", "provider_error_handling"),
    ("redaction-review.json", "redaction_review"),
];

#[test]
fn stub_evidence_runner_generates_non_live_provider_evidence_without_certifying_preflight() {
    let temp_dir = tempdir().expect("temp dir");
    let config = openrouter_config();

    let report = run_provider_live_certification(ProviderLiveCertificationOptions {
        config: config.clone(),
        evidence_root: temp_dir.path().to_path_buf(),
        mode: ProviderLiveCertificationMode::StubEvidence,
    })
    .expect("stub evidence generation should succeed");

    assert_eq!(report.provider, "openrouter");
    assert_eq!(report.status, "passed");
    assert_eq!(report.generated_evidence.len(), EVIDENCE_FILES.len());

    let evidence_dir = temp_dir
        .path()
        .join("target/reports/provider-certification/openrouter");
    for (file_name, evidence_kind) in EVIDENCE_FILES {
        let path = evidence_dir.join(file_name);
        assert!(path.exists(), "missing generated evidence file {file_name}");
        let bytes = fs::read(&path).expect("read evidence");
        assert!(!bytes.is_empty(), "evidence file must be non-empty JSON");
        let value: Value = serde_json::from_slice(&bytes).expect("evidence JSON");
        assert_eq!(value["provider"], "openrouter");
        assert_eq!(value["status"], "passed");
        assert_eq!(value["evidence_kind"], *evidence_kind);
        assert_eq!(value["local_only"], true);
        assert_eq!(value["mode"], "stub/simulated");
        assert_eq!(
            value["endpoint_shape"],
            "https://openrouter.example.test/<redacted-path>"
        );
        assert_eq!(value["credential_configured"], true);
        assert!(
            value["generated_at"]
                .as_str()
                .is_some_and(|stamp| !stamp.is_empty()),
            "generated_at must be present"
        );
        assert!(
            value["non_claims"]
                .as_array()
                .expect("non_claims array")
                .iter()
                .any(|claim| claim
                    .as_str()
                    .is_some_and(|claim| claim.contains("not real live provider evidence"))),
            "stub evidence must clearly avoid live-provider claims"
        );
    }

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config,
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert!(!summary.live_certified);
    assert_eq!(summary.live_certification_status, "blocked");
    assert_eq!(summary.missing_live_evidence.len(), EVIDENCE_FILES.len());
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "invalid"),
        "stub evidence must not be accepted as live certification evidence: {:?}",
        summary.live_evidence
    );
}

#[test]
fn stub_evidence_serialization_redacts_secrets_and_url_sensitive_parts() {
    let temp_dir = tempdir().expect("temp dir");
    let config = AppConfig {
        model_provider: ModelProviderKind::OpenAiCompatible,
        model_config: ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: "https://url-user:url-password@provider.example.test/api/sk-path-secret/v1?token=query-secret".to_string(),
            api_key: "api-secret-key".to_string(),
            model: "provider/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        ..Default::default()
    };

    run_provider_live_certification(ProviderLiveCertificationOptions {
        config,
        evidence_root: temp_dir.path().to_path_buf(),
        mode: ProviderLiveCertificationMode::StubEvidence,
    })
    .expect("stub evidence generation should succeed");

    let evidence_dir = temp_dir
        .path()
        .join("target/reports/provider-certification/openai-compatible");
    let serialized = EVIDENCE_FILES
        .iter()
        .map(|(file_name, _)| fs::read_to_string(evidence_dir.join(file_name)).expect("evidence"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(serialized.contains("https://provider.example.test/<redacted-path>"));
    for secret in [
        "url-user",
        "url-password",
        "sk-path-secret",
        "query-secret",
        "api-secret-key",
        "provider/test-model",
        "/api/",
    ] {
        assert!(
            !serialized.contains(secret),
            "live certification evidence must not serialize secret or provider-native fragment: {secret}"
        );
    }
}

#[test]
fn preflight_remains_blocked_when_live_evidence_has_not_been_generated() {
    let temp_dir = tempdir().expect("temp dir");

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: openrouter_config(),
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert!(!summary.live_certified);
    assert_eq!(summary.live_certification_status, "blocked");
    assert_eq!(summary.missing_live_evidence.len(), EVIDENCE_FILES.len());
}

#[test]
fn preflight_accepts_explicit_live_evidence_files_only() {
    let temp_dir = tempdir().expect("temp dir");
    write_live_evidence_files(
        temp_dir.path(),
        "scripts/provider-live-certification-run.sh --live",
    );

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: openrouter_config(),
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert!(summary.live_certified);
    assert_eq!(summary.live_certification_status, "passed");
    assert!(summary.missing_live_evidence.is_empty());
    assert!(
        !summary
            .non_claims
            .contains(&"not live provider certification".to_string()),
        "the narrow live_certified preflight flag must not be contradicted by broad non-claims"
    );
    for non_claim in [
        "not provider quality certification",
        "not provider SLA evidence",
        "not provider gateway certification",
        "not live decision quality evidence",
        "not live self-revision quality evidence",
        "not release approval",
    ] {
        assert!(
            summary.non_claims.contains(&non_claim.to_string()),
            "missing provider certification boundary: {non_claim}"
        );
    }
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "present")
    );
}

#[test]
fn provider_certification_summary_redacts_model_id_in_json_outputs() {
    let temp_dir = tempdir().expect("temp dir");
    write_live_evidence_files(
        temp_dir.path(),
        "scripts/provider-live-certification-run.sh --live",
    );
    let output_json_path = temp_dir.path().join("provider-certification-summary.json");

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: openrouter_config(),
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: Some(output_json_path.clone()),
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert_eq!(
        summary.provider_config_shape.model.as_deref(),
        Some("<redacted-model>")
    );
    let serialized_summary = serde_json::to_string(&summary).expect("summary JSON");
    let serialized_output = fs::read_to_string(output_json_path).expect("summary output JSON");
    for serialized in [serialized_summary, serialized_output] {
        for secret in ["openrouter/test-model", "openrouter-secret-key"] {
            assert!(
                !serialized.contains(secret),
                "provider certification summary must not expose provider secret or model id: {secret}"
            );
        }
    }
}

#[test]
fn preflight_keeps_live_certification_blocked_when_config_preflight_fails() {
    let temp_dir = tempdir().expect("temp dir");
    write_live_evidence_files(
        temp_dir.path(),
        "scripts/provider-live-certification-run.sh --live",
    );

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: invalid_openrouter_config(),
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert_eq!(summary.config_preflight_status, "failed");
    assert!(!summary.live_certified);
    assert_eq!(summary.live_certification_status, "blocked");
    assert!(summary.missing_live_evidence.is_empty());
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "present"),
        "live evidence remains independently reported as present: {:?}",
        summary.live_evidence
    );
}

#[test]
fn preflight_rejects_unsupported_command_evidence() {
    let temp_dir = tempdir().expect("temp dir");
    write_live_evidence_files(temp_dir.path(), "echo not-provider-live-certification");

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: openrouter_config(),
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert!(!summary.live_certified);
    assert_eq!(summary.live_certification_status, "blocked");
    assert_eq!(summary.missing_live_evidence.len(), EVIDENCE_FILES.len());
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "invalid"),
        "unsupported command evidence must not satisfy live certification: {:?}",
        summary.live_evidence
    );
}

#[test]
fn preflight_rejects_thin_self_labeled_live_evidence_files() {
    let temp_dir = tempdir().expect("temp dir");
    let evidence_dir = temp_dir
        .path()
        .join("target/reports/provider-certification/openrouter");
    fs::create_dir_all(&evidence_dir).expect("create provider evidence dir");
    for (file_name, evidence_kind) in EVIDENCE_FILES {
        fs::write(
            evidence_dir.join(file_name),
            serde_json::json!({
                "provider": "openrouter",
                "status": "passed",
                "evidence_kind": evidence_kind,
                "mode": "live",
                "generated_at": "2026-06-08T00:00:00Z",
                "local_only": false
            })
            .to_string(),
        )
        .expect("write thin live evidence");
    }

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: openrouter_config(),
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert!(!summary.live_certified);
    assert_eq!(summary.live_certification_status, "blocked");
    assert_eq!(summary.missing_live_evidence.len(), EVIDENCE_FILES.len());
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "invalid"),
        "thin self-labeled live files must remain invalid: {:?}",
        summary.live_evidence
    );
}

#[test]
fn preflight_rejects_minimal_spoofed_live_evidence_files() {
    let temp_dir = tempdir().expect("temp dir");
    let evidence_dir = temp_dir
        .path()
        .join("target/reports/provider-certification/openrouter");
    fs::create_dir_all(&evidence_dir).expect("create provider evidence dir");
    for (file_name, _) in EVIDENCE_FILES {
        fs::write(
            evidence_dir.join(file_name),
            serde_json::json!({
                "provider": "openrouter",
                "status": "passed"
            })
            .to_string(),
        )
        .expect("write spoof evidence");
    }

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: openrouter_config(),
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert!(!summary.live_certified);
    assert_eq!(summary.live_certification_status, "blocked");
    assert_eq!(summary.missing_live_evidence.len(), EVIDENCE_FILES.len());
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "invalid"),
        "minimal spoofed files must remain invalid: {:?}",
        summary.live_evidence
    );
}

#[test]
fn live_runner_generates_preflight_accepted_evidence_files() {
    let temp_dir = tempdir().expect("temp dir");
    let server = BlockingChatServer::spawn(vec![
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "provider_live_certification_decision_probe"
                }
            }]
        }),
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "{\"should_reflect\":false,\"rationale\":\"live certification self-revision parse probe\",\"machine_patch\":{\"identity_patch\":null,\"commitment_patch\":null}}"
                }
            }]
        }),
    ]);
    let config = openrouter_config_with_base_url(server.base_url());

    let report = run_provider_live_certification(ProviderLiveCertificationOptions {
        config: config.clone(),
        evidence_root: temp_dir.path().to_path_buf(),
        mode: ProviderLiveCertificationMode::Live,
    })
    .expect("live evidence generation should succeed against an explicit provider endpoint");

    assert_eq!(report.provider, "openrouter");
    assert_eq!(report.status, "passed");
    assert_eq!(report.mode, "live");
    assert_eq!(report.generated_evidence.len(), EVIDENCE_FILES.len());
    assert_eq!(server.request_count(), 2);
    assert!(
        server
            .request_paths()
            .iter()
            .all(|path| path == "/chat/completions"),
        "live runner should use the OpenAI-compatible chat completions path: {:?}",
        server.request_paths()
    );

    for (file_name, evidence_kind) in EVIDENCE_FILES {
        let path = temp_dir
            .path()
            .join("target/reports/provider-certification/openrouter")
            .join(file_name);
        assert!(
            path.exists(),
            "missing generated live evidence file {file_name}"
        );
        let value: Value =
            serde_json::from_slice(&fs::read(&path).expect("read evidence")).expect("json");
        assert_eq!(value["provider"], "openrouter");
        assert_eq!(value["status"], "passed");
        assert_eq!(value["evidence_kind"], *evidence_kind);
        assert_eq!(value["mode"], "live");
        assert_eq!(value["local_only"], false);
        assert_eq!(value["endpoint_reached"], true);
        assert_eq!(value["redaction_reviewed"], true);
        assert_eq!(value["request_outcome"], "passed");
        assert_eq!(
            value["command_evidence"][0]["command"],
            "./scripts/provider-live-certification-run.sh --live"
        );
    }

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config,
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight");

    assert!(summary.live_certified);
    assert_eq!(summary.live_certification_status, "passed");
    assert!(summary.missing_live_evidence.is_empty());
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "present"),
        "live runner evidence should satisfy preflight: {:?}",
        summary.live_evidence
    );
}

#[test]
fn live_evidence_serialization_redacts_provider_secrets_and_payloads() {
    let temp_dir = tempdir().expect("temp dir");
    let server = BlockingChatServer::spawn(vec![
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "provider_live_certification_decision_probe"
                }
            }]
        }),
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "{\"should_reflect\":false,\"rationale\":\"live redaction probe\",\"machine_patch\":{\"identity_patch\":null,\"commitment_patch\":null}}"
                }
            }]
        }),
    ]);
    let secret_base_url = format!(
        "http://{}:{}/api/sk-path-secret/v1",
        server.host(),
        server.port()
    );
    let config = AppConfig {
        model_provider: ModelProviderKind::OpenAiCompatible,
        model_config: ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: secret_base_url,
            api_key: "api-secret-key".to_string(),
            model: "provider/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        ..Default::default()
    };

    run_provider_live_certification(ProviderLiveCertificationOptions {
        config,
        evidence_root: temp_dir.path().to_path_buf(),
        mode: ProviderLiveCertificationMode::Live,
    })
    .expect("live evidence generation should succeed");

    let evidence_dir = temp_dir
        .path()
        .join("target/reports/provider-certification/openai-compatible");
    let serialized = EVIDENCE_FILES
        .iter()
        .map(|(file_name, _)| fs::read_to_string(evidence_dir.join(file_name)).expect("evidence"))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(serialized.contains("http://127.0.0.1"));
    assert!(serialized.contains("/<redacted-path>"));
    for secret in [
        "url-user",
        "url-password",
        "sk-path-secret",
        "query-secret",
        "api-secret-key",
        "provider/test-model",
        "provider_live_certification_decision_probe",
        "live redaction probe",
        "/api/",
    ] {
        assert!(
            !serialized.contains(secret),
            "live evidence must not serialize secret, model id, URL fragment, or raw provider payload: {secret}"
        );
    }
}

#[test]
fn live_provider_errors_do_not_serialize_raw_response_payloads() {
    let temp_dir = tempdir().expect("temp dir");
    let raw_error_payload = serde_json::json!({
        "error": "provider-raw-response-secret api-secret-key provider/test-model provider_live_certification_decision_probe"
    })
    .to_string();
    let server = FailingChatServer::spawn(500, raw_error_payload);
    let config = AppConfig {
        model_provider: ModelProviderKind::OpenAiCompatible,
        model_config: ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: server.base_url(),
            api_key: "api-secret-key".to_string(),
            model: "provider/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        ..Default::default()
    };

    let error = run_provider_live_certification(ProviderLiveCertificationOptions {
        config,
        evidence_root: temp_dir.path().to_path_buf(),
        mode: ProviderLiveCertificationMode::Live,
    })
    .expect_err("live provider error should fail closed");

    let message = error.to_string();
    assert!(
        message.contains("live decision path failed"),
        "error should preserve bounded failure provenance: {message}"
    );
    for secret in [
        "provider-raw-response-secret",
        "api-secret-key",
        "provider/test-model",
        "provider_live_certification_decision_probe",
    ] {
        assert!(
            !message.contains(secret),
            "live provider error must not serialize raw provider payload or secret: {secret}"
        );
    }
    assert_eq!(server.request_count(), 1);
}

#[test]
fn default_cli_mode_requires_explicit_live_or_stub_and_mock_live_is_rejected() {
    let temp_dir = tempdir().expect("temp dir");
    let api_error = run_provider_live_certification(ProviderLiveCertificationOptions {
        config: AppConfig::default(),
        evidence_root: temp_dir.path().to_path_buf(),
        mode: ProviderLiveCertificationMode::Live,
    })
    .expect_err("mock provider cannot produce live evidence");

    assert!(
        api_error
            .to_string()
            .contains("mock cannot produce live evidence"),
        "API error should keep mock from satisfying live provider evidence: {api_error}"
    );

    let binary = std::env::var("CARGO_BIN_EXE_provider_live_certification_run")
        .expect("provider_live_certification_run test binary path");
    let output = Command::new(binary)
        .arg("--evidence-root")
        .arg(temp_dir.path())
        .output()
        .expect("run provider live certification CLI");

    assert!(!output.status.success(), "default CLI live mode must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--live") && stderr.contains("--stub-evidence"),
        "CLI must require an explicit mode without pretending certification happened: {stderr}"
    );
}

#[test]
fn wrapper_rejects_conflicting_live_modes_before_treating_mode_as_path() {
    let output = Command::new("bash")
        .arg("scripts/provider-live-certification-run.sh")
        .arg("--live")
        .arg("--stub-evidence")
        .output()
        .expect("run provider live certification wrapper");

    assert!(
        !output.status.success(),
        "conflicting wrapper modes must fail"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("choose exactly one mode"),
        "wrapper should reject conflicting modes instead of treating the second mode as a config path: {stderr}"
    );
}

#[test]
fn explicit_live_flag_generates_evidence_by_cli_and_wrapper() {
    let temp_dir = tempdir().expect("temp dir");
    let server = BlockingChatServer::spawn(vec![
        live_decision_response(),
        live_self_revision_response(),
        live_decision_response(),
        live_self_revision_response(),
    ]);
    let config_path = temp_dir.path().join("config.toml");
    fs::write(
        &config_path,
        openrouter_config_toml_with_base_url(&server.base_url()),
    )
    .expect("write config");

    let binary = std::env::var("CARGO_BIN_EXE_provider_live_certification_run")
        .expect("provider_live_certification_run test binary path");
    let cli_output = Command::new(binary)
        .arg("--live")
        .arg("--config-path")
        .arg(&config_path)
        .arg("--evidence-root")
        .arg(temp_dir.path())
        .output()
        .expect("run provider live certification CLI");

    assert!(
        cli_output.status.success(),
        "explicit CLI live mode should generate bounded evidence: stderr={}",
        String::from_utf8_lossy(&cli_output.stderr)
    );
    let cli_stdout: Value =
        serde_json::from_slice(&cli_output.stdout).expect("CLI live report JSON");
    assert_eq!(cli_stdout["mode"], "live");
    assert_eq!(cli_stdout["status"], "passed");

    let wrapper_output = Command::new("bash")
        .arg("scripts/provider-live-certification-run.sh")
        .arg("--live")
        .arg(&config_path)
        .arg(temp_dir.path())
        .output()
        .expect("run provider live certification wrapper");

    assert!(
        wrapper_output.status.success(),
        "explicit wrapper live mode should generate bounded evidence: stderr={}",
        String::from_utf8_lossy(&wrapper_output.stderr)
    );
    let wrapper_stdout: Value =
        serde_json::from_slice(&wrapper_output.stdout).expect("wrapper live report JSON");
    assert_eq!(wrapper_stdout["mode"], "live");
    assert_eq!(wrapper_stdout["status"], "passed");
    assert_eq!(server.request_count(), 4);
}

fn openrouter_config() -> AppConfig {
    openrouter_config_with_base_url("https://openrouter.example.test/api/v1?token=query-secret")
}

fn openrouter_config_with_base_url(base_url: impl Into<String>) -> AppConfig {
    AppConfig {
        model_provider: ModelProviderKind::OpenRouter,
        model_config: ModelConfig::OpenRouter(OpenAiCompatibleConfig {
            base_url: base_url.into(),
            api_key: "openrouter-secret-key".to_string(),
            model: "openrouter/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        ..Default::default()
    }
}

fn invalid_openrouter_config() -> AppConfig {
    AppConfig {
        model_provider: ModelProviderKind::OpenRouter,
        model_config: ModelConfig::OpenRouter(OpenAiCompatibleConfig {
            base_url: "https://openrouter.example.test/api/v1?token=query-secret".to_string(),
            api_key: "".to_string(),
            model: "openrouter/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        ..Default::default()
    }
}

fn write_live_evidence_files(evidence_root: &std::path::Path, command: &str) {
    let evidence_dir = evidence_root.join("target/reports/provider-certification/openrouter");
    fs::create_dir_all(&evidence_dir).expect("create provider evidence dir");
    for (file_name, evidence_kind) in EVIDENCE_FILES {
        fs::write(
            evidence_dir.join(file_name),
            serde_json::to_vec_pretty(&serde_json::json!({
                "provider": "openrouter",
                "status": "passed",
                "evidence_kind": evidence_kind,
                "mode": "live",
                "generated_at": "2026-06-08T00:00:00Z",
                "local_only": false,
                "endpoint_reached": true,
                "redaction_reviewed": true,
                "request_outcome": "passed",
                "command_evidence": [
                    {
                        "name": "provider-live-certification",
                        "command": command,
                        "status": "passed",
                        "exit_code": 0
                    }
                ]
            }))
            .expect("json"),
        )
        .expect("write live evidence");
    }
}

fn openrouter_config_toml_with_base_url(base_url: &str) -> String {
    format!(
        r#"
[model]
provider = "openrouter"

[model.openrouter]
base_url = "{base_url}"
api_key = "openrouter-secret-key"
model = "openrouter/test-model"
"#
    )
}

fn live_decision_response() -> Value {
    serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "provider_live_certification_decision_probe"
            }
        }]
    })
}

fn live_self_revision_response() -> Value {
    serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "{\"should_reflect\":false,\"rationale\":\"live certification parse probe\",\"machine_patch\":{\"identity_patch\":null,\"commitment_patch\":null}}"
            }
        }]
    })
}

struct BlockingChatServer {
    host: String,
    port: u16,
    request_paths: Arc<Mutex<Vec<String>>>,
    expected_requests: usize,
    handle: Option<thread::JoinHandle<()>>,
}

impl BlockingChatServer {
    fn spawn(responses: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind server");
        let address = listener.local_addr().expect("local addr");
        let expected_requests = responses.len();
        let request_paths = Arc::new(Mutex::new(Vec::new()));
        let paths = Arc::clone(&request_paths);
        let handle = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                let path = read_request_path(&mut stream);
                paths.lock().expect("paths lock").push(path);
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        Self {
            host: address.ip().to_string(),
            port: address.port(),
            request_paths,
            expected_requests,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    fn host(&self) -> &str {
        &self.host
    }

    fn port(&self) -> u16 {
        self.port
    }

    fn request_count(&self) -> usize {
        self.request_paths.lock().expect("paths lock").len()
    }

    fn request_paths(&self) -> Vec<String> {
        self.request_paths.lock().expect("paths lock").clone()
    }
}

impl Drop for BlockingChatServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let handled_requests = self.request_count();
            for _ in handled_requests..self.expected_requests {
                if let Ok(mut stream) = TcpStream::connect((self.host.as_str(), self.port)) {
                    let _ = stream.write_all(
                        b"GET /__test_shutdown HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n",
                    );
                }
            }
            let _ = handle.join();
        }
    }
}

struct FailingChatServer {
    host: String,
    port: u16,
    request_paths: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl FailingChatServer {
    fn spawn(status: u16, body: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind failing server");
        let address = listener.local_addr().expect("local addr");
        let request_paths = Arc::new(Mutex::new(Vec::new()));
        let paths = Arc::clone(&request_paths);
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept failing request");
            let path = read_request_path(&mut stream);
            paths.lock().expect("paths lock").push(path);
            let response = format!(
                "HTTP/1.1 {status} Internal Server Error\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("write failing response");
        });

        Self {
            host: address.ip().to_string(),
            port: address.port(),
            request_paths,
            handle: Some(handle),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    fn request_count(&self) -> usize {
        self.request_paths.lock().expect("paths lock").len()
    }
}

impl Drop for FailingChatServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            if self.request_count() == 0
                && let Ok(mut stream) = TcpStream::connect((self.host.as_str(), self.port))
            {
                let _ = stream.write_all(
                    b"GET /__test_shutdown HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n",
                );
            }
            let _ = handle.join();
        }
    }
}

fn read_request_path(stream: &mut TcpStream) -> String {
    let mut buffer = [0_u8; 16 * 1024];
    let bytes_read = stream.read(&mut buffer).expect("read request");
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("<missing-path>")
        .to_string()
}

#[test]
fn native_providers_support_redacted_stub_evidence_but_reject_live_certification() {
    use agent_llm_mm::support::config::NativeModelConfig;
    for kind in [
        ModelProviderKind::OpenAiResponses,
        ModelProviderKind::Anthropic,
    ] {
        let directory = tempdir().expect("temp dir");
        let native = NativeModelConfig {
            base_url: "https://native.example.test/v1?token=secret-query".into(),
            api_key: "native-private-key".into(),
            model: "native-private-model".into(),
            timeout_ms: 1000,
            max_tokens: 2048,
            temperature: None,
        };
        let config = AppConfig {
            model_provider: kind,
            model_config: match kind {
                ModelProviderKind::OpenAiResponses => ModelConfig::OpenAiResponses(native),
                ModelProviderKind::Anthropic => ModelConfig::Anthropic(native),
                _ => unreachable!(),
            },
            ..AppConfig::default()
        };
        let report = run_provider_live_certification(ProviderLiveCertificationOptions {
            config: config.clone(),
            evidence_root: directory.path().into(),
            mode: ProviderLiveCertificationMode::StubEvidence,
        })
        .expect("native stub evidence");
        for evidence in report.generated_evidence {
            let text = fs::read_to_string(evidence.path).expect("read evidence");
            for secret in ["native-private-key", "native-private-model", "secret-query"] {
                assert!(!text.contains(secret));
            }
        }
        let summary = summarize_provider_certification(ProviderCertificationOptions {
            config: config.clone(),
            evidence_root: directory.path().into(),
            output_json_path: None,
            output_markdown_path: None,
        })
        .expect("native summary");
        assert!(!summary.live_certified);
        assert_eq!(summary.live_certification_status, "blocked");
        let error = run_provider_live_certification(ProviderLiveCertificationOptions {
            config,
            evidence_root: directory.path().into(),
            mode: ProviderLiveCertificationMode::Live,
        })
        .expect_err("native live runner must not silently route to chat completions");
        assert!(
            error
                .to_string()
                .contains("native provider live certification is not implemented")
        );
    }
}
