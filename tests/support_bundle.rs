use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    domain::operation_log::{ActorKind, OperationLogEntry, OperationLogKind, OperationLogStatus},
    ports::OperationLogStore,
    support::{
        config::{
            AppConfig, DashboardConfig, ModelConfig, ModelProviderKind, OpenAiCompatibleConfig,
            TransportKind,
        },
        support_bundle::{SupportBundleOptions, generate_support_bundle},
    },
};
use chrono::{Duration, Utc};
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command};
use tempfile::tempdir;

#[tokio::test]
async fn support_bundle_generates_redacted_local_diagnostics() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir.path().join("support-bundle.sqlite");
    let database_url = sqlite_url(database_path.clone());
    let store = SqliteStore::bootstrap(&database_url)
        .await
        .expect("store should bootstrap");
    let secret = "sk-support-secret-12345";
    let base_url_user = "bundle-user";
    let base_url_secret = "base-url-token-12345";
    let base_url_query_secret = "base-url-query-secret";
    let now = Utc::now();

    for index in 0..30 {
        store
            .append_operation(OperationLogEntry {
                operation_id: format!("op-{index:02}"),
                occurred_at: now + Duration::seconds(index),
                namespace: Some("self".to_string()),
                actor_kind: ActorKind::System,
                actor_id: "mcp-stdio".to_string(),
                entrypoint: "ingest_interaction".to_string(),
                operation_kind: OperationLogKind::Tool,
                status: OperationLogStatus::Ok,
                correlation_id: Some(format!("corr-{index:02}")),
                request_summary_json: Some(format!(r#"{{"secret":"{secret}"}}"#)),
                response_summary_json: Some(r#"{"event_id":"event-1"}"#.to_string()),
                diagnostic_summary_json: None,
                redaction_version: 1,
            })
            .await
            .expect("operation append should succeed");
    }
    let identity_count_before = table_count(&database_url, "identity_claims").await;
    let commitment_count_before = table_count(&database_url, "commitments").await;

    let config = AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        transport: TransportKind::Stdio,
        database_url: database_url.clone(),
        model_provider: ModelProviderKind::OpenAiCompatible,
        model_config: ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: format!(
                "https://{base_url_user}:{base_url_secret}@api.example.test/v1?token={base_url_query_secret}"
            ),
            api_key: secret.to_string(),
            model: "gpt-4o-mini".to_string(),
            timeout_ms: 30_000,
        }),
        dashboard: DashboardConfig {
            enabled: true,
            host: "127.0.0.1".to_string(),
            port: 8787,
            base_path: "/agent-llm-mm".to_string(),
            event_capacity: 2000,
            sse_enabled: true,
            open_browser: false,
            required: true,
        },
        ..Default::default()
    };
    let output_dir = temp_dir.path().join("bundle");

    generate_support_bundle(SupportBundleOptions {
        config,
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    assert_eq!(
        table_count(&database_url, "identity_claims").await,
        identity_count_before,
        "support bundle generation must not write identity rows"
    );
    assert_eq!(
        table_count(&database_url, "commitments").await,
        commitment_count_before,
        "support bundle generation must not write commitment rows"
    );

    for file in [
        "manifest.json",
        "doctor.json",
        "config-shape.json",
        "operation-summaries.json",
        "release-metadata.json",
        "product-smoke-summary.json",
        "local-log-excerpts.json",
    ] {
        assert!(
            output_dir.join(file).is_file(),
            "support bundle should include {file}"
        );
    }

    let all_bundle_text = fs::read_dir(&output_dir)
        .expect("bundle dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| fs::read_to_string(entry.path()).expect("bundle file text"))
        .collect::<Vec<_>>()
        .join("\n");
    let lower_bundle_text = all_bundle_text.to_lowercase();

    assert!(!all_bundle_text.contains(secret));
    assert!(!all_bundle_text.contains(base_url_user));
    assert!(!all_bundle_text.contains(base_url_secret));
    assert!(!all_bundle_text.contains(base_url_query_secret));
    assert!(!all_bundle_text.contains(database_path.to_string_lossy().as_ref()));
    assert!(!lower_bundle_text.contains("api_key"));
    assert!(!all_bundle_text.contains("Authorization"));
    assert!(!all_bundle_text.contains("Bearer"));
    assert!(
        fs::read_dir(&output_dir)
            .expect("bundle entries")
            .filter_map(|entry| entry.ok())
            .all(|entry| entry.path().extension().and_then(|ext| ext.to_str()) != Some("sqlite")),
        "support bundle must not include full sqlite databases by default"
    );

    let doctor = read_json(output_dir.join("doctor.json"));
    assert_eq!(doctor["status"], "config-shape-ok");
    assert_eq!(doctor["database_url_shape"], "sqlite://<local-path>");
    assert_eq!(doctor["self_revision_write_path"], "run_reflection");
    assert_eq!(doctor["runtime_bootstrap_performed"], false);

    let manifest = read_json(output_dir.join("manifest.json"));
    assert_eq!(manifest["local_only"], true);
    assert_eq!(manifest["upload_performed"], false);
    assert_eq!(manifest["safety_checks"]["read_only"], true);
    assert_eq!(
        manifest["safety_checks"]["runtime_bootstrap_performed"],
        false
    );
    assert_eq!(manifest["safety_checks"]["sqlite_files_included"], false);
    assert_eq!(manifest["safety_checks"]["toml_files_included"], false);
    assert_eq!(manifest["safety_checks"]["raw_log_files_included"], false);
    assert_eq!(
        manifest["safety_checks"]["provider_payloads_included"],
        false
    );
    assert!(
        manifest["excluded_by_default"]
            .as_array()
            .expect("manifest excluded list")
            .iter()
            .any(|entry| entry == "provider url userinfo, path, and query values")
    );
    assert_eq!(manifest["integrity"]["algorithm"], "sha256");
    assert_eq!(
        manifest["integrity"]["covers"],
        "non-manifest bundle files listed in manifest.files"
    );
    let manifest_files = manifest["files"].as_array().expect("manifest files");
    let integrity_files = manifest["integrity"]["files"]
        .as_array()
        .expect("integrity file entries");
    assert_eq!(integrity_files.len(), manifest_files.len() - 1);
    assert!(integrity_files.iter().any(|entry| {
        entry["path"] == "doctor.json"
            && entry["sha256"].as_str().is_some_and(|value| {
                value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
            })
    }));

    let config_shape = read_json(output_dir.join("config-shape.json"));
    assert_eq!(config_shape["model"]["provider"], "openai-compatible");
    assert_eq!(
        config_shape["model"]["base_url"],
        "https://api.example.test/<redacted-path>"
    );
    assert_eq!(config_shape["model"]["credential_configured"], true);
    assert_eq!(config_shape["dashboard"]["enabled"], true);
    assert_eq!(config_shape["database_url_shape"], "sqlite://<local-path>");

    let operation_summaries = read_json(output_dir.join("operation-summaries.json"));
    let entries = operation_summaries["entries"]
        .as_array()
        .expect("operation entries array");
    assert_eq!(entries.len(), 25);
    assert_eq!(operation_summaries["limit"], 25);
    assert_eq!(operation_summaries["available"], true);
    assert_eq!(operation_summaries["unavailable_reason"], Value::Null);
    assert!(entries.iter().all(|entry| entry["read_only"] == true));

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["available"], false);
    assert_eq!(log_excerpts["unavailable_reason"], "log file not requested");
    assert_eq!(log_excerpts["read_only"], true);
}

#[tokio::test]
async fn support_bundle_manifest_marks_bundle_as_local_artifact_not_support_channel() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let manifest = read_json(output_dir.join("manifest.json"));
    assert_eq!(manifest["artifact_scope"], "local-only-diagnostic-artifact");
    assert_eq!(manifest["production_support_channel"], false);
    assert_eq!(manifest["remote_support_surface"], false);

    let rendered = serde_json::to_string_pretty(&manifest).expect("json text");
    let lower_rendered = rendered.to_lowercase();
    for forbidden in [
        "upload_url",
        "ticket",
        "support channel",
        "team support",
        "remote support",
        "remote management",
    ] {
        assert!(
            !lower_rendered.contains(forbidden),
            "manifest claimed a remote or production support surface: {forbidden}"
        );
    }
}

