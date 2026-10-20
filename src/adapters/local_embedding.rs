//! Explicit opt-in Ollama embeddings. Loopback is a transport restriction, not
//! proof that the operator's service never forwards data to another service.
use crate::{
    domain::types::Namespace,
    error::AppError,
    ports::embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
};
use async_trait::async_trait;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::{Instant, timeout_at};
use url::{Host, Url};

const MAX_TEXT_BYTES: usize = 256 * 1024;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_BATCH: usize = 64;
const MAX_DIMENSIONS: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LocalEmbeddingConfig {
    pub enabled: bool,
    pub cache_enabled: bool,
    pub local_only_declared: bool,
    pub namespace_allowlist: Vec<String>,
    pub endpoint: String,
    pub timeout_ms: u64,
    pub profile: EmbeddingProfile,
}
impl Default for LocalEmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cache_enabled: false,
            local_only_declared: false,
            namespace_allowlist: vec![],
            endpoint: "http://127.0.0.1:11434".into(),
            timeout_ms: 5000,
            profile: EmbeddingProfile::default(),
        }
    }
}
fn error(message: &str) -> AppError {
    AppError::Message(format!("local embedding: {message}"))
}
impl LocalEmbeddingConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        self.validate_enabled().map_err(|e| e.to_string())
    }
    fn validate_enabled(&self) -> Result<(), AppError> {
        if !self.enabled || !self.local_only_declared || self.namespace_allowlist.is_empty() {
            return Err(error(
                "requires enabled, local_only_declared and explicit namespaces",
            ));
        }
        for namespace in &self.namespace_allowlist {
            Namespace::parse(namespace.clone())
                .map_err(|_| error("invalid namespace allowlist"))?;
        }
        let url = Url::parse(&self.endpoint).map_err(|_| error("invalid endpoint"))?;
        // Validate the original authority too: URL normalization accepts integer,
        // octal and abbreviated IPv4 forms which are not literal IP addresses.
        let authority = self
            .endpoint
            .split_once("://")
            .map(|(_, rest)| rest.split('/').next().unwrap_or(""))
            .unwrap_or("");
        let literal_host = if authority.starts_with('[') {
            authority
                .split_once(']')
                .map(|(host, _)| &host[1..])
                .unwrap_or("")
        } else {
            authority.split(':').next().unwrap_or("")
        };
        let literal_loopback = literal_host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
        let loopback = match url.host() {
            Some(Host::Ipv4(ip)) => ip.is_loopback(),
            Some(Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if !literal_loopback
            || !loopback
            || !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(error(
                "endpoint must be a literal loopback HTTP(S) origin without credentials",
            ));
        }
        let profile = &self.profile;
        let digest = profile
            .model_digest
            .strip_prefix("sha256:")
            .unwrap_or(&profile.model_digest);
        if profile.model.trim().is_empty()
            || profile.model.len() > 256
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
            || profile.dimensions == 0
            || profile.dimensions > MAX_DIMENSIONS
            || profile.document_prefix.len() > 4096
            || profile.query_prefix.len() > 4096
            || self.timeout_ms == 0
            || self.timeout_ms > 60_000
        {
            return Err(error(
                "invalid pinned model/digest/dimension/prefix/timeout profile",
            ));
        }
        Ok(())
    }
}

