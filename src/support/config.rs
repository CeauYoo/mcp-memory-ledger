use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    str::FromStr,
};

pub const DATABASE_URL_ENV_VAR: &str = "AGENT_LLM_MM_DATABASE_URL";
pub const CONFIG_PATH_ENV_VAR: &str = "AGENT_LLM_MM_CONFIG";
pub const DEFAULT_CONFIG_FILE_NAME: &str = "agent-llm-mm.local.toml";
const DEFAULT_OPENAI_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_DATABASE_DIR_NAME: &str = "agent-llm-mm";
const DEFAULT_DATABASE_FILE_NAME: &str = "agent-llm-mm.sqlite";
const FUTURE_PROVIDER_MISSING_IMPLEMENTATION: &str = "config parser, doctor diagnostics, model adapter, error handling, redaction, and MCP stdio tests";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransportKind {
    Stdio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelProviderKind {
    Mock,
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "openrouter")]
    OpenRouter,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    Anthropic,
}

impl ModelProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ModelProviderKind::Mock => "mock",
            ModelProviderKind::OpenAiCompatible => "openai-compatible",
            ModelProviderKind::OpenRouter => "openrouter",
            ModelProviderKind::OpenAiResponses => "openai-responses",
            ModelProviderKind::Anthropic => "anthropic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderMatrixEntry {
    pub provider: &'static str,
    pub state: &'static str,
    pub configurable: bool,
    pub adapter: &'static str,
    pub missing_implementation: &'static str,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OpenAiCompatibleConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_ms: u64,
}

#[derive(Clone, PartialEq)]
pub struct NativeModelConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

impl std::fmt::Debug for OpenAiCompatibleConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OpenAiCompatibleConfig")
            .field("base_url", &"<redacted>")
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("timeout_ms", &self.timeout_ms)
            .finish()
    }
}

impl std::fmt::Debug for NativeModelConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeModelConfig")
            .field("base_url", &"<redacted>")
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("timeout_ms", &self.timeout_ms)
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .finish()
    }
}

impl NativeModelConfig {
    pub fn validate(&self, provider: &str) -> Result<(), String> {
        validate_native_config(provider, self)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ModelConfig {
    Mock,
    OpenAiCompatible(OpenAiCompatibleConfig),
    OpenRouter(OpenAiCompatibleConfig),
    OpenAiResponses(NativeModelConfig),
    Anthropic(NativeModelConfig),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub base_path: String,
    pub event_capacity: usize,
    pub sse_enabled: bool,
    pub open_browser: bool,
    pub required: bool,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 8787,
            base_path: "/".to_string(),
            event_capacity: 2000,
            sse_enabled: true,
            open_browser: false,
            required: false,
        }
    }
}

impl DashboardConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.event_capacity == 0 {
            return Err("dashboard.event_capacity must be greater than 0".to_string());
        }
        if !self.base_path.starts_with('/') {
            return Err("dashboard.base_path must start with /".to_string());
        }
        if self.enabled && !is_loopback_dashboard_host(&self.host) {
            return Err(
                "dashboard.host must be localhost or a loopback IP while dashboard authentication is unavailable"
                    .to_string(),
            );
        }
        Ok(())
    }
}

fn is_loopback_dashboard_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || IpAddr::from_str(host).is_ok_and(|address| address.is_loopback())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub poll_interval_ms: u64,
    pub max_concurrent_tasks: u32,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_interval_ms: 60_000,
            max_concurrent_tasks: 1,
        }
    }
}

impl DaemonConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.enabled && self.poll_interval_ms == 0 {
            return Err("daemon.poll_interval_ms must be greater than 0".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppConfig {
    pub transport: TransportKind,
    pub database_url: String,
    pub model_provider: ModelProviderKind,
    pub model_config: ModelConfig,
    pub dashboard: DashboardConfig,
    pub daemon: DaemonConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            transport: TransportKind::Stdio,
            database_url: default_database_url(),
            model_provider: ModelProviderKind::Mock,
            model_config: ModelConfig::Mock,
            dashboard: DashboardConfig::default(),
            daemon: DaemonConfig::default(),
        }
    }
}