#[tokio::test]
async fn support_bundle_reports_openrouter_config_shape_without_provider_secrets() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir.path().join("openrouter-support-bundle.sqlite");
    let database_url = sqlite_url(database_path);
    SqliteStore::bootstrap(&database_url)
        .await
        .expect("store should bootstrap");
    let output_dir = temp_dir.path().join("bundle");
    let api_key = "openrouter-support-secret";
    let url_user = "openrouter-user";
    let url_password = "openrouter-password";
    let path_secret = "sk-openrouter-path-secret";
    let query_secret = "openrouter-query-secret";

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig {
            local_embedding: Default::default(),
            retrieval: Default::default(),
            transport: TransportKind::Stdio,
            database_url,
            model_provider: ModelProviderKind::OpenRouter,
            model_config: ModelConfig::OpenRouter(OpenAiCompatibleConfig {
                base_url: format!(
                    "https://{url_user}:{url_password}@openrouter.example.test/api/{path_secret}/v1?token={query_secret}"
                ),
                api_key: api_key.to_string(),
                model: "openrouter/test-model".to_string(),
                timeout_ms: 45_000,
            }),
            dashboard: Default::default(),
            ..Default::default()
        },
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let config_shape = read_json(output_dir.join("config-shape.json"));
    assert_eq!(config_shape["model"]["provider"], "openrouter");
    assert_eq!(
        config_shape["model"]["base_url"],
        "https://openrouter.example.test/<redacted-path>"
    );
    assert_eq!(config_shape["model"]["model"], "openrouter/test-model");
    assert_eq!(config_shape["model"]["timeout_ms"], 45_000);
    assert_eq!(config_shape["model"]["credential_configured"], true);

    let all_bundle_text = fs::read_dir(&output_dir)
        .expect("bundle dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| fs::read_to_string(entry.path()).expect("bundle file text"))
        .collect::<Vec<_>>()
        .join("\n");

    for forbidden in [api_key, url_user, url_password, path_secret, query_secret] {
        assert!(
            !all_bundle_text.contains(forbidden),
            "support bundle leaked OpenRouter provider secret: {forbidden}"
        );
    }
}

