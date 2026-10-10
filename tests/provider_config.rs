use agent_llm_mm::{
    run_doctor,
    support::config::{
        AppConfig, DATABASE_URL_ENV_VAR, ModelConfig, ModelProviderKind, OpenAiCompatibleConfig,
        TransportKind,
    },
    support::doctor::ProviderSupportState,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use tempfile::tempdir;

#[test]
fn default_config_uses_mock_provider_when_no_config_file_is_present() {
    let config = AppConfig::default();

    assert_eq!(config.model_provider, ModelProviderKind::Mock);
    assert_eq!(config.model_config, ModelConfig::Mock);
}

#[test]
fn load_from_path_reads_openai_compatible_provider_from_toml_file() {
    let temp_dir = tempdir().expect("temp dir");
    let config_path = temp_dir.path().join("agent-llm-mm.local.toml");
    fs::write(
        &config_path,
        r#"
transport = "stdio"
database_url = "sqlite:///D:/back/agent-llm-mm-test.sqlite"

[model]
provider = "openai-compatible"

[model.openai_compatible]
base_url = "https://api.example.test/v1"
api_key = "example-test-key"
model = "gpt-4o-mini"
timeout_ms = 45000
"#,
    )
    .expect("write config");
    let _guard = EnvGuard::set([(DATABASE_URL_ENV_VAR, Some("sqlite:///tmp/from-env.sqlite"))]);

    let config = AppConfig::load_from_path(&config_path).expect("config");

    assert_eq!(config.transport, TransportKind::Stdio);
    assert_eq!(
        config.database_url,
        "sqlite:///D:/back/agent-llm-mm-test.sqlite"
    );
    assert_eq!(config.model_provider, ModelProviderKind::OpenAiCompatible);
    assert_eq!(
        config.model_config,
        ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: "https://api.example.test/v1".to_string(),
            api_key: "example-test-key".to_string(),
            model: "gpt-4o-mini".to_string(),
            timeout_ms: 45_000,
        })
    );
}

#[test]
fn load_from_path_reads_openrouter_provider_from_toml_file() {
    let temp_dir = tempdir().expect("temp dir");
    let config_path = temp_dir.path().join("agent-llm-mm-openrouter.toml");
    fs::write(
        &config_path,
        r#"
transport = "stdio"
database_url = "sqlite:///tmp/openrouter-provider.sqlite"

[model]
provider = "openrouter"

[model.openrouter]
base_url = "https://openrouter.example.test/api/v1"
api_key = "example-openrouter-key"
model = "openrouter/test-model"
timeout_ms = 55000
"#,
    )
    .expect("write config");

    let config = AppConfig::load_from_path(&config_path).expect("config");

    assert_eq!(config.model_provider, ModelProviderKind::OpenRouter);
    assert_eq!(
        config.model_config,
        ModelConfig::OpenRouter(OpenAiCompatibleConfig {
            base_url: "https://openrouter.example.test/api/v1".to_string(),
            api_key: "example-openrouter-key".to_string(),
            model: "openrouter/test-model".to_string(),
            timeout_ms: 55_000,
        })
    );
    config.validate().expect("openrouter config validates");
}

#[test]
fn load_from_path_reads_provider_api_key_from_environment_reference() {
    let temp_dir = tempdir().expect("temp dir");
    let config_path = temp_dir.path().join("agent-llm-mm-openrouter-env.toml");
    fs::write(
        &config_path,
        r#"
transport = "stdio"
database_url = "sqlite:///tmp/openrouter-provider.sqlite"

[model]
provider = "openrouter"

[model.openrouter]
base_url = "https://openrouter.example.test/api/v1"
api_key_env = "AGENT_LLM_MM_TEST_PROVIDER_API_KEY"
model = "openrouter/test-model"
timeout_ms = 55000
"#,
    )
    .expect("write config");
    let _guard = EnvGuard::set([(
        "AGENT_LLM_MM_TEST_PROVIDER_API_KEY",
        Some("env-openrouter-key"),
    )]);

    let config = AppConfig::load_from_path(&config_path).expect("config");

    assert_eq!(
        config.model_config,
        ModelConfig::OpenRouter(OpenAiCompatibleConfig {
            base_url: "https://openrouter.example.test/api/v1".to_string(),
            api_key: "env-openrouter-key".to_string(),
            model: "openrouter/test-model".to_string(),
            timeout_ms: 55_000,
        })
    );
    config
        .validate()
        .expect("provider api_key_env config validates");
}

