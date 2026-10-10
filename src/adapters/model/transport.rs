//! The only HTTP effect boundary. A generation is sent once, with no implicit retry.
use crate::error::AppError;
use reqwest::{
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;

#[derive(Clone)]
pub(super) struct Transport {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    provider: &'static str,
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Transport")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

pub(super) enum Authentication {
    Bearer,
    Anthropic,
}

impl Transport {
    pub fn new(
        base_url: &str,
        path: &str,
        api_key: &str,
        timeout_ms: u64,
        provider: &'static str,
        authentication: Authentication,
    ) -> Result<Self, AppError> {
        let invalid = || AppError::Message(format!("{provider} invalid HTTP configuration"));
        let mut endpoint = reqwest::Url::parse(base_url).map_err(|_| invalid())?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || timeout_ms == 0
            || api_key.trim().is_empty()
        {
            return Err(invalid());
        }
        endpoint.set_path(&format!("{}{path}", endpoint.path().trim_end_matches('/')));
        let mut headers = HeaderMap::new();
        let (name, value) = match authentication {
            Authentication::Bearer => (AUTHORIZATION, format!("Bearer {api_key}")),
            Authentication::Anthropic => {
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
                (
                    reqwest::header::HeaderName::from_static("x-api-key"),
                    api_key.to_owned(),
                )
            }
        };
        let mut value = HeaderValue::from_str(&value).map_err(|_| invalid())?;
        value.set_sensitive(true);
        headers.insert(name, value);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .default_headers(headers)
            .build()
            .map_err(|_| invalid())?;
        Ok(Self {
            client,
            endpoint,
            provider,
        })
    }

    pub async fn send(&self, payload: Value) -> Result<Value, AppError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .json(&payload)
            .send()
            .await
            .map_err(|error| self.network_error(error.is_timeout()))?;
        if !response.status().is_success() {
            // Provider bodies, request IDs and URL paths can contain credentials or prompt data.
            return Err(AppError::Message(format!(
                "{} request failed with status {}",
                self.provider,
                response.status().as_u16()
            )));
        }
        response.json().await.map_err(|error| {
            if error.is_timeout() {
                self.network_error(true)
            } else {
                AppError::Message(format!("{} response could not be parsed", self.provider))
            }
        })
    }

    fn network_error(&self, timeout: bool) -> AppError {
        AppError::Message(format!(
            "{} network error: {}",
            self.provider,
            if timeout {
                "request timed out"
            } else {
                "request failed"
            }
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_configuration_without_echoing_secrets() {
        for (url, key, timeout) in [
            ("https://user:secret-password@example.com/v1", "key", 1000),
            ("https://example.com/v1?token=secret-query", "key", 1000),
            ("https://example.com/v1#secret-fragment", "key", 1000),
            ("file:///secret-file", "key", 1000),
            ("https://example.com/v1", "secret-header\r\nextra", 1000),
            ("https://example.com/v1", "", 1000),
            ("https://example.com/v1", "key", 0),
        ] {
            let error = Transport::new(
                url,
                "/messages",
                key,
                timeout,
                "anthropic",
                Authentication::Anthropic,
            )
            .unwrap_err();
            assert_eq!(error.to_string(), "anthropic invalid HTTP configuration");
        }
    }
}