#[tokio::test]
async fn support_bundle_does_not_create_or_bootstrap_missing_database() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir.path().join("missing.sqlite");
    let output_dir = temp_dir.path().join("bundle");
    let config = AppConfig {
        local_embedding: Default::default(),
        retrieval: Default::default(),
        database_url: sqlite_url(database_path.clone()),
        ..Default::default()
    };

    generate_support_bundle(SupportBundleOptions {
        config,
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate without bootstrapping db");

    assert!(
        !database_path.exists(),
        "support bundle generation must not create or migrate the sqlite database"
    );

    let doctor = read_json(output_dir.join("doctor.json"));
    assert_eq!(doctor["runtime_bootstrap_performed"], false);
    assert_eq!(doctor["self_revision_write_path"], "run_reflection");

    let operation_summaries = read_json(output_dir.join("operation-summaries.json"));
    assert_eq!(operation_summaries["available"], false);
    assert_eq!(
        operation_summaries["unavailable_reason"],
        "sqlite database file not found"
    );
    assert_eq!(
        operation_summaries["entries"]
            .as_array()
            .expect("entries array")
            .len(),
        0
    );
}

#[tokio::test]
async fn support_bundle_filters_operation_summaries_by_correlation_id() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir.path().join("support-bundle-filter.sqlite");
    let database_url = sqlite_url(database_path);
    let store = SqliteStore::bootstrap(&database_url)
        .await
        .expect("store should bootstrap");
    let now = Utc::now();
    let target_correlation_id = "mcp-tool-call-018fbc89-9ac1-4f5d-8b2a-1f6f5f27b201";
    let secret = "sk-correlation-filter-secret";

    for (operation_id, correlation_id, occurred_at) in [
        (
            "matching-older",
            target_correlation_id,
            now - Duration::seconds(1),
        ),
        (
            "other-correlation",
            "mcp-tool-call-018fbc89-9ac1-4f5d-8b2a-1f6f5f27b202",
            now,
        ),
        (
            "matching-newer",
            target_correlation_id,
            now + Duration::seconds(1),
        ),
    ] {
        store
            .append_operation(OperationLogEntry {
                operation_id: operation_id.to_string(),
                occurred_at,
                namespace: Some("self".to_string()),
                actor_kind: ActorKind::System,
                actor_id: "mcp-stdio".to_string(),
                entrypoint: "ingest_interaction".to_string(),
                operation_kind: OperationLogKind::Tool,
                status: OperationLogStatus::Ok,
                correlation_id: Some(correlation_id.to_string()),
                request_summary_json: Some(format!(r#"{{"api_key":"{secret}"}}"#)),
                response_summary_json: Some(r#"{"event_id":"event-1"}"#.to_string()),
                diagnostic_summary_json: Some(format!(r#"{{"token":"{secret}"}}"#)),
                redaction_version: 1,
            })
            .await
            .expect("operation append should succeed");
    }

    let output_dir = temp_dir.path().join("bundle");
    generate_support_bundle(SupportBundleOptions {
        config: AppConfig {
            local_embedding: Default::default(),
            retrieval: Default::default(),
            database_url: database_url.clone(),
            ..Default::default()
        },
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: Some(target_correlation_id.to_string()),
    })
    .await
    .expect("support bundle should generate");

    let operation_summaries = read_json(output_dir.join("operation-summaries.json"));
    assert_eq!(operation_summaries["available"], true);
    assert_eq!(operation_summaries["limit"], 25);
    assert_eq!(
        operation_summaries["filter"]["correlation_id"],
        target_correlation_id
    );

    let entries = operation_summaries["entries"]
        .as_array()
        .expect("operation entries array");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["operation_id"], "matching-newer");
    assert_eq!(entries[0]["operation_kind"], "tool");
    assert_eq!(entries[0]["status"], "ok");
    assert!(
        entries[0].get("kind").is_none(),
        "support bundle operation metadata should use operation_kind, not kind"
    );
    assert_eq!(entries[1]["operation_id"], "matching-older");
    assert!(entries.iter().all(|entry| {
        entry["correlation_id"] == target_correlation_id && entry["read_only"] == true
    }));

    let rendered = serde_json::to_string_pretty(&operation_summaries).expect("json text");
    assert!(!rendered.contains("other-correlation"));
    assert!(!rendered.contains(secret));
    assert!(!rendered.contains("request_summary"));
    assert!(!rendered.contains("diagnostic_summary"));
}

#[tokio::test]
async fn support_bundle_redacts_secret_like_operation_metadata() {
    let temp_dir = tempdir().expect("temp dir");
    let database_path = temp_dir
        .path()
        .join("support-bundle-secret-metadata.sqlite");
    let database_url = sqlite_url(database_path);
    let store = SqliteStore::bootstrap(&database_url)
        .await
        .expect("store should bootstrap");
    let now = Utc::now();

    store
        .append_operation(OperationLogEntry {
            operation_id: "op-sk-secret-token".to_string(),
            occurred_at: now,
            namespace: Some("project/sk-secret-/Users/Alice/private-case".to_string()),
            actor_kind: ActorKind::System,
            actor_id: "mcp-stdio".to_string(),
            entrypoint: "ingest_interaction".to_string(),
            operation_kind: OperationLogKind::Tool,
            status: OperationLogStatus::Ok,
            correlation_id: Some("corr-sk-secret-token".to_string()),
            request_summary_json: None,
            response_summary_json: None,
            diagnostic_summary_json: None,
            redaction_version: 1,
        })
        .await
        .expect("operation append should succeed");

    let output_dir = temp_dir.path().join("bundle");
    generate_support_bundle(SupportBundleOptions {
        config: AppConfig {
            local_embedding: Default::default(),
            retrieval: Default::default(),
            database_url: database_url.clone(),
            ..Default::default()
        },
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let operation_summaries = read_json(output_dir.join("operation-summaries.json"));
    let entries = operation_summaries["entries"]
        .as_array()
        .expect("operation entries array");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["operation_id"], "<redacted-metadata>");
    assert_eq!(entries[0]["namespace"], "project/<namespace>");
    assert_eq!(entries[0]["correlation_id"], "<redacted-metadata>");
    assert_eq!(entries[0]["entrypoint"], "ingest_interaction");

    let rendered = serde_json::to_string_pretty(&operation_summaries).expect("json text");
    for forbidden in [
        "sk-secret",
        "secret",
        "token",
        "/Users/Alice",
        "private-case",
        "op-sk-secret-token",
        "corr-sk-secret-token",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "operation summaries leaked secret-like metadata: {forbidden}"
        );
    }
}

#[tokio::test]
async fn support_bundle_rejects_non_generated_correlation_id_filter() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");

    let result = generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: Some("sk-secret-correlation".to_string()),
    })
    .await;

    let error = result.expect_err("invalid correlation ids must be rejected");
    assert_error_contains(&error, "correlation id must start with mcp-tool-call-");
    assert!(
        !output_dir.join("operation-summaries.json").exists(),
        "invalid correlation ids must not produce bundle artifacts"
    );
    assert!(
        !output_dir.exists(),
        "invalid correlation ids should fail before creating the bundle directory"
    );
}

#[tokio::test]
async fn support_bundle_rejects_prefixed_secret_like_correlation_id_filter() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");

    let result = generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: Some("mcp-tool-call-sk-secret-token".to_string()),
    })
    .await;

    let error = result.expect_err("prefixed secret-like correlation ids must be rejected");
    assert_error_contains(
        &error,
        "correlation id must use the generated mcp-tool-call-<uuid-v4> shape",
    );
    assert!(
        !output_dir.join("operation-summaries.json").exists(),
        "invalid correlation ids must not produce bundle artifacts"
    );
    assert!(
        !output_dir.exists(),
        "invalid correlation ids should fail before creating the bundle directory"
    );
}

