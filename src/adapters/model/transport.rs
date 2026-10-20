//! The only HTTP effect boundary. A generation is sent once, with no implicit retry.
use super::metering::CertificationMeter;
use crate::error::AppError;
use reqwest::{
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;

/// Bound the complete provider envelope, independently of any generated text limit.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Transport {
    meter: Option<CertificationMeter>,
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
            meter: None,
            client,
            endpoint,
            provider,
        })
    }

    pub fn with_meter(mut self, meter: CertificationMeter) -> Self {
        self.meter = Some(meter);
        self
    }

    pub async fn send(&self, payload: Value) -> Result<Value, AppError> {
        let bytes = serde_json::to_vec(&payload)
            .map_err(|_| AppError::Message("could not encode model request".into()))?;
        let index = self
            .meter
            .as_ref()
            .map(|meter| meter.begin(&payload, bytes.len()))
            .transpose()?;
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(bytes)
            .send()
            .await
            .map_err(|error| self.network_error(error.is_timeout()))?;
        if let (Some(meter), Some(index)) = (&self.meter, index) {
            meter.status(index, response.status().as_u16());
        }
        if !response.status().is_success() {
            // Provider bodies, request IDs and URL paths can contain credentials or prompt data.
            return Err(AppError::Message(format!(
                "{} request failed with status {}",
                self.provider,
                response.status().as_u16()
            )));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            return Err(self.response_too_large());
        }
        // Content-Length is optional and untrusted. Check actual bytes before each
        // append, including chunked/close-delimited bodies; never buffer unbounded JSON.
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|error| {
            if error.is_timeout() {
                self.network_error(true)
            } else {
                self.response_parse_error()
            }
        })? {
            if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(bytes.len()) {
                return Err(self.response_too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        let body = serde_json::from_slice(&bytes).map_err(|_| self.response_parse_error())?;
        if let (Some(meter), Some(index)) = (&self.meter, index) {
            meter.usage(index, &body);
        }
        Ok(body)
    }

    fn response_too_large(&self) -> AppError {
        AppError::Message(format!(
            "{} response exceeded 16777216 bytes",
            self.provider
        ))
    }

    fn response_parse_error(&self) -> AppError {
        AppError::Message(format!("{} response could not be parsed", self.provider))
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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Stub {
        base: String,
        requests: Arc<AtomicUsize>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Stub {
        async fn start(headers: String, body: Vec<u8>, body_delay_ms: u64) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let requests = Arc::new(AtomicUsize::new(0));
            let count = requests.clone();
            let body = Arc::new(body);
            let task = tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    count.fetch_add(1, Ordering::SeqCst);
                    // Consume the tiny request so closing the response doesn't reset it.
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    loop {
                        let read = stream.read(&mut buffer).await.unwrap_or(0);
                        if read == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..read]);
                        if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&request[..end]);
                            let length = head
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())
                                        .flatten()
                                })
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    if stream.write_all(headers.as_bytes()).await.is_err() {
                        continue;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(body_delay_ms)).await;
                    let _ = stream.write_all(&body).await;
                    let _ = stream.shutdown().await;
                }
            });
            Self {
                base,
                requests,
                task,
            }
        }

        fn transport(&self, timeout_ms: u64) -> Transport {
            Transport::new(
                &self.base,
                "/secret-path",
                "secret-token",
                timeout_ms,
                "test-provider",
                Authentication::Bearer,
            )
            .unwrap()
        }
    }

    impl Drop for Stub {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

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

    #[tokio::test]
    async fn rejects_oversized_content_length_before_waiting_for_body() {
        let stub = Stub::start(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                MAX_RESPONSE_BYTES + 1
            ),
            Vec::new(),
            1000,
        )
        .await;
        let error = stub
            .transport(100)
            .send(serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "test-provider response exceeded 16777216 bytes"
        );
        assert_eq!(stub.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn enforces_actual_limit_with_chunked_and_close_delimited_bodies() {
        for chunked in [true, false] {
            let mut body = vec![b' '; MAX_RESPONSE_BYTES + 1];
            body[0] = b'{';
            body[1] = b'}';
            let headers = if chunked {
                let mut wire = format!("{:x}\r\n", body.len()).into_bytes();
                wire.extend_from_slice(&body);
                wire.extend_from_slice(b"\r\n0\r\n\r\n");
                body = wire;
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n"
            };
            let stub = Stub::start(headers.into(), body, 0).await;
            let error = stub
                .transport(5000)
                .send(serde_json::json!({}))
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "test-provider response exceeded 16777216 bytes"
            );
            assert_eq!(stub.requests.load(Ordering::SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn accepts_complete_json_envelope_exactly_at_limit() {
        let mut body = vec![b' '; MAX_RESPONSE_BYTES];
        body[0] = b'{';
        body[1] = b'}';
        let stub = Stub::start(
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()),
            body,
            0,
        )
        .await;
        assert_eq!(
            stub.transport(5000)
                .send(serde_json::json!({}))
                .await
                .unwrap(),
            serde_json::json!({})
        );
        assert_eq!(stub.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn malformed_json_errors_are_redacted_and_not_retried() {
        let body = b"secret-provider-body".to_vec();
        let stub = Stub::start(
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()),
            body,
            0,
        )
        .await;
        let error = stub
            .transport(1000)
            .send(serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "test-provider response could not be parsed"
        );
        assert_eq!(stub.requests.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn body_read_timeout_stays_redacted_and_is_not_retried() {
        let stub = Stub::start(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n".into(),
            b"{}".to_vec(),
            1000,
        )
        .await;
        let error = stub
            .transport(100)
            .send(serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "test-provider network error: request timed out"
        );
        assert_eq!(stub.requests.load(Ordering::SeqCst), 1);
    }
}