#[test]
fn load_prefers_config_path_from_environment() {
    let temp_dir = tempdir().expect("temp dir");
    let config_path = temp_dir.path().join("custom-provider.toml");
    fs::write(
        &config_path,
        r#"
[model]
provider = "mock"
"#,
    )
    .expect("write config");

    let _guard = EnvGuard::set([(
        "AGENT_LLM_MM_CONFIG",
        Some(config_path.to_string_lossy().as_ref()),
    )]);

    let config = AppConfig::load().expect("config");

    assert_eq!(config.model_provider, ModelProviderKind::Mock);
    assert_eq!(config.model_config, ModelConfig::Mock);
}

#[test]
fn load_prefers_database_url_env_over_default_config_file() {
    let temp_dir = tempdir().expect("temp dir");
    let config_path = temp_dir.path().join("agent-llm-mm.local.toml");
    fs::write(
        &config_path,
        r#"
transport = "stdio"
database_url = "sqlite:///tmp/from-config-file.sqlite"

[model]
provider = "mock"
"#,
    )
    .expect("write config");

    let _guard = ProcessContextGuard::apply(
        temp_dir.path(),
        [
            ("AGENT_LLM_MM_CONFIG", None),
            (DATABASE_URL_ENV_VAR, Some("sqlite:///tmp/from-env.sqlite")),
        ],
    );

    let config = AppConfig::load().expect("config");

    assert_eq!(config.database_url, "sqlite:///tmp/from-env.sqlite");
    assert_eq!(config.model_provider, ModelProviderKind::Mock);
}

#[test]
fn dev_example_config_parses_without_real_secrets() {
    let config = load_example_config("agent-llm-mm.dev.example.toml");

    assert_eq!(config.transport, TransportKind::Stdio);
    assert_eq!(config.model_provider, ModelProviderKind::Mock);
    assert_eq!(config.model_config, ModelConfig::Mock);
    assert!(!config.dashboard.enabled);
    assert!(!config.daemon.enabled);
    config
        .validate()
        .expect("dev example config should validate");
}

#[test]
fn prod_local_example_config_parses_with_local_dashboard_and_disabled_daemon() {
    let config = load_example_config("agent-llm-mm.prod-local.example.toml");

    assert_eq!(config.transport, TransportKind::Stdio);
    assert_eq!(config.model_provider, ModelProviderKind::OpenAiCompatible);
    let ModelConfig::OpenAiCompatible(provider_config) = &config.model_config else {
        panic!("prod-local example should use openai-compatible provider settings");
    };
    assert_eq!(provider_config.api_key, "REPLACE_WITH_LOCAL_SECRET");
    assert!(
        !provider_config.api_key.starts_with("sk-"),
        "prod-local example must not contain a live-looking API key"
    );
    assert_eq!(config.dashboard.host, "127.0.0.1");
    assert!(!config.daemon.enabled);
    config
        .validate()
        .expect("prod-local example config structure should validate");
}

#[test]
fn openrouter_example_config_parses_without_live_looking_secret() {
    let _guard = EnvGuard::set([(
        "AGENT_LLM_MM_OPENROUTER_API_KEY",
        Some("example-openrouter-key"),
    )]);
    let config = load_example_config("agent-llm-mm.openrouter.example.toml");

    assert_eq!(config.transport, TransportKind::Stdio);
    assert_eq!(config.model_provider, ModelProviderKind::OpenRouter);
    let ModelConfig::OpenRouter(provider_config) = &config.model_config else {
        panic!("openrouter example should use openrouter provider settings");
    };
    assert_eq!(provider_config.api_key, "example-openrouter-key");
    assert!(
        !provider_config.api_key.starts_with("sk-"),
        "OpenRouter example must not contain a live-looking API key"
    );
    assert_eq!(provider_config.model, "openrouter/auto");
    assert!(!config.daemon.enabled);
    config
        .validate()
        .expect("openrouter example config structure should validate");
}

#[test]
fn generic_example_config_parses_and_keeps_daemon_disabled() {
    let config = load_example_config("agent-llm-mm.example.toml");

    assert_eq!(config.transport, TransportKind::Stdio);
    assert!(!config.daemon.enabled);
    config
        .validate()
        .expect("generic example config should validate");
}