#[tokio::test]
async fn support_bundle_rejects_non_v4_correlation_id_filter() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");

    let result = generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: Some(
            "mcp-tool-call-00000000-0000-1000-8000-000000000000".to_string(),
        ),
    })
    .await;

    let error = result.expect_err("non-v4 correlation ids must be rejected");
    assert_error_contains(
        &error,
        "correlation id must use the canonical generated mcp-tool-call-<uuid-v4> shape",
    );
    assert!(
        !output_dir.join("operation-summaries.json").exists(),
        "invalid correlation ids must not produce bundle artifacts"
    );
    assert!(
        !output_dir.exists(),
        "invalid correlation ids should fail before creating the bundle directory"
    );
}

#[tokio::test]
async fn support_bundle_rejects_non_canonical_correlation_id_filter() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");

    let result = generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: Some(
            "mcp-tool-call-018FBC89-9AC1-4F5D-8B2A-1F6F5F27B201".to_string(),
        ),
    })
    .await;

    let error = result.expect_err("non-canonical correlation ids must be rejected");
    assert_error_contains(
        &error,
        "correlation id must use the canonical generated mcp-tool-call-<uuid-v4> shape",
    );
    assert!(
        !output_dir.join("operation-summaries.json").exists(),
        "invalid correlation ids must not produce bundle artifacts"
    );
    assert!(
        !output_dir.exists(),
        "invalid correlation ids should fail before creating the bundle directory"
    );
}

#[tokio::test]
async fn support_bundle_redacts_secret_like_config_and_log_filenames() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let config_path = temp_dir
        .path()
        .join("agent-sk-secret-token-private-user.toml");
    let log_path = temp_dir
        .path()
        .join("trace-sk-secret-token-private-user.log");
    fs::write(&log_path, "INFO safe support bundle line\n").expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: Some(config_path),
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let config_shape = read_json(output_dir.join("config-shape.json"));
    assert_eq!(config_shape["config_path"], "<local-path>/<redacted-name>");
    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["source"], "<local-path>/<redacted-name>");

    let rendered = fs::read_dir(&output_dir)
        .expect("bundle dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_file())
        .map(|entry| fs::read_to_string(entry.path()).expect("bundle file text"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        "sk-secret",
        "secret",
        "token",
        "private-user",
        "agent-sk-secret-token-private-user.toml",
        "trace-sk-secret-token-private-user.log",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "support bundle leaked secret-like path metadata: {forbidden}"
        );
    }
}

#[tokio::test]
async fn support_bundle_generates_bounded_redacted_local_log_excerpts() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_dir = temp_dir.path().join("logs");
    fs::create_dir_all(&log_dir).expect("log dir");
    let log_path = log_dir.join("agent-llm-mm.local.log");
    let local_toml = temp_dir.path().join("agent-llm-mm.local.toml");
    let local_sqlite = temp_dir.path().join("support.sqlite");
    let local_secret = "sk-local-support-secret-1234567890";
    let bearer = "Bearer support-bearer-secret";
    let api_key = "api_key=support-api-key";
    let token = "token=support-token";
    let password = "password=support-password";
    let secret = "secret=support-secret";
    let provider_url = "https://support-user:support-pass@provider.example.test/v1?token=query-secret&api_key=query-key";
    let provider_url_without_query = "https://plain-user:plain-pass@provider.example.test/v1";
    let sqlite_url = format!("sqlite://{}", local_sqlite.display());
    let ordinary_local_path = temp_dir.path().join("Downloads/error.txt");

    let payload = format!(
        "startup ok\n\
         Authorization: Bearer opaque-auth-value\n\
         Authorization=Bearer equals-auth-value\n\
         Authorization: {bearer}\n\
         token plain-token-value\n\
         password plain-password-value\n\
         secret plain-secret-value\n\
         database_url={sqlite_url}\n\
         config_path={} sqlite_path={}\n\
         error_path={}\n\
         {api_key} {token} {password} {secret}\n\
         {{\"api_key\":\"json-secret-value\",\"note\":\"summary ok\"}}\n\
         {{\"token\":\"json-token-value\"}}\n\
         {{\"password\":\"json-password-value\"}}\n\
         {{\"secret\":\"json-secret-field-value\"}}\n\
         headers={{\"authorization\":\"Bearer json-auth-value\"}}\n\
         password:\"json-password\" token:\"json-token\"\n\
         openai_api_key: plain-colon-key\n\
         provider_url={provider_url}\n\
         provider_url={provider_url_without_query}\n\
         ssh_key=/Users/private/.ssh/id_ed25519\n\
         {{\"messages\":[{{\"role\":\"user\",\"content\":\"raw provider message should be removed\"}}]}}\n\
         {{\"input\":\"raw provider input should be removed\"}}\n\
         {{\"prompt\":\"raw user prompt should be removed\",\"request\":{{\"messages\":[\"hidden\"]}},\"response\":{{\"text\":\"hidden\"}},\"tool_args\":{{\"path\":\"/Users/private/.ssh/id_ed25519\"}},\"api_key\":\"json-secret\",\"authorization\":\"Bearer json-bearer\",\"note\":\"keep tiny summary\"}}\n\
         cookie browser session localStorage profile should not leak\n\
         final line with {local_secret}\n",
        local_toml.display(),
        local_sqlite.display(),
        ordinary_local_path.display()
    );
    fs::write(&log_path, payload).expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path.clone()),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    assert!(
        !output_dir.join("agent-llm-mm.local.log").exists(),
        "support bundle must not copy raw log files"
    );

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["available"], true);
    assert_eq!(log_excerpts["unavailable_reason"], Value::Null);
    assert_eq!(log_excerpts["read_only"], true);
    assert_eq!(log_excerpts["redaction_version"], 1);
    assert_eq!(log_excerpts["line_count"], 26);
    assert_eq!(log_excerpts["line_number_scope"], "file");
    assert_eq!(log_excerpts["input_truncated"], false);
    assert_eq!(
        log_excerpts["source"],
        "<local-path>/agent-llm-mm.local.log"
    );
    assert_eq!(log_excerpts["bounds"]["max_excerpts"], 12);
    assert_eq!(log_excerpts["bounds"]["max_excerpt_chars"], 240);
    assert_eq!(log_excerpts["bounds"]["max_total_bytes"], 4096);
    assert_eq!(log_excerpts["bounds"]["max_input_bytes"], 65536);
    assert_eq!(log_excerpts["bounds"]["tail_lines"], 200);
    assert!(
        log_excerpts["exclusions"]
            .as_array()
            .expect("exclusions array")
            .iter()
            .any(|entry| entry == "raw provider payloads")
    );

    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    let rendered_lower = rendered.to_lowercase();
    for forbidden in [
        local_secret,
        "support-bearer-secret",
        "support-api-key",
        "support-token",
        "support-password",
        "support-secret",
        "support-user",
        "support-pass",
        "plain-user",
        "plain-pass",
        "query-secret",
        "query-key",
        "opaque-auth-value",
        "equals-auth-value",
        "plain-token-value",
        "plain-password-value",
        "plain-secret-value",
        "json-secret-value",
        "json-token-value",
        "json-password-value",
        "json-secret-field-value",
        "json-auth-value",
        "json-password",
        "json-token",
        "plain-colon-key",
        local_toml.to_string_lossy().as_ref(),
        local_sqlite.to_string_lossy().as_ref(),
        ordinary_local_path.to_string_lossy().as_ref(),
        "/Users/private/.ssh/id_ed25519",
        "raw provider message should be removed",
        "raw provider input should be removed",
        "raw user prompt should be removed",
        "\"messages\"",
        "\"content\"",
        "\"input\"",
        "json-secret",
        "json-bearer",
        "cookie",
        "browser session",
        "localstorage",
        "profile",
    ] {
        assert!(
            !rendered_lower.contains(&forbidden.to_lowercase()),
            "local log excerpts leaked forbidden text: {forbidden}"
        );
    }

    assert!(!rendered.contains("Authorization"));
    assert!(!rendered.contains("Bearer"));
    assert!(!rendered.contains("api_key"));
    assert!(!rendered.contains("password"));
    assert!(!rendered.contains("secret"));
    assert!(!rendered.contains("token"));
    assert!(rendered.contains("<redacted"));
    assert!(rendered.contains("<local-path>/agent-llm-mm.local.toml"));
    assert!(rendered.contains("<local-path>/support.sqlite"));

    let manifest = read_json(output_dir.join("manifest.json"));
    assert_eq!(manifest["local_only"], true);
    assert_eq!(manifest["upload_performed"], false);
    assert!(
        manifest["files"]
            .as_array()
            .expect("manifest files")
            .iter()
            .any(|entry| entry == "local-log-excerpts.json")
    );
    assert!(
        manifest["excluded_by_default"]
            .as_array()
            .expect("manifest exclusions")
            .iter()
            .any(|entry| entry == "raw local log files")
    );
    assert!(
        manifest["bounds"]["local_log_excerpts"]["max_input_bytes"] == 65536,
        "manifest should record local log bounds"
    );
}