impl AppConfig {
    pub fn provider_matrix() -> Vec<ProviderMatrixEntry> {
        vec![
            ProviderMatrixEntry {
                provider: "mock",
                state: "supported",
                configurable: true,
                adapter: "built-in deterministic mock",
                missing_implementation: "",
            },
            ProviderMatrixEntry {
                provider: "openai-compatible",
                state: "supported",
                configurable: true,
                adapter: "openai-compatible chat completions",
                missing_implementation: "",
            },
            ProviderMatrixEntry {
                provider: "openrouter",
                state: "supported",
                configurable: true,
                adapter: "openrouter chat completions via openai-compatible transport",
                missing_implementation: "",
            },
            ProviderMatrixEntry {
                provider: "openai-responses",
                state: "supported",
                configurable: true,
                adapter: "native OpenAI Responses; offline-tested, not live-certified",
                missing_implementation: "live provider certification runner",
            },
            ProviderMatrixEntry {
                provider: "anthropic",
                state: "supported",
                configurable: true,
                adapter: "native Anthropic Messages; offline-tested, not live-certified",
                missing_implementation: "live provider certification runner",
            },
            ProviderMatrixEntry {
                provider: "azure-openai",
                state: "planned-only",
                configurable: false,
                adapter: "not implemented",
                missing_implementation: FUTURE_PROVIDER_MISSING_IMPLEMENTATION,
            },
            ProviderMatrixEntry {
                provider: "local",
                state: "planned-only",
                configurable: false,
                adapter: "not implemented",
                missing_implementation: FUTURE_PROVIDER_MISSING_IMPLEMENTATION,
            },
        ]
    }

    pub fn load() -> Result<Self, String> {
        let mut config = match std::env::var(CONFIG_PATH_ENV_VAR) {
            Ok(path) => Self::load_from_path(path),
            Err(_) => {
                let default_path = PathBuf::from(DEFAULT_CONFIG_FILE_NAME);
                if default_path.exists() {
                    Self::load_from_path(default_path)
                } else {
                    Ok(Self::default())
                }
            }
        }?;

        if let Ok(database_url) = std::env::var(DATABASE_URL_ENV_VAR) {
            config.database_url = database_url;
        }

        Ok(config)
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let content = fs::read_to_string(path)
            .map_err(|error| format!("failed to read config file {}: {error}", path.display()))?;
        let file_config: FileConfig = toml::from_str(&content).map_err(|error: toml::de::Error| {
            // TOML diagnostics include source snippets, which may contain inline credentials.
            // Retain a useful location but never propagate the parser's raw display/message.
            let location = error.span().map(|span| {
                let before = &content.as_bytes()[..span.start.min(content.len())];
                let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
                let column = before.iter().rev().take_while(|byte| **byte != b'\n').count() + 1;
                format!(" at line {line}, column {column}")
            }).unwrap_or_default();
            format!("failed to parse config file {}{location}: invalid TOML or unsupported configuration value", path.display())
        })?;

        let mut config = Self::default();

        if let Some(transport) = file_config.transport {
            config.transport = transport;
        }
        if let Some(database_url) = file_config.database_url {
            config.database_url = database_url;
        }
        if let Some(dashboard) = file_config.dashboard {
            if let Some(enabled) = dashboard.enabled {
                config.dashboard.enabled = enabled;
            }
            if let Some(host) = dashboard.host {
                config.dashboard.host = host;
            }
            if let Some(port) = dashboard.port {
                config.dashboard.port = port;
            }
            if let Some(base_path) = dashboard.base_path {
                config.dashboard.base_path = base_path;
            }
            if let Some(event_capacity) = dashboard.event_capacity {
                config.dashboard.event_capacity = event_capacity;
            }
            if let Some(sse_enabled) = dashboard.sse_enabled {
                config.dashboard.sse_enabled = sse_enabled;
            }
            if let Some(open_browser) = dashboard.open_browser {
                config.dashboard.open_browser = open_browser;
            }
            if let Some(required) = dashboard.required {
                config.dashboard.required = required;
            }
        }
        if let Some(model) = file_config.model {
            let provider = model.provider.unwrap_or(ModelProviderKind::Mock);
            config.model_provider = provider;
            config.model_config = match provider {
                ModelProviderKind::Mock => ModelConfig::Mock,
                ModelProviderKind::OpenAiCompatible => {
                    let openai = model.openai_compatible.unwrap_or_default();
                    ModelConfig::OpenAiCompatible(provider_config(openai))
                }
                ModelProviderKind::OpenAiResponses => ModelConfig::OpenAiResponses(
                    native_provider_config(model.openai_responses.unwrap_or_default()),
                ),
                ModelProviderKind::Anthropic => ModelConfig::Anthropic(native_provider_config(
                    model.anthropic.unwrap_or_default(),
                )),
                ModelProviderKind::OpenRouter => {
                    let openrouter = model.openrouter.unwrap_or_default();
                    ModelConfig::OpenRouter(provider_config(openrouter))
                }
            };
        }
        if let Some(daemon) = file_config.daemon {
            if let Some(enabled) = daemon.enabled {
                config.daemon.enabled = enabled;
            }
            if let Some(poll_interval_ms) = daemon.poll_interval_ms {
                config.daemon.poll_interval_ms = poll_interval_ms;
            }
            if let Some(max_concurrent_tasks) = daemon.max_concurrent_tasks {
                config.daemon.max_concurrent_tasks = max_concurrent_tasks;
            }
        }

        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.validate_model_config()?;
        self.dashboard.validate()?;
        self.daemon.validate()?;
        Ok(())
    }

