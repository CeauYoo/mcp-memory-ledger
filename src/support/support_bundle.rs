use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::Serialize;
use sha2::Digest;
use sqlx::{
    Row,
    sqlite::{SqliteConnectOptions, SqlitePool},
};
use uuid::{Uuid, Version};

use crate::{
    interfaces,
    support::config::{AppConfig, ModelConfig},
};

const OPERATION_SUMMARY_LIMIT: usize = 25;
const LOG_MAX_EXCERPTS: usize = 12;
const LOG_MAX_EXCERPT_CHARS: usize = 240;
const LOG_MAX_TOTAL_BYTES: usize = 4096;
const LOG_MAX_INPUT_BYTES: u64 = 65_536;
const LOG_TAIL_LINES: usize = 200;
const LOG_REDACTION_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct SupportBundleOptions {
    pub config: AppConfig,
    pub output_dir: PathBuf,
    pub config_path: Option<PathBuf>,
    pub project_root: PathBuf,
    pub local_log_path: Option<PathBuf>,
    pub operation_correlation_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct SafeDoctorReport {
    transport: String,
    database_url_shape: String,
    provider: String,
    base_url: Option<String>,
    model: Option<String>,
    dashboard_enabled: bool,
    dashboard_host: String,
    dashboard_port: u16,
    dashboard_base_path: String,
    dashboard_required: bool,
    daemon_enabled: bool,
    daemon_poll_interval_ms: u64,
    daemon_max_concurrent_tasks: u32,
    auto_reflection_runtime_hooks: Vec<String>,
    self_revision_write_path: &'static str,
    runtime_bootstrap_performed: bool,
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct ConfigShape {
    transport: String,
    database_url_shape: String,
    model: ModelShape,
    dashboard: DashboardShape,
    daemon: DaemonShape,
    config_path: Option<String>,
}

#[derive(Debug, Serialize)]
struct ModelShape {
    provider: String,
    base_url: Option<String>,
    model: Option<String>,
    timeout_ms: Option<u64>,
    credential_configured: bool,
}

#[derive(Debug, Serialize)]
struct DashboardShape {
    enabled: bool,
    host: String,
    port: u16,
    base_path: String,
    event_capacity: usize,
    sse_enabled: bool,
    open_browser: bool,
    required: bool,
}

#[derive(Debug, Serialize)]
struct DaemonShape {
    enabled: bool,
    poll_interval_ms: u64,
    max_concurrent_tasks: u32,
}

#[derive(Debug, Serialize)]
struct OperationSummaries {
    limit: usize,
    available: bool,
    filter: Option<OperationSummaryFilter>,
    unavailable_reason: Option<&'static str>,
    entries: Vec<OperationSummary>,
}

#[derive(Debug, Serialize)]
struct OperationSummaryFilter {
    correlation_id: String,
}

#[derive(Debug, Serialize)]
struct OperationSummary {
    operation_id: String,
    occurred_at: String,
    namespace: Option<String>,
    entrypoint: String,
    operation_kind: String,
    status: String,
    correlation_id: Option<String>,
    read_only: bool,
}

#[derive(Debug, Serialize)]
struct ReleaseMetadata {
    generated_at: String,
    git_commit: Option<String>,
    git_branch: Option<String>,
    platform: String,
    rustc_version: Option<String>,
    verification_commands: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
struct ProductSmokeSummary {
    required_artifacts: Vec<&'static str>,
    latest_artifacts_present: bool,
    self_revision_write_path_expected: &'static str,
}

#[derive(Debug, Serialize)]
struct Manifest {
    bundle_format: &'static str,
    generated_at: String,
    local_only: bool,
    artifact_scope: &'static str,
    upload_performed: bool,
    production_support_channel: bool,
    remote_support_surface: bool,
    safety_checks: ManifestSafetyChecks,
    excluded_by_default: Vec<&'static str>,
    files: Vec<&'static str>,
    integrity: ManifestIntegrity,
    bounds: ManifestBounds,
}

#[derive(Debug, Serialize)]
struct ManifestSafetyChecks {
    read_only: bool,
    runtime_bootstrap_performed: bool,
    sqlite_files_included: bool,
    toml_files_included: bool,
    raw_log_files_included: bool,
    provider_payloads_included: bool,
}

#[derive(Debug, Serialize)]
struct ManifestBounds {
    local_log_excerpts: LocalLogBounds,
}

#[derive(Debug, Serialize)]
struct ManifestIntegrity {
    algorithm: &'static str,
    covers: &'static str,
    files: Vec<ManifestIntegrityFile>,
}

#[derive(Debug, Serialize)]
struct ManifestIntegrityFile {
    path: &'static str,
    sha256: String,
    size_bytes: u64,
}

#[derive(Debug, Serialize, Clone, Copy)]
struct LocalLogBounds {
    max_excerpts: usize,
    max_excerpt_chars: usize,
    max_total_bytes: usize,
    max_input_bytes: u64,
    tail_lines: usize,
}

#[derive(Debug, Serialize)]
struct LocalLogExcerpts {
    available: bool,
    unavailable_reason: Option<&'static str>,
    read_only: bool,
    source: Option<String>,
    redaction_version: u32,
    line_count: usize,
    line_number_scope: &'static str,
    input_truncated: bool,
    bounds: LocalLogBounds,
    exclusions: Vec<&'static str>,
    excerpts: Vec<LocalLogExcerpt>,
}

#[derive(Debug, Serialize)]
struct LocalLogExcerpt {
    line: usize,
    text: String,
    truncated: bool,
}

pub async fn generate_support_bundle(options: SupportBundleOptions) -> Result<()> {
    let SupportBundleOptions {
        config,
        output_dir,
        config_path,
        project_root,
        local_log_path,
        operation_correlation_id,
    } = options;

    validate_operation_correlation_id(operation_correlation_id.as_deref())?;

    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "failed to create support bundle directory {}",
            output_dir.display()
        )
    })?;
    ensure_empty_output_dir(&output_dir)?;

    config.validate().map_err(anyhow::Error::msg)?;

    let operations =
        query_operation_summaries(&config.database_url, operation_correlation_id.as_deref()).await;
    let generated_at = Utc::now().to_rfc3339();

    write_json(
        &output_dir.join("doctor.json"),
        &safe_doctor_report(&config),
    )?;
    write_json(
        &output_dir.join("config-shape.json"),
        &config_shape(&config, config_path.as_deref()),
    )?;
    write_json(&output_dir.join("operation-summaries.json"), &operations)?;
    write_json(
        &output_dir.join("release-metadata.json"),
        &release_metadata(&project_root, &generated_at),
    )?;
    write_json(
        &output_dir.join("product-smoke-summary.json"),
        &product_smoke_summary(&project_root),
    )?;
    write_json(
        &output_dir.join("local-log-excerpts.json"),
        &local_log_excerpts(local_log_path.as_deref()),
    )?;
    write_json(
        &output_dir.join("manifest.json"),
        &manifest(&generated_at, &output_dir)?,
    )?;

    Ok(())
}

