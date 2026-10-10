use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use serde_json::Value;

use crate::support::config::{AppConfig, ModelConfig};

const REQUIRED_LIVE_EVIDENCE: &[(&str, &str, &str)] = &[
    (
        "live_decision_path",
        "target/reports/provider-certification/{provider}/live-decision.json",
        "live_decision",
    ),
    (
        "live_self_revision_path",
        "target/reports/provider-certification/{provider}/live-self-revision.json",
        "live_self_revision",
    ),
    (
        "provider_error_handling",
        "target/reports/provider-certification/{provider}/provider-error-handling.json",
        "provider_error_handling",
    ),
    (
        "redaction_review",
        "target/reports/provider-certification/{provider}/redaction-review.json",
        "redaction_review",
    ),
];

#[derive(Debug, Clone)]
pub struct ProviderCertificationOptions {
    pub config: AppConfig,
    pub evidence_root: PathBuf,
    pub output_json_path: Option<PathBuf>,
    pub output_markdown_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderCertificationSummary {
    pub generated_at: String,
    pub kind: &'static str,
    pub provider: String,
    pub config_preflight_status: &'static str,
    pub config_preflight_error: Option<String>,
    pub provider_config_shape: ProviderConfigShape,
    pub live_certified: bool,
    pub live_certification_status: &'static str,
    pub missing_live_evidence: Vec<String>,
    pub live_evidence: Vec<ProviderCertificationEvidence>,
    pub non_claims: Vec<String>,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderConfigShape {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub timeout_ms: Option<u64>,
    pub credential_configured: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderCertificationEvidence {
    pub name: &'static str,
    pub status: &'static str,
    pub evidence_path: String,
}

pub fn summarize_provider_certification(
    options: ProviderCertificationOptions,
) -> Result<ProviderCertificationSummary> {
    let provider = options.config.model_provider.as_str().to_string();
    let config_preflight_error = options.config.validate().err();
    let config_preflight_status = if config_preflight_error.is_some() {
        "failed"
    } else {
        "passed"
    };
    let live_evidence = live_evidence(&provider, &options.evidence_root);
    let missing_live_evidence = live_evidence
        .iter()
        .filter(|entry| entry.status != "present")
        .map(|entry| entry.name.to_string())
        .collect::<Vec<_>>();
    let native_without_live_runner = matches!(
        options.config.model_config,
        ModelConfig::OpenAiResponses(_) | ModelConfig::Anthropic(_)
    );
    let live_certified = !native_without_live_runner
        && config_preflight_error.is_none()
        && missing_live_evidence.is_empty();
    let live_certification_status = if live_certified { "passed" } else { "blocked" };
    let mut non_claims = vec![
        "not provider quality certification".to_string(),
        "not provider SLA evidence".to_string(),
        "not provider gateway certification".to_string(),
        "not live decision quality evidence".to_string(),
        "not live self-revision quality evidence".to_string(),
        "not release approval".to_string(),
    ];
    if native_without_live_runner {
        non_claims.push("native protocol adapters are verified offline only; native live certification runner is not implemented".to_string());
    }
    let markdown = render_markdown(
        &provider,
        config_preflight_status,
        live_certified,
        live_certification_status,
        &live_evidence,
    );

    let summary = ProviderCertificationSummary {
        generated_at: Utc::now().to_rfc3339(),
        kind: "provider_certification_summary",
        provider,
        config_preflight_status,
        config_preflight_error,
        provider_config_shape: provider_config_shape(&options.config),
        live_certified,
        live_certification_status,
        missing_live_evidence,
        live_evidence,
        non_claims,
        markdown,
    };

    if let Some(path) = options.output_json_path {
        write_json(&path, &summary)?;
    }
    if let Some(path) = options.output_markdown_path {
        write_text(&path, &summary.markdown)?;
    }

    Ok(summary)
}

fn live_evidence(provider: &str, evidence_root: &Path) -> Vec<ProviderCertificationEvidence> {
    REQUIRED_LIVE_EVIDENCE
        .iter()
        .map(|(name, template, expected_kind)| {
            let evidence_path = template.replace("{provider}", provider);
            let status =
                live_evidence_status(provider, expected_kind, &evidence_root.join(&evidence_path));

            ProviderCertificationEvidence {
                name,
                status,
                evidence_path,
            }
        })
        .collect()
}

fn live_evidence_status(provider: &str, expected_kind: &str, path: &Path) -> &'static str {
    let Ok(bytes) = fs::read(path) else {
        return "missing";
    };
    if bytes.is_empty() {
        return "invalid";
    }

    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return "invalid";
    };
    let provider_matches = value.get("provider").and_then(Value::as_str) == Some(provider);
    let passed = value.get("status").and_then(Value::as_str) == Some("passed");
    let evidence_kind_matches =
        value.get("evidence_kind").and_then(Value::as_str) == Some(expected_kind);
    let live_mode = value.get("mode").and_then(Value::as_str) == Some("live");
    let generated_at_present = value
        .get("generated_at")
        .and_then(Value::as_str)
        .is_some_and(|stamp| !stamp.trim().is_empty());
    let live_not_local_only = value.get("local_only").and_then(Value::as_bool) == Some(false);
    let live_provenance_complete = live_provenance_is_complete(&value);

    if provider_matches
        && passed
        && evidence_kind_matches
        && live_mode
        && generated_at_present
        && live_not_local_only
        && live_provenance_complete
    {
        "present"
    } else {
        "invalid"
    }
}

fn live_provenance_is_complete(value: &Value) -> bool {
    value.get("endpoint_reached").and_then(Value::as_bool) == Some(true)
        && value.get("redaction_reviewed").and_then(Value::as_bool) == Some(true)
        && value.get("request_outcome").and_then(Value::as_str) == Some("passed")
        && value
            .get("command_evidence")
            .and_then(Value::as_array)
            .is_some_and(|commands| commands.iter().any(command_evidence_is_successful))
}

fn command_evidence_is_successful(value: &Value) -> bool {
    value.get("status").and_then(Value::as_str) == Some("passed")
        && value.get("exit_code").and_then(Value::as_i64) == Some(0)
        && value
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.trim() == "provider-live-certification")
        && value
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(command_is_supported_live_certification_shape)
}