    pub fn validate_model_config(&self) -> Result<(), String> {
        match (&self.model_provider, &self.model_config) {
            (ModelProviderKind::OpenAiResponses, ModelConfig::OpenAiResponses(config)) => {
                validate_native_config("openai-responses", config)
            }
            (ModelProviderKind::Anthropic, ModelConfig::Anthropic(config)) => {
                validate_native_config("anthropic", config)
            }
            (ModelProviderKind::OpenAiResponses | ModelProviderKind::Anthropic, _) => {
                Err("native model provider and model config do not match".to_string())
            }
            (ModelProviderKind::Mock, ModelConfig::Mock) => Ok(()),
            (ModelProviderKind::Mock, _) => {
                Err("model provider is mock but model config is not mock".to_string())
            }
            (ModelProviderKind::OpenAiCompatible, ModelConfig::OpenAiCompatible(config)) => {
                validate_chat_completion_config("openai-compatible", config)
            }
            (ModelProviderKind::OpenAiCompatible, _) => Err(
                "model provider is openai-compatible but model config is not openai-compatible"
                    .to_string(),
            ),
            (ModelProviderKind::OpenRouter, ModelConfig::OpenRouter(config)) => {
                validate_chat_completion_config("openrouter", config)
            }
            (ModelProviderKind::OpenRouter, _) => {
                Err("model provider is openrouter but model config is not openrouter".to_string())
            }
        }
    }

    pub fn doctor_model(&self) -> Option<String> {
        match &self.model_config {
            ModelConfig::Mock => None,
            ModelConfig::OpenAiCompatible(config) | ModelConfig::OpenRouter(config) => {
                Some(config.model.clone())
            }
            ModelConfig::OpenAiResponses(config) | ModelConfig::Anthropic(config) => {
                Some(config.model.clone())
            }
        }
    }

    pub fn doctor_base_url(&self) -> Option<String> {
        match &self.model_config {
            ModelConfig::Mock => None,
            ModelConfig::OpenAiCompatible(config) | ModelConfig::OpenRouter(config) => {
                Some(config.base_url.clone())
            }
            ModelConfig::OpenAiResponses(config) | ModelConfig::Anthropic(config) => {
                Some(config.base_url.clone())
            }
        }
    }
}

fn provider_config(config: FileOpenAiCompatibleConfig) -> OpenAiCompatibleConfig {
    OpenAiCompatibleConfig {
        base_url: config.base_url.unwrap_or_default(),
        api_key: provider_api_key(config.api_key, config.api_key_env),
        model: config.model.unwrap_or_default(),
        timeout_ms: config.timeout_ms.unwrap_or(DEFAULT_OPENAI_TIMEOUT_MS),
    }
}

