use std::fs;

use agent_llm_mm::support::{
    config::{AppConfig, ModelConfig, ModelProviderKind, OpenAiCompatibleConfig},
    packaging_preflight::{PackagingPreflightOptions, summarize_packaging_preflight},
    provider_certification::{ProviderCertificationOptions, summarize_provider_certification},
    release_evidence_index::{ReleaseEvidenceIndexOptions, build_release_evidence_index},
};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn release_evidence_index_lists_present_missing_and_blocked_candidate_evidence() {
    let temp_dir = tempdir().expect("temp dir");
    write_product_smoke_latest(temp_dir.path());
    write_first_run_simulation(temp_dir.path());
    write_support_bundle(temp_dir.path());

    let index = build_release_evidence_index(ReleaseEvidenceIndexOptions {
        evidence_root: temp_dir.path().to_path_buf(),
        release_candidate: "local-alpha-20260608.1-rc.1".to_string(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("release evidence index should build from partial evidence");

    assert_eq!(index.kind, "release_evidence_index");
    assert_eq!(index.release_candidate, "local-alpha-20260608.1-rc.1");
    assert!(index.local_only);
    assert!(!index.ready_for_human_review);
    assert!(index.missing_required_count > 0);
    assert!(index.blocked_gate_count > 0);
    assert_entry(&index, "product_smoke", "present");
    assert_entry(&index, "support_bundle", "present");
    assert_entry(&index, "windows_parity", "not_verified");
    assert_entry(&index, "release_decision", "blocked");
    assert_entry(&index, "remote_team", "blocked");
    assert!(
        index
            .non_claims
            .iter()
            .any(|claim| claim.contains("not release approval"))
    );
}

#[test]
fn release_evidence_index_preserves_blocked_product_wording_gate_status() {
    let temp_dir = tempdir().expect("temp dir");
    write_product_smoke_latest(temp_dir.path());
    write_first_run_simulation(temp_dir.path());
    write_support_bundle(temp_dir.path());

    let index = build_release_evidence_index(ReleaseEvidenceIndexOptions {
        evidence_root: temp_dir.path().to_path_buf(),
        release_candidate: "ga-production-ready-remote-team".to_string(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("release evidence index should build with blocked product wording");

    assert_entry(&index, "product_wording", "blocked");
    assert_eq!(
        index.blocked_gate_count,
        index
            .entries
            .iter()
            .filter(|entry| entry.status == "blocked")
            .count(),
        "blocked_gate_count should exactly match blocked index entries"
    );
}

#[test]
fn provider_certification_preflight_keeps_openrouter_live_certification_blocked_without_evidence() {
    let config = AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        model_provider: ModelProviderKind::OpenRouter,
        model_config: ModelConfig::OpenRouter(OpenAiCompatibleConfig {
            base_url: "https://user:password@openrouter.example.test/api/v1?token=query-secret"
                .to_string(),
            api_key: "openrouter-secret-key".to_string(),
            model: "openrouter/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        ..Default::default()
    };

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config,
        evidence_root: tempdir().expect("temp dir").path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight should summarize supported provider");

    assert_eq!(summary.kind, "provider_certification_summary");
    assert_eq!(summary.provider, "openrouter");
    assert_eq!(summary.config_preflight_status, "passed");
    assert!(!summary.live_certified);
    assert_eq!(summary.live_certification_status, "blocked");
    assert!(
        summary
            .missing_live_evidence
            .contains(&"live_decision_path".to_string())
    );

    let serialized = serde_json::to_string(&summary).expect("summary JSON");
    for secret in ["openrouter-secret-key", "user", "password", "query-secret"] {
        assert!(
            !serialized.contains(secret),
            "provider certification summary must not expose secret fragment: {secret}"
        );
    }
}

#[test]
fn provider_certification_preflight_redacts_path_secret_fragments() {
    let config = AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        model_provider: ModelProviderKind::OpenAiCompatible,
        model_config: ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: "https://provider.example.test/api/sk-path-secret/v1".to_string(),
            api_key: "provider-api-key".to_string(),
            model: "provider/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        ..Default::default()
    };

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config,
        evidence_root: tempdir().expect("temp dir").path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight should redact path-shaped secrets");

    let serialized = serde_json::to_string(&summary).expect("summary JSON");
    for secret in ["provider-api-key", "sk-path-secret"] {
        assert!(
            !serialized.contains(secret),
            "provider certification summary must not expose secret fragment: {secret}"
        );
    }
    assert_eq!(
        summary.provider_config_shape.base_url.as_deref(),
        Some("https://provider.example.test/<redacted-path>")
    );
}

#[test]
fn provider_certification_preflight_rejects_placeholder_live_evidence_files() {
    let temp_dir = tempdir().expect("temp dir");
    let evidence_dir = temp_dir
        .path()
        .join("target/reports/provider-certification/openrouter");
    fs::create_dir_all(&evidence_dir).expect("create provider evidence dir");
    fs::write(evidence_dir.join("live-decision.json"), "")
        .expect("write empty live evidence placeholder");
    fs::write(
        evidence_dir.join("live-self-revision.json"),
        json!({
            "provider": "different-provider",
            "status": "passed"
        })
        .to_string(),
    )
    .expect("write wrong-provider evidence placeholder");
    fs::write(
        evidence_dir.join("provider-error-handling.json"),
        json!({
            "provider": "openrouter",
            "status": "failed"
        })
        .to_string(),
    )
    .expect("write failed evidence placeholder");
    fs::write(evidence_dir.join("redaction-review.json"), "not json")
        .expect("write malformed evidence placeholder");

    let summary = summarize_provider_certification(ProviderCertificationOptions {
        config: AppConfig {
            local_embedding: Default::default(),
            retrieval: Default::default(),
            model_provider: ModelProviderKind::OpenRouter,
            model_config: ModelConfig::OpenRouter(OpenAiCompatibleConfig {
                base_url: "https://openrouter.example.test/api/v1".to_string(),
                api_key: "openrouter-secret-key".to_string(),
                model: "openrouter/test-model".to_string(),
                timeout_ms: 30_000,
            }),
            ..Default::default()
        },
        evidence_root: temp_dir.path().to_path_buf(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("provider certification preflight should classify placeholder evidence");

    assert_eq!(
        summary.missing_live_evidence,
        vec![
            "live_decision_path".to_string(),
            "live_self_revision_path".to_string(),
            "provider_error_handling".to_string(),
            "redaction_review".to_string()
        ]
    );
    assert!(
        summary
            .live_evidence
            .iter()
            .all(|entry| entry.status == "invalid")
    );
}

#[test]
fn packaging_preflight_keeps_release_packaging_blocked_until_artifacts_exist() {
    let temp_dir = tempdir().expect("temp dir");
    let summary = summarize_packaging_preflight(PackagingPreflightOptions {
        evidence_root: temp_dir.path().to_path_buf(),
        release_candidate: "local-alpha-20260608.1-rc.1".to_string(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("packaging preflight should summarize missing packaging artifacts");

    assert_eq!(summary.kind, "packaging_preflight_summary");
    assert_eq!(summary.release_candidate, "local-alpha-20260608.1-rc.1");
    assert!(!summary.packaging_ready);
    assert_eq!(summary.overall_status, "blocked");
    assert_blocker(&summary, "binary_archive", "missing");
    assert_blocker(&summary, "installer", "not_implemented");
    assert_blocker(&summary, "service_manager", "not_implemented");
    assert_blocker(&summary, "auto_updater", "not_implemented");
    assert!(
        summary
            .non_claims
            .iter()
            .any(|claim| claim.contains("not installer evidence"))
    );
}

#[test]
fn packaging_preflight_rejects_zero_byte_and_partial_binary_archives() {
    let temp_dir = tempdir().expect("temp dir");
    let packaging_dir = temp_dir
        .path()
        .join("target/reports/releases/local-alpha-20260608.1-rc.1/packaging");
    fs::create_dir_all(&packaging_dir).expect("create packaging evidence dir");
    fs::write(packaging_dir.join("agent-llm-mm-linux-x86_64.tar.gz"), [])
        .expect("write zero-byte archive placeholder");

    let summary = summarize_packaging_preflight(PackagingPreflightOptions {
        evidence_root: temp_dir.path().to_path_buf(),
        release_candidate: "local-alpha-20260608.1-rc.1".to_string(),
        output_json_path: None,
        output_markdown_path: None,
    })
    .expect("packaging preflight should classify placeholder archives");

    assert!(!summary.packaging_ready);
    assert_eq!(summary.overall_status, "blocked");
    assert_blocker(&summary, "binary_archive", "invalid");
    let blocker = summary
        .blockers
        .iter()
        .find(|blocker| blocker.subject == "binary_archive")
        .expect("binary archive blocker");
    assert!(blocker.reason.contains("zero-byte"));
    assert!(blocker.reason.contains("missing archives"));
}

fn assert_entry(
    index: &agent_llm_mm::support::release_evidence_index::ReleaseEvidenceIndex,
    name: &str,
    status: &str,
) {
    let entry = index
        .entries
        .iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| {
            panic!(
                "missing evidence index entry {name}; entries={:?}",
                index.entries
            )
        });
    assert_eq!(entry.status, status);
}

fn assert_blocker(
    summary: &agent_llm_mm::support::packaging_preflight::PackagingPreflightSummary,
    subject: &str,
    status: &str,
) {
    let blocker = summary
        .blockers
        .iter()
        .find(|blocker| blocker.subject == subject)
        .unwrap_or_else(|| {
            panic!(
                "missing packaging blocker {subject}; blockers={:?}",
                summary.blockers
            )
        });
    assert_eq!(blocker.status, status);
}

fn write_product_smoke_latest(root: &std::path::Path) {
    let output_dir = root.join("target/reports/self-revision-demo/latest");
    fs::create_dir_all(&output_dir).expect("create product smoke dir");
    for file in [
        "doctor.json",
        "snapshot-before.json",
        "snapshot-after.json",
        "decision-before.json",
        "decision-after.json",
        "timeline.json",
        "sqlite-summary.json",
        "report.md",
    ] {
        fs::write(output_dir.join(file), "evidence").expect("write product smoke evidence");
    }
}

fn write_first_run_simulation(root: &std::path::Path) {
    let output_dir = root.join("target/first-run-bootstrap-smoke/local-alpha-gate");
    fs::create_dir_all(&output_dir).expect("create first-run dir");
    fs::write(
        output_dir.join("summary.json"),
        serde_json::to_vec_pretty(&json!({
            "kind": "local_first_run_bootstrap_simulation",
            "local_only": true,
            "fresh_machine_simulation": true,
            "real_fresh_machine_evidence": false,
            "doctor_status": "ok",
            "self_revision_write_path": "run_reflection",
            "daemon_enabled": false,
            "daemon_writes_allowed": false,
            "sqlite_database_exists": true,
            "started_serve": false,
            "ran_product_smoke": false
        }))
        .expect("json"),
    )
    .expect("write first-run summary");
}

fn write_support_bundle(root: &std::path::Path) {
    let output_dir = root.join("target/support-bundles/local-alpha-gate");
    fs::create_dir_all(&output_dir).expect("create support bundle dir");
    for (file, value) in [
        (
            "manifest.json",
            json!({
                "bundle_format": "agent-llm-mm-local-alpha-support-bundle-v1",
                "local_only": true,
                "upload_performed": false
            }),
        ),
        (
            "doctor.json",
            json!({
                "status": "config-shape-ok",
                "self_revision_write_path": "run_reflection",
                "runtime_bootstrap_performed": false
            }),
        ),
        (
            "product-smoke-summary.json",
            json!({
                "required_artifacts": [
                    "doctor.json",
                    "snapshot-before.json",
                    "snapshot-after.json",
                    "decision-before.json",
                    "decision-after.json",
                    "timeline.json",
                    "sqlite-summary.json",
                    "report.md"
                ],
                "self_revision_write_path_expected": "run_reflection"
            }),
        ),
        ("config-shape.json", json!({ "ok": true })),
        ("operation-summaries.json", json!({ "ok": true })),
        ("release-metadata.json", json!({ "ok": true })),
        ("local-log-excerpts.json", json!({ "ok": true })),
    ] {
        fs::write(
            output_dir.join(file),
            serde_json::to_vec_pretty(&value).expect("json"),
        )
        .expect("write support bundle file");
    }
}