#[tokio::test]
async fn support_bundle_skips_unquoted_provider_payload_fields() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("provider-payload.log");
    fs::write(
        &log_path,
        "INFO startup ok\n\
         INFO messages=user raw private message should not enter\n\
         INFO content=user private content should not enter\n\
         INFO input=user private input should not enter\n\
         INFO messages = raw private spaced message should not enter\n\
         INFO content = private spaced content should not enter\n\
         INFO input = private spaced input should not enter\n\
         INFO safe summary retained\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "raw private message should not enter",
        "private content should not enter",
        "private input should not enter",
        "messages=user",
        "content=user",
        "input=user",
        "raw private spaced message should not enter",
        "private spaced content should not enter",
        "private spaced input should not enter",
        "messages =",
        "content =",
        "input =",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked provider payload field: {forbidden}"
        );
    }
    assert!(rendered.contains("INFO startup ok"));
    assert!(rendered.contains("INFO safe summary retained"));
}

#[tokio::test]
async fn support_bundle_skips_raw_provider_payload_and_diagnostic_fields() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("raw-provider-diagnostics.log");
    fs::write(
        &log_path,
        "INFO startup ok\n\
         INFO provider_payload=raw provider prompt text should not enter\n\
         INFO provider_request_body=raw request body should not enter\n\
         INFO provider_response_body=raw response body should not enter\n\
         INFO provider_diagnostic=raw provider diagnostic should not enter\n\
         INFO raw_diagnostics=raw diagnostic text should not enter\n\
         INFO diagnostic_summary_json=raw diagnostic json should not enter\n\
         INFO safe summary retained\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "raw provider prompt text should not enter",
        "raw request body should not enter",
        "raw response body should not enter",
        "raw provider diagnostic should not enter",
        "raw diagnostic text should not enter",
        "raw diagnostic json should not enter",
        "provider_payload=",
        "provider_request_body=",
        "provider_response_body=",
        "provider_diagnostic=",
        "raw_diagnostics=",
        "diagnostic_summary_json=",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked raw provider diagnostic field: {forbidden}"
        );
    }
    assert!(rendered.contains("INFO startup ok"));
    assert!(rendered.contains("INFO safe summary retained"));
}

#[tokio::test]
async fn support_bundle_preserves_safe_request_response_metadata() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("safe-metadata.log");
    fs::write(
        &log_path,
        "INFO request_id=req-123 response_status=500 response_time_ms=42 profile=local-alpha\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    assert!(rendered.contains("request_id=req-123"));
    assert!(rendered.contains("response_status=500"));
    assert!(rendered.contains("response_time_ms=42"));
    assert!(rendered.contains("profile=local-alpha"));
}