pub struct OllamaEmbeddingAdapter {
    config: LocalEmbeddingConfig,
    client: Client,
    origin: Url,
}
impl OllamaEmbeddingAdapter {
    pub fn new(config: LocalEmbeddingConfig) -> Result<Self, AppError> {
        config.validate_enabled()?;
        let origin = Url::parse(&config.endpoint).map_err(|_| error("invalid endpoint"))?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| error("HTTP client initialization failed"))?;
        Ok(Self {
            config,
            client,
            origin,
        })
    }
    async fn read_json<T: serde::de::DeserializeOwned>(
        &self,
        mut response: Response,
    ) -> Result<T, AppError> {
        if !response.status().is_success() {
            return Err(error("service returned non-success status"));
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(error("response exceeds limit"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| error("response read failed"))?
        {
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(error("response exceeds limit"));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| error("invalid service response"))
    }
    async fn check_model(&self) -> Result<(), AppError> {
        #[derive(Deserialize)]
        struct Model {
            name: String,
            digest: String,
        }
        #[derive(Deserialize)]
        struct Tags {
            models: Vec<Model>,
        }
        let response = self
            .client
            .get(
                self.origin
                    .join("api/tags")
                    .map_err(|_| error("invalid endpoint"))?,
            )
            .send()
            .await
            .map_err(|_| error("model identity request failed"))?;
        let tags: Tags = self.read_json(response).await?;
        let normalized = |digest: &str| {
            digest
                .strip_prefix("sha256:")
                .unwrap_or(digest)
                .to_ascii_lowercase()
        };
        let matching: Vec<_> = tags
            .models
            .iter()
            .filter(|m| m.name == self.config.profile.model)
            .collect();
        if matching.len() != 1
            || normalized(&matching[0].digest) != normalized(&self.config.profile.model_digest)
        {
            return Err(error("model digest mismatch or model unavailable"));
        }
        Ok(())
    }
}
#[async_trait]
impl EmbeddingPort for OllamaEmbeddingAdapter {
    fn profile(&self) -> &EmbeddingProfile {
        &self.config.profile
    }
    async fn embed(
        &self,
        namespace: &Namespace,
        purpose: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        let deadline = Instant::now() + Duration::from_millis(self.config.timeout_ms);
        timeout_at(deadline, async {
            if !self
                .config
                .namespace_allowlist
                .iter()
                .any(|n| n == namespace.as_str())
            {
                return Err(error("namespace not allowed"));
            }
            if texts.is_empty() || texts.len() > MAX_BATCH {
                return Err(error("batch size outside limits"));
            }
            let prefix = match purpose {
                EmbeddingPurpose::Document => &self.config.profile.document_prefix,
                EmbeddingPurpose::Query => &self.config.profile.query_prefix,
            };
            let bytes = texts
                .iter()
                .try_fold(0usize, |n, text| {
                    n.checked_add(text.len())?.checked_add(prefix.len())
                })
                .ok_or_else(|| error("input exceeds limit"))?;
            if bytes > MAX_TEXT_BYTES {
                return Err(error("input exceeds limit"));
            }
            #[derive(Serialize)]
            struct Request<'a> {
                model: &'a str,
                input: Vec<String>,
                truncate: bool,
                dimensions: usize,
            }
            let request = Request {
                model: &self.config.profile.model,
                input: texts.iter().map(|text| format!("{prefix}{text}")).collect(),
                truncate: false,
                dimensions: self.config.profile.dimensions,
            };
            let body = serde_json::to_vec(&request).map_err(|_| error("serialization failed"))?;
            if Instant::now() >= deadline {
                return Err(error("total deadline exceeded"));
            }
            self.check_model().await?;
            let response = self
                .client
                .post(
                    self.origin
                        .join("api/embed")
                        .map_err(|_| error("invalid endpoint"))?,
                )
                .header("content-type", "application/json")
                .body(body)
                .send()
                .await
                .map_err(|_| error("embedding request failed"))?;
            #[derive(Deserialize)]
            struct Embeddings {
                model: String,
                embeddings: Vec<Vec<f32>>,
            }
            let result: Embeddings = self.read_json(response).await?;
            if result.model != self.config.profile.model || result.embeddings.len() != texts.len() {
                return Err(error("response model or vector count mismatch"));
            }
            for vector in &result.embeddings {
                if vector.len() != self.config.profile.dimensions
                    || vector.iter().any(|v| !v.is_finite())
                    || vector.iter().all(|v| *v == 0.0)
                {
                    return Err(error("invalid vector dimensions or values"));
                }
            }
            self.check_model().await?;
            if Instant::now() >= deadline {
                return Err(error("total deadline exceeded"));
            }
            Ok(result.embeddings)
        })
        .await
        .map_err(|_| error("total deadline exceeded"))?
    }
}