fn local_log_excerpts(log_path: Option<&Path>) -> LocalLogExcerpts {
    let Some(log_path) = log_path else {
        return unavailable_local_log_excerpts("log file not requested", None);
    };

    let source = Some(redact_path(log_path));
    let Ok(metadata) = fs::metadata(log_path) else {
        return unavailable_local_log_excerpts("log file not found", source);
    };
    if !metadata.is_file() {
        return unavailable_local_log_excerpts("log file not found", source);
    }

    let Ok((bytes, was_input_truncated)) = read_bounded_log_bytes(log_path, metadata.len()) else {
        return unavailable_local_log_excerpts("log file unreadable", source);
    };

    let text = String::from_utf8_lossy(&bytes);
    let line_count = text.lines().count();
    let start_line = line_count.saturating_sub(LOG_TAIL_LINES);
    let mut total_bytes = 0usize;
    let mut excerpts = Vec::new();
    let mut skipping_private_key_block = retained_tail_starts_inside_private_key_block(&text);
    for (offset, line) in text.lines().skip(start_line).enumerate() {
        if excerpts.len() >= LOG_MAX_EXCERPTS || total_bytes >= LOG_MAX_TOTAL_BYTES {
            break;
        }
        if should_skip_private_key_block_line(line, &mut skipping_private_key_block) {
            continue;
        }
        if should_skip_log_line(line) {
            continue;
        }

        let (mut redacted, mut truncated) = redact_log_line(line);
        if redacted.trim().is_empty() || should_skip_log_line(&redacted) {
            continue;
        }
        if redacted.chars().count() > LOG_MAX_EXCERPT_CHARS {
            redacted = redacted
                .chars()
                .take(LOG_MAX_EXCERPT_CHARS)
                .collect::<String>();
            truncated = true;
        }
        let redacted_bytes = redacted.len();
        if total_bytes + redacted_bytes > LOG_MAX_TOTAL_BYTES {
            break;
        }

        total_bytes += redacted_bytes;
        excerpts.push(LocalLogExcerpt {
            line: start_line + offset + 1,
            text: redacted,
            truncated,
        });
    }

    LocalLogExcerpts {
        available: true,
        unavailable_reason: None,
        read_only: true,
        source,
        redaction_version: LOG_REDACTION_VERSION,
        line_count,
        line_number_scope: if was_input_truncated { "tail" } else { "file" },
        input_truncated: was_input_truncated,
        bounds: local_log_bounds(),
        exclusions: local_log_exclusions(),
        excerpts,
    }
}

fn read_bounded_log_bytes(log_path: &Path, file_len: u64) -> Result<(Vec<u8>, bool)> {
    let mut file = fs::File::open(log_path)?;
    let input_truncated = file_len > LOG_MAX_INPUT_BYTES;
    if input_truncated {
        file.seek(SeekFrom::Start(file_len - LOG_MAX_INPUT_BYTES))?;
    }

    let mut bytes = Vec::new();
    file.take(LOG_MAX_INPUT_BYTES).read_to_end(&mut bytes)?;
    if input_truncated {
        if let Some(first_newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes = bytes[(first_newline + 1)..].to_vec();
        } else {
            bytes.clear();
        }
    }
    Ok((bytes, input_truncated))
}

