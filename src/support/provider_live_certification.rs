use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::Serialize;

use crate::{
    adapters::model::{
        metering::{
            CertificationMeter, MAX_INPUT_BYTES, MAX_OUTPUT_TOKENS, MAX_REQUESTS, RequestUsage,
        },
        native::{NativeModel, NativeProtocol},
        openai_compatible::OpenAiCompatibleModel,
    },
    domain::{
        self_revision::{SelfRevisionRequest, TriggerType},
        snapshot::SelfSnapshot,
        types::Namespace,
    },
    ports::{ModelDecisionRequest, ModelPort},
    support::config::{AppConfig, ModelConfig, OpenAiCompatibleConfig},
};

const EVIDENCE_DIR_TEMPLATE: &str = "target/reports/provider-certification";
const EVIDENCE_FILES: &[(&str, &str)] = &[
    ("live-decision.json", "live_decision"),
    ("live-self-revision.json", "live_self_revision"),
    ("provider-error-handling.json", "provider_error_handling"),
    ("redaction-review.json", "redaction_review"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLiveCertificationMode {
    StubEvidence,
    Live,
}

#[derive(Debug, Clone)]
pub struct ProviderLiveCertificationOptions {
    pub config: AppConfig,
    pub evidence_root: PathBuf,
    pub mode: ProviderLiveCertificationMode,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderLiveCertificationReport {
    pub generated_at: String,
    pub kind: &'static str,
    pub provider: String,
    pub status: &'static str,
    pub mode: &'static str,
    pub evidence_dir: String,
    pub generated_evidence: Vec<GeneratedEvidenceFile>,
    pub non_claims: Vec<String>,
    pub accounting: CertificationAccounting,
}

/// Attempts may have incurred charges even when HTTP or parsing fails.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CertificationAccounting {
    pub max_requests: usize,
    pub max_serialized_input_bytes_per_request: usize,
    pub max_output_tokens_per_request: u32,
    pub implicit_retries: usize,
    pub requests_attempted: usize,
    pub requests: Vec<RequestUsage>,
    pub price_status: &'static str,
    pub actual_cost_usd: Option<String>,
    pub software_enforced_usd_cap: bool,
    pub real_mcp_client_acceptance: bool,
}

fn accounting(meter: &CertificationMeter) -> CertificationAccounting {
    let requests = meter.snapshot();
    CertificationAccounting {
        max_requests: MAX_REQUESTS,
        max_serialized_input_bytes_per_request: MAX_INPUT_BYTES,
        max_output_tokens_per_request: MAX_OUTPUT_TOKENS,
        implicit_retries: 0,
        requests_attempted: requests.len(),
        requests,
        price_status: "unknown; reconcile provider billing, including failed requests",
        actual_cost_usd: None,
        software_enforced_usd_cap: false,
        real_mcp_client_acceptance: false,
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GeneratedEvidenceFile {
    pub evidence_kind: &'static str,
    pub status: &'static str,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ProviderLiveEvidence {
    provider: String,
    status: &'static str,
    evidence_kind: &'static str,
    generated_at: String,
    local_only: bool,
    endpoint_shape: String,
    credential_configured: bool,
    mode: &'static str,
    endpoint_reached: bool,
    redaction_reviewed: bool,
    request_outcome: &'static str,
    command_evidence: Vec<ProviderLiveCommandEvidence>,
    non_claims: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ProviderLiveCommandEvidence {
    name: &'static str,
    command: &'static str,
    status: &'static str,
    exit_code: i32,
}

pub fn run_provider_live_certification(
    options: ProviderLiveCertificationOptions,
) -> Result<ProviderLiveCertificationReport> {
    match options.mode {
        ProviderLiveCertificationMode::StubEvidence => write_stub_evidence(options),
        ProviderLiveCertificationMode::Live => write_live_evidence(options),
    }
}

fn write_stub_evidence(
    options: ProviderLiveCertificationOptions,
) -> Result<ProviderLiveCertificationReport> {
    options
        .config
        .validate()
        .map_err(|error| anyhow!("provider configuration is not valid: {error}"))?;

    let provider = options.config.model_provider.as_str().to_string();
    let generated_at = Utc::now().to_rfc3339();
    let shape = provider_shape(&options.config);
    let evidence_dir = options
        .evidence_root
        .join(EVIDENCE_DIR_TEMPLATE)
        .join(&provider);
    reserve_evidence_dir(&evidence_dir)?;

    let non_claims = stub_non_claims();
    let mut generated_evidence = Vec::with_capacity(EVIDENCE_FILES.len());
    for (file_name, evidence_kind) in EVIDENCE_FILES {
        let evidence = ProviderLiveEvidence {
            provider: provider.clone(),
            status: "passed",
            evidence_kind,
            generated_at: generated_at.clone(),
            local_only: true,
            endpoint_shape: shape.endpoint_shape.clone(),
            credential_configured: shape.credential_configured,
            mode: "stub/simulated",
            endpoint_reached: false,
            redaction_reviewed: true,
            request_outcome: "not_applicable",
            command_evidence: Vec::new(),
            non_claims: non_claims.clone(),
        };
        let path = evidence_dir.join(file_name);
        write_live_json(&path, &evidence, &options.config)?;
        generated_evidence.push(GeneratedEvidenceFile {
            evidence_kind,
            status: "passed",
            path: path.to_string_lossy().to_string(),
        });
    }

    Ok(ProviderLiveCertificationReport {
        generated_at,
        kind: "provider_live_certification_evidence_run",
        provider,
        status: "passed",
        mode: "stub/simulated",
        evidence_dir: evidence_dir.to_string_lossy().to_string(),
        generated_evidence,
        non_claims,
        accounting: accounting(&CertificationMeter::default()),
    })
}

fn write_live_evidence(
    options: ProviderLiveCertificationOptions,
) -> Result<ProviderLiveCertificationReport> {
    options
        .config
        .validate()
        .map_err(|error| anyhow!("provider configuration is not valid: {error}"))?;
    let provider = options.config.model_provider.as_str().to_string();
    let generated_at = Utc::now().to_rfc3339();
    let shape = provider_shape(&options.config);
    let meter = CertificationMeter::default();
    let model = live_model(&options.config, meter.clone())?;
    let evidence_dir = options
        .evidence_root
        .join(EVIDENCE_DIR_TEMPLATE)
        .join(&provider);
    // Reserve before any potentially billable request; never overwrite earlier results.
    reserve_evidence_dir(&evidence_dir)?;
    let loopback = options
        .config
        .doctor_base_url()
        .and_then(|url| reqwest::Url::parse(&url).ok())
        .is_some_and(|url| {
            url.host_str().is_some_and(|host| {
                host.eq_ignore_ascii_case("localhost")
                    || host
                        .trim_matches(['[', ']'])
                        .parse::<std::net::IpAddr>()
                        .is_ok_and(|ip| ip.is_loopback())
            })
        });
    let mode = if loopback {
        "offline/protocol-fixture"
    } else {
        "live"
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build provider live certification runtime")?;
    write_accounting(&evidence_dir, &meter)?;
    let mut decision_passed = false;
    let mut revision_passed = false;
    let outcome = runtime.block_on(async {
        let decision = model.decide(live_decision_request()).await;
        write_accounting(&evidence_dir, &meter)?;
        decision.map_err(|_| redacted_live_probe_error("live decision path"))?;
        decision_passed = true;
        let revision = model
            .propose_self_revision(live_self_revision_request())
            .await;
        write_accounting(&evidence_dir, &meter)?;
        revision.map_err(|_| redacted_live_probe_error("live self-revision path"))?;
        revision_passed = true;
        Ok::<(), anyhow::Error>(())
    });
    let mut non_claims = live_non_claims();
    non_claims.extend([
        "not real MCP client acceptance".into(),
        "input byte limit is not a tokenizer-exact input token limit".into(),
        "unknown prices and provider billing prevent a software-enforced dollar cap".into(),
        "successful generation does not test provider error handling or constitute human redaction review".into(),
    ]);
    if loopback {
        non_claims.retain(|claim| claim != "live provider endpoint evidence only");
        non_claims.push("offline protocol fixture; not real live provider evidence".into());
    }
    let mut generated_evidence = Vec::new();
    for (file_name, evidence_kind) in EVIDENCE_FILES {
        let passed = match *evidence_kind {
            "live_decision" => decision_passed,
            "live_self_revision" => revision_passed,
            _ => false,
        };
        let status = if passed {
            "passed"
        } else if matches!(
            *evidence_kind,
            "provider_error_handling" | "redaction_review"
        ) {
            "not_tested"
        } else {
            "failed_or_not_run"
        };
        let evidence = ProviderLiveEvidence {
            provider: provider.clone(),
            status,
            evidence_kind,
            generated_at: generated_at.clone(),
            local_only: loopback,
            endpoint_shape: shape.endpoint_shape.clone(),
            credential_configured: shape.credential_configured,
            mode,
            endpoint_reached: meter
                .snapshot()
                .iter()
                .any(|request| request.http_status.is_some()),
            redaction_reviewed: false,
            request_outcome: status,
            // This library observes probes, not the shell command's eventual exit.
            command_evidence: Vec::new(),
            non_claims: non_claims.clone(),
        };
        let path = evidence_dir.join(file_name);
        write_live_json(&path, &evidence, &options.config)?;
        generated_evidence.push(GeneratedEvidenceFile {
            evidence_kind,
            status,
            path: path.to_string_lossy().into_owned(),
        });
    }
    let report = ProviderLiveCertificationReport {
        generated_at,
        kind: "provider_live_certification_evidence_run",
        provider,
        status: if outcome.is_ok() { "passed" } else { "failed" },
        mode,
        evidence_dir: evidence_dir.to_string_lossy().into_owned(),
        generated_evidence,
        non_claims,
        accounting: accounting(&meter),
    };
    write_live_json(
        &evidence_dir.join("run-accounting.json"),
        &report,
        &options.config,
    )?;
    outcome?;
    Ok(report)
}

// This schema contains only fixed labels, numbers and booleans. It deliberately
// excludes config and paths, so the fallback ledger survives report redaction failures.
fn write_accounting(directory: &Path, meter: &CertificationMeter) -> Result<()> {
    fs::write(
        directory.join("request-accounting.json"),
        serde_json::to_vec_pretty(&accounting(meter))?,
    )
    .context("could not persist request accounting; stop and reconcile provider billing")
}

fn reserve_evidence_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir(path)
        .context("evidence directory must be new; previous evidence is never overwritten")
}

fn redacted_live_probe_error(label: &'static str) -> anyhow::Error {
    anyhow!("{label} failed; raw provider request and response details were not serialized")
}

fn live_model(config: &AppConfig, meter: CertificationMeter) -> Result<Box<dyn ModelPort>> {
    let model: Box<dyn ModelPort> = match &config.model_config {
        ModelConfig::Mock => return Err(anyhow!("mock cannot produce live evidence")),
        ModelConfig::OpenAiResponses(provider) => Box::new(
            NativeModel::new(provider.clone(), NativeProtocol::OpenAiResponses)?
                .with_certification_meter(meter),
        ),
        ModelConfig::Anthropic(provider) => Box::new(
            NativeModel::new(provider.clone(), NativeProtocol::Anthropic)?
                .with_certification_meter(meter),
        ),
        ModelConfig::OpenAiCompatible(provider) => {
            Box::new(OpenAiCompatibleModel::new(provider.clone())?.with_certification_meter(meter))
        }
        ModelConfig::OpenRouter(provider) => Box::new(
            OpenAiCompatibleModel::new_for_provider(provider.clone(), "openrouter")?
                .with_certification_meter(meter),
        ),
    };
    Ok(model)
}

fn live_decision_request() -> ModelDecisionRequest {
    ModelDecisionRequest::new(
        "Provider live certification decision probe. Return a short non-empty action token."
            .to_string(),
        "provider_live_certification_decision_probe".to_string(),
        certification_snapshot(),
    )
}

fn live_self_revision_request() -> SelfRevisionRequest {
    SelfRevisionRequest::new(
        TriggerType::Periodic,
        Namespace::self_(),
        certification_snapshot(),
        vec!["provider-live-certification-evidence".to_string()],
        vec!["provider live certification".to_string()],
    )
}

fn certification_snapshot() -> SelfSnapshot {
    SelfSnapshot {
        identity: vec!["identity:self=provider-certification-runner".to_string()],
        commitments: vec!["forbid:serialize_provider_payloads".to_string()],
        claims: vec!["provider.certification.mode=live".to_string()],
        evidence: vec!["event:provider-live-certification-evidence".to_string()],
        episodes: vec!["episode:provider-live-certification".to_string()],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderShape {
    endpoint_shape: String,
    credential_configured: bool,
}

fn provider_shape(config: &AppConfig) -> ProviderShape {
    match &config.model_config {
        ModelConfig::Mock => ProviderShape {
            endpoint_shape: "<none>".to_string(),
            credential_configured: false,
        },
        ModelConfig::OpenAiCompatible(provider) | ModelConfig::OpenRouter(provider) => {
            provider_shape_for_openai_compatible(provider)
        }
        ModelConfig::OpenAiResponses(provider) | ModelConfig::Anthropic(provider) => {
            ProviderShape {
                endpoint_shape: endpoint_shape(&provider.base_url),
                credential_configured: !provider.api_key.trim().is_empty(),
            }
        }
    }
}

fn provider_shape_for_openai_compatible(provider: &OpenAiCompatibleConfig) -> ProviderShape {
    ProviderShape {
        endpoint_shape: endpoint_shape(&provider.base_url),
        credential_configured: !provider.api_key.trim().is_empty(),
    }
}

fn endpoint_shape(base_url: &str) -> String {
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

fn stub_non_claims() -> Vec<String> {
    vec![
        "stub/simulated evidence only".to_string(),
        "not real live provider evidence".to_string(),
        "not provider endpoint reachability evidence".to_string(),
        "not provider decision quality evidence".to_string(),
        "not provider self-revision quality evidence".to_string(),
        "does not include request bodies, response bodies, API keys, URL userinfo, URL paths, query values, or provider-native payloads".to_string(),
    ]
}

fn live_non_claims() -> Vec<String> {
    vec![
        "live provider endpoint evidence only".to_string(),
        "not provider quality certification".to_string(),
        "not release approval".to_string(),
        "not Beta, GA, hosted, remote/team, or production-ready evidence".to_string(),
        "does not include request bodies, response bodies, API keys, URL userinfo, URL paths, query values, model ids, or provider-native payloads".to_string(),
    ]
}

fn write_live_json(path: &Path, value: &impl Serialize, config: &AppConfig) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .context("serialize provider live certification evidence")?;
    let serialized =
        std::str::from_utf8(&bytes).context("provider live certification evidence is utf-8")?;
    for fragment in forbidden_live_evidence_fragments(config) {
        if serialized.contains(&fragment) {
            return Err(anyhow!(
                "live provider evidence redaction check failed before writing {}",
                path.display()
            ));
        }
    }
    fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

fn forbidden_live_evidence_fragments(config: &AppConfig) -> Vec<String> {
    let mut fragments = vec![
        "Provider live certification decision probe".to_string(),
        "provider_live_certification_decision_probe".to_string(),
        "provider live certification".to_string(),
        "forbid:serialize_provider_payloads".to_string(),
        "provider-live-certification-evidence".to_string(),
    ];

    let fields = match &config.model_config {
        ModelConfig::Mock => None,
        ModelConfig::OpenAiCompatible(provider) | ModelConfig::OpenRouter(provider) => {
            Some((&provider.api_key, &provider.model, &provider.base_url))
        }
        ModelConfig::OpenAiResponses(provider) | ModelConfig::Anthropic(provider) => {
            Some((&provider.api_key, &provider.model, &provider.base_url))
        }
    };
    if let Some((api_key, model, base_url)) = fields {
        push_non_empty(&mut fragments, api_key.trim());
        push_non_empty(&mut fragments, model.trim());

        if let Ok(parsed) = reqwest::Url::parse(base_url) {
            push_non_empty(&mut fragments, parsed.username());
            if let Some(password) = parsed.password() {
                push_non_empty(&mut fragments, password);
            }
            if parsed.path() != "/" {
                push_non_empty(&mut fragments, parsed.path());
                for segment in parsed.path_segments().into_iter().flatten() {
                    if segment.len() > 2 {
                        push_non_empty(&mut fragments, segment);
                    }
                }
            }
            if let Some(query) = parsed.query() {
                push_non_empty(&mut fragments, query);
                for part in query.split('&') {
                    push_non_empty(&mut fragments, part);
                    for value in part.split('=') {
                        if value.len() > 2 {
                            push_non_empty(&mut fragments, value);
                        }
                    }
                }
            }
        }
    }

    fragments
}

fn push_non_empty(fragments: &mut Vec<String>, value: &str) {
    if !value.is_empty() {
        fragments.push(value.to_string());
    }
}