#[tokio::test]
async fn support_bundle_skips_session_material() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("session-material.log");
    fs::write(
        &log_path,
        "INFO session_id=abc123 status=ok\n\
         INFO session token retained-by-cookie should not leak\n\
         INFO safe summary retained\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in ["session_id", "abc123", "retained-by-cookie"] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked session material: {forbidden}"
        );
    }
    assert!(rendered.contains("INFO safe summary retained"));
}

#[tokio::test]
async fn support_bundle_redacts_hyphenated_api_key_markers() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("credential-marker.log");
    fs::write(
        &log_path,
        "INFO provider x-api-key: leak-me-12345\n\
         INFO provider api-key=leak-me-67890\n\
         {\"api-key\":\"json-hyphen-secret\",\"note\":\"keep\"}\n\
         INFO payload={\"openai_api_key\":\"leak-openai-json\",\"note\":\"x\"}\n\
         INFO safe summary retained\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "leak-me-12345",
        "leak-me-67890",
        "json-hyphen-secret",
        "leak-openai-json",
        "x-api-key",
        "openai_api_key",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked hyphenated API key marker: {forbidden}"
        );
    }
    assert!(rendered.contains("<redacted-value>"));
    assert!(rendered.contains("INFO safe summary retained"));
}

#[tokio::test]
async fn support_bundle_skips_prefixed_json_compound_secret_keys() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("prefixed-json-secrets.log");
    fs::write(
        &log_path,
        "INFO payload={\"access_token\":\"leak-access-token\",\"refresh_token\":\"leak-refresh-token\",\"provider_token\":\"leak-provider-token\",\"client_secret\":\"leak-client-secret\",\"user_password\":\"leak-user-password\",\"note\":\"safe\"}\n\
         INFO safe summary retained\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "leak-access-token",
        "leak-refresh-token",
        "leak-provider-token",
        "leak-client-secret",
        "leak-user-password",
        "access_token",
        "refresh_token",
        "provider_token",
        "client_secret",
        "user_password",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked prefixed JSON compound secret: {forbidden}"
        );
    }
    assert!(rendered.contains("INFO safe summary retained"));
}

#[tokio::test]
async fn support_bundle_redacts_spaced_secret_markers() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("spaced-secrets.log");
    fs::write(
        &log_path,
        "INFO token = plain-token-value\n\
         INFO password = plain-password-value\n\
         INFO secret : plain-secret-value\n\
         INFO Authorization : Bearer bearer-value\n\
         INFO Authorization:Bearer compact-bearer-value\n\
         INFO api key = plain-api-key-value\n\
         INFO API key: colon-api-key-value\n\
         INFO access key = access-key-value\n\
         INFO bearer token = bearer-token-value\n\
         INFO safe summary retained\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "plain-token-value",
        "plain-password-value",
        "plain-secret-value",
        "bearer-value",
        "compact-bearer-value",
        "plain-api-key-value",
        "colon-api-key-value",
        "access-key-value",
        "bearer-token-value",
        "Authorization",
        "Bearer",
        "token =",
        "password =",
        "secret :",
        "api key =",
        "API key:",
        "access key =",
        "bearer token =",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked spaced secret marker: {forbidden}"
        );
    }
    assert!(rendered.contains("<redacted"));
    assert!(rendered.contains("INFO safe summary retained"));
}

#[tokio::test]
async fn support_bundle_skips_ssh_private_key_blocks() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("ssh-key.log");
    fs::write(
        &log_path,
        "INFO startup ok\n\
         -----BEGIN OPENSSH PRIVATE KEY-----\n\
         b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQ==\n\
         -----END OPENSSH PRIVATE KEY-----\n\
         -----BEGIN RSA PRIVATE KEY-----\n\
         private-key-body-line\n\
         -----END RSA PRIVATE KEY-----\n\
         INFO safe summary retained\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "BEGIN OPENSSH PRIVATE KEY",
        "END OPENSSH PRIVATE KEY",
        "BEGIN RSA PRIVATE KEY",
        "END RSA PRIVATE KEY",
        "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQ==",
        "private-key-body-line",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked SSH private key block text: {forbidden}"
        );
    }
    assert!(rendered.contains("INFO startup ok"));
    assert!(rendered.contains("INFO safe summary retained"));
}

#[tokio::test]
async fn support_bundle_skips_oversized_tail_inside_unclosed_private_key_block() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("oversized-unclosed-ssh-key.log");
    let key_body = (0..2500)
        .map(|index| format!("BASE64LIKEKEYBODY{index:04}SHOULDNOTLEAK"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        &log_path,
        format!(
            "INFO before key\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\n\
             {key_body}\n"
        ),
    )
    .expect("oversized unclosed ssh key log");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["input_truncated"], true);
    assert_eq!(log_excerpts["line_number_scope"], "tail");
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "BASE64LIKEKEYBODY",
        "SHOULDNOTLEAK",
        "BEGIN OPENSSH PRIVATE KEY",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked unclosed oversized private key tail text: {forbidden}"
        );
    }
}

#[tokio::test]
async fn support_bundle_skips_short_private_key_body_lines_in_oversized_tail() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("oversized-short-key-tail.log");
    let key_body = std::iter::repeat_n("abc001", 30000)
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        &log_path,
        format!(
            "INFO before key\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\n\
             {key_body}\n"
        ),
    )
    .expect("oversized short private key body log");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["input_truncated"], true);
    assert_eq!(log_excerpts["line_number_scope"], "tail");
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in ["abc001", "BEGIN OPENSSH PRIVATE KEY"] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked short private key body tail text: {forbidden}"
        );
    }
}