#[test]
fn provider_matrix_lists_supported_and_future_providers_as_contract_only() {
    let entries = AppConfig::provider_matrix();

    assert_eq!(entries.len(), 7);
    assert_eq!(entries[0].provider, "mock");
    assert_eq!(entries[0].state, "supported");
    assert!(entries[0].configurable);
    assert_eq!(entries[0].adapter, "built-in deterministic mock");

    assert_eq!(entries[1].provider, "openai-compatible");
    assert_eq!(entries[1].state, "supported");
    assert!(entries[1].configurable);
    assert_eq!(entries[1].adapter, "openai-compatible chat completions");

    assert_eq!(entries[2].provider, "openrouter");
    assert_eq!(entries[2].state, "supported");
    assert!(entries[2].configurable);
    assert_eq!(
        entries[2].adapter,
        "openrouter chat completions via openai-compatible transport"
    );
    assert!(entries[2].missing_implementation.is_empty());

    for entry in &entries[5..] {
        assert_eq!(entry.state, "planned-only");
        assert!(
            !entry.configurable,
            "{} must not be configurable before an adapter exists",
            entry.provider
        );
        assert_eq!(entry.adapter, "not implemented");
        assert!(
            entry.missing_implementation.contains("config parser")
                && entry.missing_implementation.contains("doctor diagnostics")
                && entry.missing_implementation.contains("model adapter")
                && entry.missing_implementation.contains("MCP stdio tests"),
            "future provider {} must explain every readiness prerequisite; got {}",
            entry.provider,
            entry.missing_implementation
        );
    }

    let future_names: Vec<_> = entries[5..].iter().map(|entry| entry.provider).collect();
    assert_eq!(future_names, ["azure-openai", "local"]);
}

#[test]
fn future_providers_are_rejected_by_config_parser_until_implemented() {
    let temp_dir = tempdir().expect("temp dir");

    for provider in ["azure-openai", "local"] {
        let config_path = temp_dir
            .path()
            .join(format!("unsupported-provider-{provider}.toml"));
        fs::write(
            &config_path,
            format!(
                r#"
[model]
provider = "{provider}"
"#
            ),
        )
        .expect("write config");

        let error = AppConfig::load_from_path(&config_path)
            .expect_err("future provider must not parse as a usable provider");

        assert!(
            error.contains("unsupported configuration value"),
            "parse error should safely reject provider {provider}: {error}"
        );
    }
}

#[tokio::test]
async fn doctor_reports_openrouter_provider_without_exposing_api_key() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("doctor-openrouter.sqlite"));
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url,
        model_provider: ModelProviderKind::OpenRouter,
        model_config: ModelConfig::OpenRouter(OpenAiCompatibleConfig {
            base_url: "https://doctor-user:doctor-password@openrouter.example.test/api/sk-doctor-path-secret/v1?token=doctor-query-secret".to_string(),
            api_key: "openrouter-secret-key".to_string(),
            model: "openrouter/test-model".to_string(),
            timeout_ms: 30_000,
        }),
        dashboard: Default::default(),
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor should pass");
    let serialized = serde_json::to_string(&report).expect("doctor report serializes");

    assert_eq!(report.provider, ModelProviderKind::OpenRouter);
    assert_eq!(report.model.as_deref(), Some("openrouter/test-model"));
    assert_eq!(
        report.base_url.as_deref(),
        Some("https://openrouter.example.test/<redacted-path>")
    );
    assert!(
        report
            .provider_matrix
            .iter()
            .any(|entry| entry.provider == "openrouter"
                && entry.support_state == ProviderSupportState::Supported
                && entry.configurable
                && entry.selected)
    );
    assert!(
        !serialized.contains("openrouter-secret-key"),
        "doctor output must not expose provider api keys"
    );
    for forbidden in [
        "doctor-user",
        "doctor-password",
        "sk-doctor-path-secret",
        "doctor-query-secret",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "doctor output must not expose provider URL secrets: {forbidden}"
        );
    }
}