fn command_is_supported_live_certification_shape(command: &str) -> bool {
    let tokens = command.split_whitespace().collect::<Vec<_>>();
    matches!(
        tokens.as_slice(),
        ["scripts/provider-live-certification-run.sh", "--live"]
            | ["./scripts/provider-live-certification-run.sh", "--live"]
    )
}

fn provider_config_shape(config: &AppConfig) -> ProviderConfigShape {
    match &config.model_config {
        ModelConfig::Mock => ProviderConfigShape {
            base_url: None,
            model: None,
            timeout_ms: None,
            credential_configured: false,
        },
        ModelConfig::OpenAiCompatible(provider) | ModelConfig::OpenRouter(provider) => {
            ProviderConfigShape {
                base_url: Some(base_url_shape(&provider.base_url)),
                model: Some("<redacted-model>".to_string()),
                timeout_ms: Some(provider.timeout_ms),
                credential_configured: !provider.api_key.trim().is_empty(),
            }
        }
        ModelConfig::OpenAiResponses(provider) | ModelConfig::Anthropic(provider) => {
            ProviderConfigShape {
                base_url: Some(base_url_shape(&provider.base_url)),
                model: Some("<redacted-model>".to_string()),
                timeout_ms: Some(provider.timeout_ms),
                credential_configured: !provider.api_key.trim().is_empty(),
            }
        }
    }
}

fn base_url_shape(base_url: &str) -> String {
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

fn render_markdown(
    provider: &str,
    config_preflight_status: &str,
    live_certified: bool,
    live_certification_status: &str,
    live_evidence: &[ProviderCertificationEvidence],
) -> String {
    let mut output = String::new();
    output.push_str("# Provider Certification Preflight\n\n");
    output.push_str(&format!("- provider: `{provider}`\n"));
    output.push_str(&format!(
        "- config_preflight_status: `{config_preflight_status}`\n"
    ));
    output.push_str(&format!("- live_certified: `{live_certified}`\n"));
    output.push_str(&format!(
        "- live_certification_status: `{live_certification_status}`\n\n"
    ));
    output.push_str("| Evidence | Status | Path |\n");
    output.push_str("| --- | --- | --- |\n");
    for entry in live_evidence {
        output.push_str(&format!(
            "| `{}` | `{}` | {} |\n",
            entry.name, entry.status, entry.evidence_path
        ));
    }
    output
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).context("serialize provider certification summary")?,
    )
    .with_context(|| format!("write {}", path.display()))
}

fn write_text(path: &Path, value: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create output directory {}", parent.display()))?;
    }
    fs::write(path, value).with_context(|| format!("write {}", path.display()))
}