fn native_provider_config(config: FileNativeModelConfig) -> NativeModelConfig {
    NativeModelConfig {
        base_url: config.base_url.unwrap_or_default(),
        api_key: provider_api_key(config.api_key, config.api_key_env),
        model: config.model.unwrap_or_default(),
        timeout_ms: config.timeout_ms.unwrap_or(DEFAULT_OPENAI_TIMEOUT_MS),
        max_tokens: config.max_tokens.unwrap_or(2048),
        temperature: config.temperature,
    }
}

fn validate_native_config(provider: &str, config: &NativeModelConfig) -> Result<(), String> {
    for (name, value) in [
        ("base_url", &config.base_url),
        ("api_key", &config.api_key),
        ("model", &config.model),
    ] {
        if value.trim().is_empty() {
            return Err(format!("missing required {provider} field: {name}"));
        }
    }
    if config.timeout_ms == 0 {
        return Err(format!("{provider} timeout_ms must be greater than 0"));
    }
    if config.max_tokens == 0 {
        return Err(format!("{provider} max_tokens must be greater than 0"));
    }
    let upper = if provider == "anthropic" { 1.0 } else { 2.0 };
    if config
        .temperature
        .is_some_and(|value| !value.is_finite() || !(0.0..=upper).contains(&value))
    {
        return Err(format!(
            "{provider} temperature must be finite and between 0 and {upper}"
        ));
    }
    Ok(())
}

fn provider_api_key(api_key: Option<String>, api_key_env: Option<String>) -> String {
    api_key
        .filter(|value| !value.trim().is_empty())
        .or_else(|| api_key_env.and_then(|env_var| std::env::var(env_var).ok()))
        .unwrap_or_default()
}

fn validate_chat_completion_config(
    provider: &str,
    config: &OpenAiCompatibleConfig,
) -> Result<(), String> {
    if config.base_url.trim().is_empty() {
        return Err(format!("missing required {provider} field: base_url"));
    }
    if config.api_key.trim().is_empty() {
        return Err(format!("missing required {provider} field: api_key"));
    }
    if config.model.trim().is_empty() {
        return Err(format!("missing required {provider} field: model"));
    }
    Ok(())
}

fn default_database_url() -> String {
    std::env::var(DATABASE_URL_ENV_VAR).unwrap_or_else(|_| sqlite_url(&default_database_path()))
}

fn sqlite_url(path: &Path) -> String {
    format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"))
}

fn default_database_path() -> PathBuf {
    default_database_base_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(DEFAULT_DATABASE_DIR_NAME)
        .join(DEFAULT_DATABASE_FILE_NAME)
}

#[cfg(target_os = "macos")]
fn default_database_base_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn default_database_base_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
}

#[cfg(windows)]
fn default_database_base_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .map(|home| home.join("AppData").join("Local"))
        })
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    transport: Option<TransportKind>,
    database_url: Option<String>,
    dashboard: Option<FileDashboardConfig>,
    model: Option<FileModelConfig>,
    daemon: Option<FileDaemonConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct FileDashboardConfig {
    enabled: Option<bool>,
    host: Option<String>,
    port: Option<u16>,
    base_path: Option<String>,
    event_capacity: Option<usize>,
    sse_enabled: Option<bool>,
    open_browser: Option<bool>,
    required: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
struct FileModelConfig {
    provider: Option<ModelProviderKind>,
    #[serde(default)]
    openai_compatible: Option<FileOpenAiCompatibleConfig>,
    #[serde(default)]
    openrouter: Option<FileOpenAiCompatibleConfig>,
    #[serde(default, alias = "openai-responses")]
    openai_responses: Option<FileNativeModelConfig>,
    #[serde(default)]
    anthropic: Option<FileNativeModelConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct FileOpenAiCompatibleConfig {
    base_url: Option<String>,
    api_key: Option<String>,
    api_key_env: Option<String>,
    model: Option<String>,
    timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct FileNativeModelConfig {
    base_url: Option<String>,
    api_key: Option<String>,
    api_key_env: Option<String>,
    model: Option<String>,
    timeout_ms: Option<u64>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
}

#[derive(Debug, Deserialize, Default)]
struct FileDaemonConfig {
    enabled: Option<bool>,
    poll_interval_ms: Option<u64>,
    max_concurrent_tasks: Option<u32>,
}