fn unavailable_local_log_excerpts(
    reason: &'static str,
    source: Option<String>,
) -> LocalLogExcerpts {
    LocalLogExcerpts {
        available: false,
        unavailable_reason: Some(reason),
        read_only: true,
        source,
        redaction_version: LOG_REDACTION_VERSION,
        line_count: 0,
        line_number_scope: "unavailable",
        input_truncated: false,
        bounds: local_log_bounds(),
        exclusions: local_log_exclusions(),
        excerpts: Vec::new(),
    }
}

fn local_log_bounds() -> LocalLogBounds {
    LocalLogBounds {
        max_excerpts: LOG_MAX_EXCERPTS,
        max_excerpt_chars: LOG_MAX_EXCERPT_CHARS,
        max_total_bytes: LOG_MAX_TOTAL_BYTES,
        max_input_bytes: LOG_MAX_INPUT_BYTES,
        tail_lines: LOG_TAIL_LINES,
    }
}

fn local_log_exclusions() -> Vec<&'static str> {
    vec![
        "raw local log files",
        "raw provider payloads",
        "authorization headers",
        "bearer values",
        "provider credentials",
        "access credentials",
        "web credential data",
        "ssh keys",
        "full local paths",
    ]
}

fn should_skip_log_line(line: &str) -> bool {
    if is_sensitive_json_payload(line) {
        return true;
    }

    let lower = line.to_lowercase();
    if text_has_sensitive_json_key(&lower) {
        return true;
    }

    if line_has_raw_provider_or_diagnostic_payload_field(&lower) {
        return true;
    }

    if [
        "messages", "content", "input", "prompt", "request", "response",
    ]
    .iter()
    .any(|field| line_has_payload_field(&lower, field))
    {
        return true;
    }

    if line_has_payload_field(&lower, "tool") {
        return true;
    }

    [
        "tool_args",
        "tool args",
        "cookie",
        "session_id",
        "session-id",
        "session token",
        "session_token",
        "session-token",
        "browser session",
        "localstorage",
        "sessionstorage",
        "browser profile",
        "profile path",
        "profile_path",
        ".ssh/",
        "ssh_key",
        "id_ed25519",
        "private key",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || is_key_material_like(line)
}

fn line_has_raw_provider_or_diagnostic_payload_field(line: &str) -> bool {
    [
        "provider_payload",
        "provider_request",
        "provider_request_body",
        "provider_response",
        "provider_response_body",
        "provider_diagnostic",
        "provider_diagnostics",
        "provider_diagnostic_payload",
        "provider_diagnostics_payload",
        "raw_diagnostic",
        "raw_diagnostics",
        "diagnostic",
        "diagnostics",
        "diagnostic_payload",
        "diagnostics_payload",
        "diagnostic_summary",
        "diagnostic_summary_json",
        "raw_provider_payload",
        "raw_provider_diagnostic",
        "raw_provider_diagnostics",
        "request_body",
        "response_body",
    ]
    .iter()
    .any(|field| line_has_payload_field(line, field))
}

fn should_skip_private_key_block_line(line: &str, skipping_private_key_block: &mut bool) -> bool {
    let lower = line.to_lowercase();
    if lower.contains("-----begin ") && lower.contains(" private key-----") {
        *skipping_private_key_block = true;
        return true;
    }
    if lower.contains("-----end ") && lower.contains(" private key-----") {
        *skipping_private_key_block = false;
        return true;
    }

    *skipping_private_key_block
}

fn retained_tail_starts_inside_private_key_block(text: &str) -> bool {
    let lower = text.to_lowercase();
    if !lower.contains(" private key-----") {
        return false;
    }

    match (lower.find("-----begin "), lower.find("-----end ")) {
        (None, Some(_)) => true,
        (Some(begin), Some(end)) => end < begin,
        _ => false,
    }
}

fn line_has_payload_field(line: &str, field: &str) -> bool {
    line.contains(&format!("\"{field}\""))
        || line.split_whitespace().enumerate().any(|(index, token)| {
            let trimmed = token.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}'
                )
            });
            trimmed == format!("{field}=")
                || trimmed == format!("{field}:")
                || trimmed.starts_with(&format!("{field}="))
                || trimmed.starts_with(&format!("{field}:"))
                || (trimmed == field
                    && line
                        .split_whitespace()
                        .nth(index + 1)
                        .is_some_and(|next| matches!(next, "=" | ":")))
        })
}

fn is_sensitive_json_payload(line: &str) -> bool {
    let trimmed = line.trim();
    if !(trimmed.starts_with('{') || trimmed.starts_with('[')) {
        return false;
    }

    serde_json::from_str::<serde_json::Value>(trimmed)
        .map(|value| json_value_has_sensitive_key(&value))
        .unwrap_or_else(|_| text_has_sensitive_json_key(trimmed))
}

fn json_value_has_sensitive_key(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => object
            .iter()
            .any(|(key, value)| is_sensitive_json_key(key) || json_value_has_sensitive_key(value)),
        serde_json::Value::Array(values) => values.iter().any(json_value_has_sensitive_key),
        _ => false,
    }
}