#[tokio::test]
async fn support_bundle_redacts_windows_style_local_paths() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("windows-paths.log");
    fs::write(
        &log_path,
        "INFO windows_path=C:\\Users\\Alice\\Private\\support.txt\n\
         INFO windows_config=C:\\Users\\Alice\\Private\\agent.toml windows_log=C:\\Users\\Alice\\Private\\agent.log\n\
         INFO quoted_windows_path=\"C:\\Users\\Alice\\Secret Folder\\support.txt\"\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "C:\\Users\\Alice\\Private\\support.txt",
        "C:\\Users\\Alice\\Private\\agent.toml",
        "C:\\Users\\Alice\\Private\\agent.log",
        "C:\\Users\\Alice\\Secret Folder\\support.txt",
        "Secret Folder",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked Windows local path: {forbidden}"
        );
    }
    assert!(rendered.contains("<local-path>/support.txt"));
    assert!(rendered.contains("<local-path>/agent.toml"));
    assert!(rendered.contains("<local-path>/agent.log"));
}

#[tokio::test]
async fn support_bundle_redacts_space_paths_and_preserves_following_fields() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("space-paths.log");
    fs::write(
        &log_path,
        "INFO posix_path=/Users/Alice/Secret Folder/support.txt status=ok\n\
         INFO quoted_posix_path=\"/Users/Alice/Secret Folder/agent.log\" next=ready\n\
         INFO 错误 posix_path=/Users/Alice/Secret Folder/nonascii.log trace=ok\n\
         INFO windows_path=C:\\Users\\Alice\\Secret Folder\\support.txt status=ok another=1\n",
    )
    .expect("log file");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "/Users/Alice/Secret Folder/support.txt",
        "/Users/Alice/Secret Folder/agent.log",
        "/Users/Alice/Secret Folder/nonascii.log",
        "C:\\Users\\Alice\\Secret Folder\\support.txt",
        "Secret Folder",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked local path with spaces: {forbidden}"
        );
    }
    assert!(rendered.contains("<local-path>/support.txt"));
    assert!(rendered.contains("<local-path>/agent.log"));
    assert!(rendered.contains("status=ok"));
    assert!(rendered.contains("another=1"));
    assert!(rendered.contains("next=ready"));
    assert!(rendered.contains("trace=ok"));
}

#[tokio::test]
async fn support_bundle_skips_oversized_tail_that_starts_inside_private_key_block() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("oversized-ssh-key.log");
    fs::write(
        &log_path,
        format!(
            "INFO before key\n\
             -----BEGIN OPENSSH PRIVATE KEY-----\n\
             {}\n\
             PRIVATEKEYBODY-SHOULD-NOT-LEAK\n\
             MOREPRIVATEKEYBODY\n\
             -----END OPENSSH PRIVATE KEY-----\n\
             INFO safe retained after key\n",
            "A".repeat(70_000)
        ),
    )
    .expect("oversized ssh key log");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["input_truncated"], true);
    assert_eq!(log_excerpts["line_number_scope"], "tail");
    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    for forbidden in [
        "PRIVATEKEYBODY-SHOULD-NOT-LEAK",
        "MOREPRIVATEKEYBODY",
        "BEGIN OPENSSH PRIVATE KEY",
        "END OPENSSH PRIVATE KEY",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "local log excerpts leaked oversized private key tail text: {forbidden}"
        );
    }
    assert!(rendered.contains("INFO safe retained after key"));
}

#[tokio::test]
async fn support_bundle_marks_oversized_log_input_as_tail_scoped() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("oversized.log");
    let partial_prefix = format!(
        "partial-line-start secret-unbounded-prefix {}\n",
        "x".repeat(70_000)
    );
    fs::write(
        &log_path,
        format!("{partial_prefix}INFO retained tail line\n"),
    )
    .expect("oversized log");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["available"], true);
    assert_eq!(log_excerpts["input_truncated"], true);
    assert_eq!(log_excerpts["line_number_scope"], "tail");
    assert_eq!(log_excerpts["line_count"], 1);

    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    assert!(
        rendered.contains("INFO retained tail line"),
        "tail line should remain available"
    );
    assert!(
        !rendered.contains("secret-unbounded-prefix"),
        "partial first retained line must be discarded after tail seek"
    );
}

#[tokio::test]
async fn support_bundle_discards_oversized_single_line_log_tail() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    let log_path = temp_dir.path().join("oversized-single-line.log");
    fs::write(
        &log_path,
        format!(
            "huge single-line prefix json-secret-field-value {}",
            "x".repeat(70_000)
        ),
    )
    .expect("oversized single-line log");

    generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: Some(log_path),
        operation_correlation_id: None,
    })
    .await
    .expect("support bundle should generate");

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["available"], true);
    assert_eq!(log_excerpts["input_truncated"], true);
    assert_eq!(log_excerpts["line_number_scope"], "tail");
    assert_eq!(log_excerpts["line_count"], 0);
    assert_eq!(
        log_excerpts["excerpts"]
            .as_array()
            .expect("excerpts array")
            .len(),
        0
    );

    let rendered = serde_json::to_string_pretty(&log_excerpts).expect("json text");
    assert!(
        !rendered.contains("json-secret-field-value"),
        "partial single-line tail content must not be included"
    );
}

#[tokio::test]
async fn support_bundle_refuses_non_empty_output_directory() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("bundle");
    fs::create_dir_all(&output_dir).expect("output dir");
    fs::write(
        output_dir.join("agent-llm-mm.local.toml"),
        "api_key = \"sk-existing\"",
    )
    .expect("pre-existing file");

    let result = generate_support_bundle(SupportBundleOptions {
        config: AppConfig::default(),
        output_dir: output_dir.clone(),
        config_path: None,
        project_root: temp_dir.path().to_path_buf(),
        local_log_path: None,
        operation_correlation_id: None,
    })
    .await;

    let error = result.expect_err("non-empty output dirs must be rejected");
    assert!(
        error.to_string().contains("must be empty"),
        "unexpected error: {error:#}"
    );
    assert!(
        !output_dir.join("manifest.json").exists(),
        "generator must fail before writing support bundle artifacts"
    );
}