#[tokio::test]
async fn doctor_fails_when_openrouter_provider_config_is_missing_model() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(
        temp_dir
            .path()
            .join("doctor-openrouter-missing-model.sqlite"),
    );
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url,
        model_provider: ModelProviderKind::OpenRouter,
        model_config: ModelConfig::OpenRouter(OpenAiCompatibleConfig {
            base_url: "https://openrouter.example.test/api/v1".to_string(),
            api_key: "example-openrouter-key".to_string(),
            model: String::new(),
            timeout_ms: 30_000,
        }),
        dashboard: Default::default(),
        ..Default::default()
    };

    let error = run_doctor(config).await.expect_err("doctor should fail");

    assert!(error.to_string().contains("openrouter"));
    assert!(error.to_string().contains("model"));
}

#[tokio::test]
async fn doctor_fails_when_openai_provider_config_is_missing_api_key() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("doctor.sqlite"));
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url,
        model_provider: ModelProviderKind::OpenAiCompatible,
        model_config: ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: "https://api.example.test/v1".to_string(),
            api_key: String::new(),
            model: "gpt-4o-mini".to_string(),
            timeout_ms: 30_000,
        }),
        dashboard: Default::default(),
        ..Default::default()
    };

    let error = run_doctor(config).await.expect_err("doctor should fail");

    assert!(error.to_string().contains("api_key"));
}

#[tokio::test]
async fn doctor_report_does_not_contain_api_key_in_serialized_output() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("doctor-redact.sqlite"));
    let secret = "sk-super-secret-key-12345";
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url,
        model_provider: ModelProviderKind::OpenAiCompatible,
        model_config: ModelConfig::OpenAiCompatible(OpenAiCompatibleConfig {
            base_url: "https://api.example.test/v1".to_string(),
            api_key: secret.to_string(),
            model: "gpt-4o-mini".to_string(),
            timeout_ms: 30_000,
        }),
        dashboard: Default::default(),
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor");
    let json = serde_json::to_string(&report).expect("serialize");

    assert!(
        !json.contains(secret),
        "doctor output must not contain the api_key secret"
    );
}