fn text_has_sensitive_json_key(value: &str) -> bool {
    let lower = value.to_lowercase();
    if [
        "\"api_key\"",
        "\"api-key\"",
        "\"apikey\"",
        "\"x-api-key\"",
        "\"openai_api_key\"",
        "\"token\"",
        "\"password\"",
        "\"secret\"",
        "\"authorization\"",
        "\"bearer\"",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
    {
        return true;
    }

    let bytes = lower.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let Some(start) = lower[cursor..].find('"') else {
            break;
        };
        let key_start = cursor + start + 1;
        let Some(end) = lower[key_start..].find('"') else {
            break;
        };
        let key_end = key_start + end;
        let rest = &lower[(key_end + 1)..];
        if rest.trim_start().starts_with(':') && is_sensitive_json_key(&lower[key_start..key_end]) {
            return true;
        }
        cursor = key_end + 1;
    }

    false
}

fn is_sensitive_json_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    lower == "authorization"
        || lower == "bearer"
        || lower.contains("api_key")
        || lower.contains("api-key")
        || lower.contains("apikey")
        || lower.contains("token")
        || lower.contains("password")
        || lower.contains("secret")
}

fn redact_log_line(line: &str) -> (String, bool) {
    let original = line.to_string();
    let mut redacted = line.to_string();
    redacted = redact_urls(&redacted);
    redacted = redact_local_paths(&redacted);
    redacted = redact_sensitive_markers(&redacted);
    redacted = redact_sk_values(&redacted);
    let changed = redacted != original;
    (redacted, changed)
}

fn redact_urls(line: &str) -> String {
    line.split_whitespace()
        .map(redact_url_token)
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_url_token(token: &str) -> String {
    let Some(scheme_marker) = token.find("://") else {
        return token.to_string();
    };
    let scheme_start = token[..scheme_marker]
        .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.')))
        .map_or(0, |index| index + 1);
    let prefix = &token[..scheme_start];
    let candidate = &token[scheme_start..];
    let trimmed = candidate.trim_end_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}'
        )
    });
    let suffix = &candidate[trimmed.len()..];

    reqwest::Url::parse(trimmed)
        .map(|url| format!("{prefix}{}{suffix}", base_url_shape(url.as_str())))
        .unwrap_or_else(|_| token.to_string())
}

fn redact_local_paths(line: &str) -> String {
    let redacted_windows_paths = redact_windows_paths(line);
    let redacted_posix_paths = redact_posix_paths(&redacted_windows_paths);
    redacted_posix_paths
        .split_whitespace()
        .map(|part| {
            let trimmed = part.trim_matches(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}'
                )
            });
            if is_local_path_like(trimmed) {
                let prefix_len = part.find(trimmed).unwrap_or(0);
                let suffix_start = prefix_len + trimmed.len();
                format!(
                    "{}{}{}",
                    &part[..prefix_len],
                    redact_log_path_like(trimmed),
                    &part[suffix_start..]
                )
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_windows_paths(line: &str) -> String {
    let mut redacted = String::new();
    let mut cursor = 0usize;
    while let Some((start, end, file_name)) = find_windows_path(&line[cursor..]) {
        let absolute_start = cursor + start;
        let absolute_end = cursor + end;
        redacted.push_str(&line[cursor..absolute_start]);
        redacted.push_str(&format!("<local-path>/{file_name}"));
        cursor = absolute_end;
    }
    redacted.push_str(&line[cursor..]);
    redacted
}

fn find_windows_path(value: &str) -> Option<(usize, usize, String)> {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index + 2 < bytes.len() {
        if bytes[index].is_ascii_alphabetic()
            && bytes[index + 1] == b':'
            && bytes[index + 2] == b'\\'
        {
            let start = index;
            let mut end = index + 3;
            while end < bytes.len() {
                let byte = bytes[end];
                if matches!(
                    byte,
                    b'"' | b'\'' | b',' | b';' | b')' | b'(' | b'[' | b']' | b'{' | b'}'
                ) || (byte.is_ascii_whitespace()
                    && next_non_whitespace_starts_field(&bytes[end..]))
                {
                    break;
                }
                end += 1;
            }
            let path = &value[start..end];
            let file_name = path
                .rsplit('\\')
                .next()
                .map(str::trim)
                .filter(|file_name| !file_name.is_empty())
                .unwrap_or("path")
                .to_string();
            return Some((start, end, file_name));
        }
        index += 1;
    }
    None
}

fn redact_posix_paths(line: &str) -> String {
    let mut redacted = String::new();
    let mut cursor = 0usize;
    while let Some((start, end, file_name)) = find_posix_path(&line[cursor..]) {
        let absolute_start = cursor + start;
        let absolute_end = cursor + end;
        redacted.push_str(&line[cursor..absolute_start]);
        redacted.push_str(&format!("<local-path>/{file_name}"));
        cursor = absolute_end;
    }
    redacted.push_str(&line[cursor..]);
    redacted
}

fn find_posix_path(value: &str) -> Option<(usize, usize, String)> {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if value.is_char_boundary(index) && starts_posix_path_like(&value[index..]) {
            let start = index;
            let mut end = index;
            while end < bytes.len() {
                let byte = bytes[end];
                if matches!(
                    byte,
                    b'"' | b'\'' | b',' | b';' | b')' | b'(' | b'[' | b']' | b'{' | b'}'
                ) || (byte.is_ascii_whitespace()
                    && next_non_whitespace_starts_field(&bytes[end..]))
                {
                    break;
                }
                end += 1;
            }
            let path = &value[start..end];
            let file_name = path
                .rsplit('/')
                .next()
                .map(str::trim)
                .filter(|file_name| !file_name.is_empty())
                .unwrap_or("path")
                .to_string();
            return Some((start, end, file_name));
        }
        index += 1;
    }
    None
}