#[test]
fn support_bundle_script_uses_dedicated_generator_binary() {
    let script =
        fs::read_to_string("scripts/generate-support-bundle.sh").expect("script should exist");
    let binary =
        fs::read_to_string("src/bin/generate_support_bundle.rs").expect("binary should exist");
    let mode = fs::metadata("scripts/generate-support-bundle.sh")
        .expect("script metadata")
        .permissions()
        .mode();

    assert!(script.contains("cargo run --quiet --bin generate_support_bundle --"));
    assert!(script.contains("usage: ./scripts/generate-support-bundle.sh"));
    assert!(script.contains("--log-file"));
    assert!(script.contains("--correlation-id"));
    assert!(binary.contains("--correlation-id"));
    assert!(binary.contains("missing value for --correlation-id"));
    assert!(!script.contains("log path does not exist"));
    assert!(script.contains("mkdir -p \"${output_parent}\""));
    assert!(binary.contains("unknown option"));
    assert_ne!(mode & 0o111, 0, "script should be directly executable");
}

#[test]
fn support_bundle_script_forwards_correlation_id_filter() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("script-correlation-bundle");
    let correlation_id = "mcp-tool-call-018fbc89-9ac1-4f5d-8b2a-1f6f5f27b203";

    let output = Command::new("bash")
        .arg("scripts/generate-support-bundle.sh")
        .arg(&output_dir)
        .arg("--correlation-id")
        .arg(correlation_id)
        .output()
        .expect("support bundle script should run");
    assert!(
        output.status.success(),
        "script should forward correlation id filters\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let operation_summaries = read_json(output_dir.join("operation-summaries.json"));
    assert_eq!(
        operation_summaries["filter"]["correlation_id"],
        correlation_id
    );
}

#[test]
fn support_bundle_script_rejects_empty_correlation_id_filter() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("script-empty-correlation-bundle");

    let output = Command::new("bash")
        .arg("scripts/generate-support-bundle.sh")
        .arg(&output_dir)
        .arg("--correlation-id")
        .arg("")
        .output()
        .expect("support bundle script should run");
    assert!(
        !output.status.success(),
        "empty correlation id filters must fail\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output_dir.exists(),
        "empty correlation id filters must not create bundle output"
    );
}

#[test]
fn support_bundle_script_forwards_empty_log_file_as_explicit_request() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("script-empty-log-bundle");

    let output = Command::new("bash")
        .arg("scripts/generate-support-bundle.sh")
        .arg(&output_dir)
        .arg("--log-file")
        .arg("")
        .output()
        .expect("support bundle script should run");
    assert!(
        output.status.success(),
        "explicit empty log paths should still be forwarded to the generator\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["available"], false);
    assert_eq!(log_excerpts["unavailable_reason"], "log file not found");
    assert_eq!(log_excerpts["source"], "<local-path>");
}

#[test]
fn support_bundle_script_rejects_empty_config_path() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("script-empty-config-bundle");

    let output = Command::new("bash")
        .arg("scripts/generate-support-bundle.sh")
        .arg(&output_dir)
        .arg("")
        .output()
        .expect("support bundle script should run");
    assert!(
        !output.status.success(),
        "explicit empty config paths must not fall back to default config\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output_dir.exists(),
        "empty config paths must not create bundle output"
    );
}

#[test]
fn support_bundle_script_forwards_config_log_and_correlation_id_together() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("script-combined-bundle");
    let config_path = temp_dir.path().join("agent-llm-mm.toml");
    let log_path = temp_dir.path().join("agent-llm-mm.log");
    let database_path = temp_dir.path().join("agent-llm-mm.sqlite");
    let correlation_id = "mcp-tool-call-018fbc89-9ac1-4f5d-8b2a-1f6f5f27b204";
    fs::write(
        &config_path,
        format!(
            "transport = \"stdio\"\n\
             database_url = \"{}\"\n\
             model_provider = \"mock\"\n",
            sqlite_url(database_path)
        ),
    )
    .expect("config file");
    fs::write(&log_path, "INFO safe combined support bundle smoke\n").expect("log file");

    let output = Command::new("bash")
        .arg("scripts/generate-support-bundle.sh")
        .arg(&output_dir)
        .arg(&config_path)
        .arg("--log-file")
        .arg(&log_path)
        .arg("--correlation-id")
        .arg(correlation_id)
        .output()
        .expect("support bundle script should run");
    assert!(
        output.status.success(),
        "script should forward config, log, and correlation id together\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let operation_summaries = read_json(output_dir.join("operation-summaries.json"));
    assert_eq!(
        operation_summaries["filter"]["correlation_id"],
        correlation_id
    );
    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["available"], true);
}

#[test]
fn support_bundle_script_records_missing_explicit_log_as_unavailable() {
    let temp_dir = tempdir().expect("temp dir");
    let output_dir = temp_dir.path().join("script-bundle");
    let missing_log_path = temp_dir.path().join("missing.log");

    let output = Command::new("bash")
        .arg("scripts/generate-support-bundle.sh")
        .arg(&output_dir)
        .arg("--log-file")
        .arg(&missing_log_path)
        .output()
        .expect("support bundle script should run");
    assert!(
        output.status.success(),
        "script should record missing logs as unavailable instead of failing\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let log_excerpts = read_json(output_dir.join("local-log-excerpts.json"));
    assert_eq!(log_excerpts["available"], false);
    assert_eq!(log_excerpts["unavailable_reason"], "log file not found");
    assert_eq!(log_excerpts["line_number_scope"], "unavailable");
    assert_eq!(log_excerpts["source"], "<local-path>/missing.log");
}

fn sqlite_url(path: PathBuf) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

fn read_json(path: PathBuf) -> Value {
    serde_json::from_slice(&fs::read(path).expect("json file")).expect("valid json")
}

fn assert_error_contains(error: &anyhow::Error, expected: &str) {
    assert!(
        error.to_string().contains(expected),
        "unexpected error: {error:#}"
    );
}

async fn table_count(database_url: &str, table: &str) -> i64 {
    let pool = sqlx::SqlitePool::connect(database_url)
        .await
        .expect("sqlite pool");
    let sql = format!("SELECT COUNT(*) FROM {table}");
    sqlx::query_scalar::<_, i64>(&sql)
        .fetch_one(&pool)
        .await
        .expect("table count")
}