#[tokio::test]
async fn doctor_reports_provider_matrix_without_marking_future_providers_supported() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("doctor-matrix.sqlite"));
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url,
        model_provider: ModelProviderKind::Mock,
        model_config: ModelConfig::Mock,
        dashboard: Default::default(),
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor");

    assert_eq!(report.provider_matrix.len(), 7);
    assert_eq!(report.provider_matrix[0].provider, "mock");
    assert_eq!(
        report.provider_matrix[0].support_state,
        ProviderSupportState::Supported
    );
    assert!(report.provider_matrix[0].configurable);
    assert!(report.provider_matrix[0].selected);
    assert!(report.provider_matrix[0].selectable_for_runtime);

    assert_eq!(report.provider_matrix[1].provider, "openai-compatible");
    assert_eq!(
        report.provider_matrix[1].support_state,
        ProviderSupportState::Supported
    );
    assert!(report.provider_matrix[1].configurable);
    assert!(!report.provider_matrix[1].selected);

    assert_eq!(report.provider_matrix[2].provider, "openrouter");
    assert_eq!(
        report.provider_matrix[2].support_state,
        ProviderSupportState::Supported
    );
    assert!(report.provider_matrix[2].configurable);
    assert!(!report.provider_matrix[2].selected);
    assert!(report.provider_matrix[2].missing_implementation.is_empty());

    for entry in &report.provider_matrix[5..] {
        assert_eq!(entry.support_state, ProviderSupportState::PlannedOnly);
        assert!(!entry.configurable);
        assert!(!entry.selected);
        assert!(!entry.selectable_for_runtime);
        assert!(
            entry.missing_implementation.contains("model adapter"),
            "{} should expose missing implementation details",
            entry.provider
        );
    }

    let json = serde_json::to_string(&report).expect("serialize");
    assert!(json.contains(r#""support_state":"planned-only""#));
    assert!(json.contains(r#""provider":"azure-openai""#));
    assert!(json.contains(r#""provider":"openrouter""#));
    assert!(json.contains(r#""provider":"local""#));
    assert!(json.contains("MCP stdio tests"));
}

#[tokio::test]
async fn provider_matrix_marks_only_supported_configurable_rows_selectable_for_runtime() {
    let temp_dir = tempdir().expect("temp dir");
    let database_url = sqlite_url(temp_dir.path().join("doctor-selectable.sqlite"));
    let config = AppConfig {
        transport: TransportKind::Stdio,
        database_url,
        model_provider: ModelProviderKind::Mock,
        model_config: ModelConfig::Mock,
        dashboard: Default::default(),
        ..Default::default()
    };

    let report = run_doctor(config).await.expect("doctor");

    // 派生标记必须等价于 supported 且 configurable，planned-only 行恒为 false。
    for entry in &report.provider_matrix {
        let expected = entry.support_state == ProviderSupportState::Supported && entry.configurable;
        assert_eq!(
            entry.selectable_for_runtime, expected,
            "{} selectable_for_runtime should track supported && configurable",
            entry.provider
        );
        if entry.support_state == ProviderSupportState::PlannedOnly {
            assert!(
                !entry.selectable_for_runtime,
                "{} is planned-only and must never be selectable for runtime",
                entry.provider
            );
        }
    }
}

fn sqlite_url(path: PathBuf) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

fn load_example_config(file_name: &str) -> AppConfig {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples")
        .join(file_name);
    AppConfig::load_from_path(path).expect("example config should parse")
}

struct EnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: HashMap<&'static str, Option<String>>,
}

impl EnvGuard {
    fn set<const N: usize>(pairs: [(&'static str, Option<&str>); N]) -> Self {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        let lock = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock");

        let mut previous = HashMap::new();
        for (key, value) in pairs {
            previous.insert(key, std::env::var(key).ok());
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }

        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, value) in &self.previous {
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
    }
}

struct ProcessContextGuard {
    _env: EnvGuard,
    previous_dir: PathBuf,
}

impl ProcessContextGuard {
    fn apply<const N: usize>(dir: &Path, pairs: [(&'static str, Option<&str>); N]) -> Self {
        let env_guard = EnvGuard::set(pairs);
        let previous_dir = std::env::current_dir().expect("current dir");
        std::env::set_current_dir(dir).expect("set current dir");

        Self {
            _env: env_guard,
            previous_dir,
        }
    }
}

impl Drop for ProcessContextGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.previous_dir).expect("restore current dir");
    }
}

#[test]
fn native_provider_config_defaults_and_explicit_fields_are_supported() {
    for (provider, section, kind) in [
        (
            "openai-responses",
            "openai_responses",
            ModelProviderKind::OpenAiResponses,
        ),
        ("anthropic", "anthropic", ModelProviderKind::Anthropic),
    ] {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("native.toml");
        let source = format!(
            r#"
[model]
provider = "{provider}"
[model.{section}]
base_url = "https://example.test/v1"
api_key = "native-secret"
model = "test-model"
"#
        );
        fs::write(&path, &source).expect("write config");
        let config = AppConfig::load_from_path(&path).expect("native config");
        config.validate().expect("valid native config");
        assert_eq!(config.model_provider, kind);
        let native = match &config.model_config {
            ModelConfig::OpenAiResponses(native) | ModelConfig::Anthropic(native) => native,
            other => panic!("wrong native config: {other:?}"),
        };
        assert_eq!(native.max_tokens, 2048);
        assert_eq!(native.timeout_ms, 30_000);
        assert_eq!(native.temperature, None);
        assert_eq!(config.doctor_model().as_deref(), Some("test-model"));
        assert_eq!(
            config.doctor_base_url().as_deref(),
            Some("https://example.test/v1")
        );
        assert!(!format!("{config:?}").contains("native-secret"));
        fs::write(
            &path,
            format!("{source}\nmax_tokens = 512\ntemperature = 0.5\ntimeout_ms = 10000\n"),
        )
        .expect("write config");
        let config = AppConfig::load_from_path(&path).expect("native config");
        config.validate().expect("explicit settings valid");
        let native = match config.model_config {
            ModelConfig::OpenAiResponses(native) | ModelConfig::Anthropic(native) => native,
            _ => unreachable!(),
        };
        assert_eq!(native.max_tokens, 512);
        assert_eq!(native.temperature, Some(0.5));
        assert_eq!(native.timeout_ms, 10_000);
    }
}

#[test]
fn native_provider_validation_rejects_invalid_settings_and_mismatches() {
    use agent_llm_mm::support::config::NativeModelConfig;
    let valid = NativeModelConfig {
        base_url: "https://example.test/v1".into(),
        api_key: "secret".into(),
        model: "test-model".into(),
        timeout_ms: 1000,
        max_tokens: 2048,
        temperature: None,
    };
    for native in [
        NativeModelConfig {
            base_url: " ".into(),
            ..valid.clone()
        },
        NativeModelConfig {
            api_key: " ".into(),
            ..valid.clone()
        },
        NativeModelConfig {
            model: " ".into(),
            ..valid.clone()
        },
        NativeModelConfig {
            timeout_ms: 0,
            ..valid.clone()
        },
        NativeModelConfig {
            max_tokens: 0,
            ..valid.clone()
        },
        NativeModelConfig {
            temperature: Some(-0.1),
            ..valid.clone()
        },
        NativeModelConfig {
            temperature: Some(f32::NAN),
            ..valid.clone()
        },
        NativeModelConfig {
            temperature: Some(f32::INFINITY),
            ..valid.clone()
        },
        NativeModelConfig {
            temperature: Some(2.1),
            ..valid.clone()
        },
    ] {
        assert!(native.validate("openai-responses").is_err());
        assert!(native.validate("anthropic").is_err());
    }
    let higher_temperature = NativeModelConfig {
        temperature: Some(1.5),
        ..valid.clone()
    };
    assert!(higher_temperature.validate("openai-responses").is_ok());
    assert!(higher_temperature.validate("anthropic").is_err());
    assert!(
        AppConfig {
            model_provider: ModelProviderKind::Anthropic,
            model_config: ModelConfig::OpenAiResponses(valid),
            ..AppConfig::default()
        }
        .validate_model_config()
        .is_err()
    );
}

#[test]
fn native_provider_env_credentials_follow_existing_precedence() {
    let _guard = EnvGuard::set([("AGENT_LLM_MM_TEST_NATIVE_KEY", Some("native-env-secret"))]);
    for (provider, section) in [
        ("openai-responses", "openai_responses"),
        ("anthropic", "anthropic"),
    ] {
        let directory = tempdir().expect("temp dir");
        let path = directory.path().join("native.toml");
        for (inline, expected) in [
            ("", "native-env-secret"),
            ("inline-secret", "inline-secret"),
        ] {
            fs::write(
                &path,
                format!(
                    r#"
[model]
provider = "{provider}"
[model.{section}]
base_url = "https://example.test/v1"
api_key = "{inline}"
api_key_env = "AGENT_LLM_MM_TEST_NATIVE_KEY"
model = "test-model"
"#
                ),
            )
            .expect("write config");
            let config = AppConfig::load_from_path(&path).expect("config");
            config.validate().expect("valid config");
            let native = match config.model_config {
                ModelConfig::OpenAiResponses(native) | ModelConfig::Anthropic(native) => native,
                _ => unreachable!(),
            };
            assert_eq!(native.api_key, expected);
        }
    }
}

#[test]
fn native_provider_matrix_is_runtime_supported_without_live_certification_claims() {
    let matrix = AppConfig::provider_matrix();
    for name in ["openai-responses", "anthropic"] {
        let entry = matrix
            .iter()
            .find(|entry| entry.provider == name)
            .expect("native entry");
        assert_eq!(entry.state, "supported");
        assert!(entry.configurable);
        assert!(entry.adapter.contains("not live-certified"));
        assert!(
            entry
                .missing_implementation
                .contains("live provider certification")
        );
    }
}

#[test]
fn malformed_config_diagnostics_do_not_echo_inline_credentials() {
    let directory = tempdir().expect("temp dir");
    let path = directory.path().join("malformed.toml");
    for (provider, section) in [
        ("openai-compatible", "openai_compatible"),
        ("openrouter", "openrouter"),
        ("openai-responses", "openai_responses"),
        ("anthropic", "anthropic"),
    ] {
        fs::write(&path, format!("[model]\nprovider = \"{provider}\"\n[model.{section}]\napi_key = \"credential-must-never-appear\" extra\n")).expect("write malformed config");
        let error = AppConfig::load_from_path(&path).expect_err("malformed TOML");
        assert!(!error.contains("credential-must-never-appear"));
        assert!(!error.contains("api_key"));
        assert!(
            error.contains("line 4"),
            "safe diagnostic should identify the location: {error}"
        );
    }
}