fn starts_posix_path_like(value: &str) -> bool {
    value.starts_with("/Users/")
        || value.starts_with("/home/")
        || value.starts_with("/tmp/")
        || value.starts_with("/var/")
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
}

fn next_non_whitespace_starts_field(bytes: &[u8]) -> bool {
    let Some(next_index) = bytes.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return false;
    };
    let rest = &bytes[next_index..];
    let field_len = rest
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    field_len > 0 && rest.get(field_len) == Some(&b'=')
}

fn redact_log_path_like(value: &str) -> String {
    if value.contains('\\') {
        let file_name = value
            .rsplit('\\')
            .next()
            .filter(|file_name| !file_name.is_empty())
            .unwrap_or("path");
        return format!("<local-path>/{file_name}");
    }

    redact_path(Path::new(value))
}

fn is_local_path_like(value: &str) -> bool {
    is_windows_path_like(value)
        || value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
        || value.contains("/Users/")
        || value.contains("/home/")
        || value.contains("/tmp/")
        || value.contains("/var/")
        || value.contains(".config/")
        || value.ends_with(".toml")
        || value.ends_with(".sqlite")
        || value.ends_with(".log")
}

fn is_windows_path_like(value: &str) -> bool {
    let lower = value.to_lowercase();
    value.as_bytes().get(1) == Some(&b':')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && value.contains('\\')
        || lower.contains("\\users\\")
        || lower.contains("\\appdata\\")
        || lower.contains("\\temp\\")
}

fn redact_sensitive_markers(line: &str) -> String {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    let mut output = Vec::new();
    let mut index = 0usize;
    while index < tokens.len() {
        let token = tokens[index];
        let lower = token.to_lowercase();

        if let Some((marker, separator, value, marker_tokens)) =
            compound_sensitive_marker_parts(&tokens, index)
        {
            let is_auth =
                marker == "authorization" || marker == "bearer" || value.contains("bearer");
            output.push(if is_auth {
                "<redacted-auth>".to_string()
            } else {
                "<redacted-value>".to_string()
            });
            index += marker_tokens
                + secret_value_tokens_to_skip(
                    &tokens[(index + marker_tokens)..],
                    separator,
                    &value,
                );
        } else if let Some((marker, separator, value)) = sensitive_marker_parts(&lower) {
            let is_auth =
                marker == "authorization" || marker == "bearer" || value.contains("bearer");
            output.push(if is_auth {
                "<redacted-auth>".to_string()
            } else {
                "<redacted-value>".to_string()
            });
            index += 1 + secret_value_tokens_to_skip(&tokens[(index + 1)..], separator, value);
        } else {
            output.push(token.to_string());
            index += 1;
        }
    }
    output.join(" ")
}

fn compound_sensitive_marker_parts(
    tokens: &[&str],
    index: usize,
) -> Option<(String, Option<char>, String, usize)> {
    let first = tokens.get(index)?.to_lowercase();
    let second = tokens.get(index + 1)?.to_lowercase();
    let (first_marker, first_separator, first_value) = marker_token_parts(&first);
    if first_separator.is_some() || !first_value.is_empty() {
        return None;
    }

    let (second_marker, separator, value) = marker_token_parts(&second);
    let marker = format!("{first_marker}_{second_marker}");
    if !is_sensitive_marker(&marker) {
        return None;
    }

    let has_separator = separator.is_some()
        || tokens
            .get(index + 2)
            .is_some_and(|next| matches!(*next, "=" | ":"));
    if !has_separator {
        return None;
    }

    Some((marker, separator, value.to_string(), 2))
}

fn sensitive_marker_parts(token: &str) -> Option<(&str, Option<char>, &str)> {
    let (marker, separator, value) = marker_token_parts(token);
    if is_sensitive_marker(marker) {
        Some((marker, separator, value))
    } else {
        None
    }
}

fn marker_token_parts(token: &str) -> (&str, Option<char>, &str) {
    let trimmed = token.trim_matches(|ch: char| {
        !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '=' | ':'))
    });
    let (marker, separator, value) = if let Some((marker, value)) = trimmed.split_once('=') {
        (marker, Some('='), value)
    } else if let Some((marker, value)) = trimmed.split_once(':') {
        (marker, Some(':'), value)
    } else {
        (trimmed, None, "")
    };
    let marker =
        marker.trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')));
    (marker, separator, value)
}

fn secret_value_tokens_to_skip(tokens: &[&str], separator: Option<char>, value: &str) -> usize {
    if separator.is_some()
        && !value
            .trim_matches(|ch: char| matches!(ch, '"' | '\''))
            .is_empty()
    {
        if value.contains("bearer") && !tokens.is_empty() {
            return 1;
        }
        return 0;
    }

    match tokens {
        [next, value, ..] if matches!(*next, "=" | ":") => {
            if value.eq_ignore_ascii_case("bearer") && tokens.len() >= 3 {
                3
            } else {
                2
            }
        }
        [next, ..] if next.eq_ignore_ascii_case("bearer") && tokens.len() >= 2 => 2,
        [_next, ..] => 1,
        [] => 0,
    }
}

fn is_sensitive_marker(marker: &str) -> bool {
    [
        "api_key",
        "api-key",
        "apikey",
        "x-api-key",
        "openai_api_key",
        "token",
        "password",
        "secret",
        "authorization",
        "bearer",
        "access_key",
        "secret_key",
    ]
    .iter()
    .any(|sensitive| {
        marker == *sensitive
            || marker.ends_with(&format!("_{sensitive}"))
            || marker.ends_with(&format!("-{sensitive}"))
    })
}

fn is_key_material_like(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 6
        && !trimmed.chars().any(char::is_whitespace)
        && trimmed.chars().any(|ch| ch.is_ascii_alphabetic())
        && trimmed.chars().any(|ch| ch.is_ascii_digit())
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=' | '-' | '_'))
}

fn redact_sk_values(line: &str) -> String {
    line.split_whitespace()
        .map(|token| {
            if token.to_lowercase().contains("sk-") {
                "<redacted-value>".to_string()
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ensure_empty_output_dir(output_dir: &Path) -> Result<()> {
    let mut entries = fs::read_dir(output_dir).with_context(|| {
        format!(
            "failed to inspect support bundle directory {}",
            output_dir.display()
        )
    })?;
    if entries.next().transpose()?.is_some() {
        anyhow::bail!(
            "support bundle output directory must be empty: {}",
            output_dir.display()
        );
    }

    Ok(())
}

fn safe_doctor_report(config: &AppConfig) -> SafeDoctorReport {
    let model = model_shape(config);

    SafeDoctorReport {
        transport: serde_name(&config.transport),
        database_url_shape: database_url_shape(&config.database_url),
        provider: serde_name(&config.model_provider),
        base_url: model.base_url,
        model: model.model,
        dashboard_enabled: config.dashboard.enabled,
        dashboard_host: config.dashboard.host.clone(),
        dashboard_port: config.dashboard.port,
        dashboard_base_path: config.dashboard.base_path.clone(),
        dashboard_required: config.dashboard.required,
        daemon_enabled: config.daemon.enabled,
        daemon_poll_interval_ms: config.daemon.poll_interval_ms,
        daemon_max_concurrent_tasks: config.daemon.max_concurrent_tasks,
        auto_reflection_runtime_hooks: interfaces::mcp::server::AUTO_REFLECTION_RUNTIME_HOOKS
            .iter()
            .map(|hook| hook.to_string())
            .collect(),
        self_revision_write_path: interfaces::mcp::server::SELF_REVISION_WRITE_PATH,
        runtime_bootstrap_performed: false,
        status: "config-shape-ok",
    }
}

fn config_shape(config: &AppConfig, config_path: Option<&Path>) -> ConfigShape {
    let model = model_shape(config);

    ConfigShape {
        transport: serde_name(&config.transport),
        database_url_shape: database_url_shape(&config.database_url),
        model,
        dashboard: DashboardShape {
            enabled: config.dashboard.enabled,
            host: config.dashboard.host.clone(),
            port: config.dashboard.port,
            base_path: config.dashboard.base_path.clone(),
            event_capacity: config.dashboard.event_capacity,
            sse_enabled: config.dashboard.sse_enabled,
            open_browser: config.dashboard.open_browser,
            required: config.dashboard.required,
        },
        daemon: DaemonShape {
            enabled: config.daemon.enabled,
            poll_interval_ms: config.daemon.poll_interval_ms,
            max_concurrent_tasks: config.daemon.max_concurrent_tasks,
        },
        config_path: config_path.map(redact_path),
    }
}

fn model_shape(config: &AppConfig) -> ModelShape {
    match &config.model_config {
        ModelConfig::Mock => ModelShape {
            provider: serde_name(&config.model_provider),
            base_url: None,
            model: None,
            timeout_ms: None,
            credential_configured: false,
        },
        ModelConfig::OpenAiCompatible(provider) | ModelConfig::OpenRouter(provider) => ModelShape {
            provider: serde_name(&config.model_provider),
            base_url: Some(base_url_shape(&provider.base_url)),
            model: Some(provider.model.clone()),
            timeout_ms: Some(provider.timeout_ms),
            credential_configured: !provider.api_key.trim().is_empty(),
        },
        ModelConfig::OpenAiResponses(provider) | ModelConfig::Anthropic(provider) => ModelShape {
            provider: serde_name(&config.model_provider),
            base_url: Some(base_url_shape(&provider.base_url)),
            model: Some(provider.model.clone()),
            timeout_ms: Some(provider.timeout_ms),
            credential_configured: !provider.api_key.trim().is_empty(),
        },
    }
}

async fn query_operation_summaries(
    database_url: &str,
    correlation_id: Option<&str>,
) -> OperationSummaries {
    if let Some(path) = sqlite_database_file_path(database_url)
        && !path.is_file()
    {
        return unavailable_operation_summaries("sqlite database file not found", correlation_id);
    }

    match query_operation_summaries_read_only(database_url, correlation_id).await {
        Ok(entries) => OperationSummaries {
            limit: OPERATION_SUMMARY_LIMIT,
            available: true,
            filter: operation_summary_filter(correlation_id),
            unavailable_reason: None,
            entries,
        },
        Err(_) => unavailable_operation_summaries(
            "read-only operation-log query unavailable",
            correlation_id,
        ),
    }
}

async fn query_operation_summaries_read_only(
    database_url: &str,
    correlation_id: Option<&str>,
) -> Result<Vec<OperationSummary>> {
    let options = SqliteConnectOptions::from_str(database_url)
        .map_err(|error| anyhow!(error.to_string()))?
        .read_only(true)
        .create_if_missing(false)
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(options).await?;

    let table_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'operation_log'",
    )
    .fetch_one(&pool)
    .await?;
    if table_count == 0 {
        return Err(anyhow!("operation_log table is not present"));
    }

    let limit = i64::try_from(OPERATION_SUMMARY_LIMIT)
        .context("operation summary limit exceeds sqlite i64 range")?;
    let rows = if let Some(correlation_id) = correlation_id {
        sqlx::query(
            "SELECT operation_id, occurred_at, namespace, entrypoint, operation_kind, status, correlation_id FROM operation_log WHERE correlation_id = ? ORDER BY occurred_at DESC, operation_id DESC LIMIT ?",
        )
        .bind(correlation_id)
        .bind(limit)
        .fetch_all(&pool)
        .await?
    } else {
        sqlx::query(
            "SELECT operation_id, occurred_at, namespace, entrypoint, operation_kind, status, correlation_id FROM operation_log ORDER BY occurred_at DESC, operation_id DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(&pool)
        .await?
    };

    Ok(rows
        .into_iter()
        .map(|entry| OperationSummary {
            operation_id: safe_metadata_value(entry.get::<String, _>("operation_id")),
            occurred_at: entry.get("occurred_at"),
            namespace: safe_namespace_shape(entry.get::<Option<String>, _>("namespace")),
            entrypoint: safe_metadata_value(entry.get::<String, _>("entrypoint")),
            operation_kind: safe_metadata_value(entry.get::<String, _>("operation_kind")),
            status: safe_metadata_value(entry.get::<String, _>("status")),
            correlation_id: entry
                .get::<Option<String>, _>("correlation_id")
                .map(safe_metadata_value),
            read_only: true,
        })
        .collect())
}

fn safe_namespace_shape(namespace: Option<String>) -> Option<String> {
    namespace.map(|namespace| {
        if namespace == "self" || namespace == "world" {
            namespace
        } else if namespace.starts_with("project/") {
            "project/<namespace>".to_string()
        } else if namespace.starts_with("user/") {
            "user/<namespace>".to_string()
        } else if metadata_value_is_sensitive(&namespace) {
            "<redacted-metadata>".to_string()
        } else {
            "<namespace>".to_string()
        }
    })
}

fn safe_metadata_value(value: String) -> String {
    if metadata_value_is_sensitive(&value) {
        "<redacted-metadata>".to_string()
    } else {
        value
    }
}

fn metadata_value_is_sensitive(value: &str) -> bool {
    value_has_secret_marker(value) || is_local_path_like(value)
}

fn value_has_secret_marker(value: &str) -> bool {
    let lower = value.to_lowercase();
    lower.contains("sk-")
        || lower.contains("api_key")
        || lower.contains("api-key")
        || lower.contains("apikey")
        || lower.contains("token")
        || lower.contains("password")
        || lower.contains("secret")
        || lower.contains("authorization")
        || lower.contains("bearer")
        || lower.contains("private")
}

fn unavailable_operation_summaries(
    reason: &'static str,
    correlation_id: Option<&str>,
) -> OperationSummaries {
    OperationSummaries {
        limit: OPERATION_SUMMARY_LIMIT,
        available: false,
        filter: operation_summary_filter(correlation_id),
        unavailable_reason: Some(reason),
        entries: Vec::new(),
    }
}

fn operation_summary_filter(correlation_id: Option<&str>) -> Option<OperationSummaryFilter> {
    correlation_id.map(|correlation_id| OperationSummaryFilter {
        correlation_id: correlation_id.to_string(),
    })
}

fn validate_operation_correlation_id(correlation_id: Option<&str>) -> Result<()> {
    let Some(correlation_id) = correlation_id else {
        return Ok(());
    };
    let Some(uuid_value) = correlation_id.strip_prefix("mcp-tool-call-") else {
        return Err(anyhow!(
            "correlation id must start with mcp-tool-call- before it can be included in a support bundle"
        ));
    };
    let uuid = Uuid::parse_str(uuid_value).map_err(|_| {
        anyhow!(
            "correlation id must use the generated mcp-tool-call-<uuid-v4> shape before it can be included in a support bundle"
        )
    })?;
    if uuid.get_version() != Some(Version::Random) || uuid.to_string() != uuid_value {
        return Err(anyhow!(
            "correlation id must use the canonical generated mcp-tool-call-<uuid-v4> shape"
        ));
    }
    Ok(())
}

fn release_metadata(project_root: &Path, generated_at: &str) -> ReleaseMetadata {
    ReleaseMetadata {
        generated_at: generated_at.to_string(),
        git_commit: command_output(project_root, "git", &["rev-parse", "--short", "HEAD"]),
        git_branch: command_output(project_root, "git", &["branch", "--show-current"]),
        platform: std::env::consts::OS.to_string(),
        rustc_version: command_output(project_root, "rustc", &["--version"]),
        verification_commands: vec![
            "cargo fmt --check",
            "git diff --check",
            "cargo clippy --all-targets --all-features -- -D warnings",
            "cargo test",
            "./scripts/agent-llm-mm.sh doctor",
            "bash scripts/product-smoke-local.sh",
        ],
    }
}

fn product_smoke_summary(project_root: &Path) -> ProductSmokeSummary {
    let latest = project_root.join("target/reports/self-revision-demo/latest");
    let required_artifacts = vec![
        "doctor.json",
        "snapshot-before.json",
        "snapshot-after.json",
        "decision-before.json",
        "decision-after.json",
        "timeline.json",
        "sqlite-summary.json",
        "report.md",
    ];
    let latest_artifacts_present = required_artifacts
        .iter()
        .all(|artifact| latest.join(artifact).is_file());

    ProductSmokeSummary {
        required_artifacts,
        latest_artifacts_present,
        self_revision_write_path_expected: "run_reflection",
    }
}

fn manifest(generated_at: &str, output_dir: &Path) -> Result<Manifest> {
    let files = vec![
        "manifest.json",
        "doctor.json",
        "config-shape.json",
        "operation-summaries.json",
        "release-metadata.json",
        "product-smoke-summary.json",
        "local-log-excerpts.json",
    ];
    let integrity = manifest_integrity(output_dir, &files)?;

    Ok(Manifest {
        bundle_format: "agent-llm-mm-local-alpha-support-bundle-v1",
        generated_at: generated_at.to_string(),
        local_only: true,
        artifact_scope: "local-only-diagnostic-artifact",
        upload_performed: false,
        production_support_channel: false,
        remote_support_surface: false,
        safety_checks: ManifestSafetyChecks {
            read_only: true,
            runtime_bootstrap_performed: false,
            sqlite_files_included: false,
            toml_files_included: false,
            raw_log_files_included: false,
            provider_payloads_included: false,
        },
        excluded_by_default: vec![
            "api keys",
            "authorization headers",
            "bearer values",
            "raw provider payloads",
            "full sqlite databases",
            "unredacted toml files",
            "raw local log files",
            "provider url userinfo, path, and query values",
            "ssh keys",
            "cookies",
            "browser session data",
        ],
        files,
        integrity,
        bounds: ManifestBounds {
            local_log_excerpts: local_log_bounds(),
        },
    })
}

fn manifest_integrity(output_dir: &Path, files: &[&'static str]) -> Result<ManifestIntegrity> {
    let integrity_files = files
        .iter()
        .copied()
        .filter(|file| *file != "manifest.json")
        .map(|file| manifest_integrity_file(output_dir, file))
        .collect::<Result<Vec<_>>>()?;

    Ok(ManifestIntegrity {
        algorithm: "sha256",
        covers: "non-manifest bundle files listed in manifest.files",
        files: integrity_files,
    })
}

fn manifest_integrity_file(output_dir: &Path, file: &'static str) -> Result<ManifestIntegrityFile> {
    let path = output_dir.join(file);
    let bytes = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(ManifestIntegrityFile {
        path: file,
        sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
        size_bytes: bytes.len() as u64,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn database_url_shape(database_url: &str) -> String {
    if database_url.starts_with("sqlite://") {
        "sqlite://<local-path>".to_string()
    } else {
        "<redacted>".to_string()
    }
}

fn sqlite_database_file_path(database_url: &str) -> Option<PathBuf> {
    let path = database_url.strip_prefix("sqlite://")?;
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if path.is_empty() || path == ":memory:" {
        return None;
    }

    #[cfg(windows)]
    let path = normalize_windows_sqlite_path(path);

    #[cfg(not(windows))]
    let path = path.to_string();

    Some(PathBuf::from(path))
}

#[cfg(windows)]
fn normalize_windows_sqlite_path(path: &str) -> String {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return path[1..].to_string();
    }

    path.to_string()
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

fn redact_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            if value_has_secret_marker(name) {
                "<local-path>/<redacted-name>".to_string()
            } else {
                format!("<local-path>/{name}")
            }
        })
        .unwrap_or_else(|| "<local-path>".to_string())
}

fn serde_name(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

fn command_output(cwd: &Path, command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
