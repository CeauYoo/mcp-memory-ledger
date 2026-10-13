//! Bounded certification accounting. No credentials, prompts or response text are retained.
use std::sync::{Arc, Mutex};

use crate::error::AppError;
use serde::Serialize;
use serde_json::Value;

pub const MAX_REQUESTS: usize = 2;
pub const MAX_INPUT_BYTES: usize = 16 * 1024;
pub const MAX_OUTPUT_TOKENS: u32 = 512;

#[derive(Debug, Clone, Default)]
pub struct CertificationMeter(Arc<Mutex<Vec<RequestUsage>>>);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RequestUsage {
    pub serialized_input_bytes: usize,
    pub output_token_limit: u64,
    pub http_status: Option<u16>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

impl CertificationMeter {
    pub fn snapshot(&self) -> Vec<RequestUsage> {
        self.0.lock().expect("certification meter lock").clone()
    }

    pub(super) fn begin(&self, payload: &Value, bytes: usize) -> Result<usize, AppError> {
        let limit = payload
            .get("max_output_tokens")
            .or_else(|| payload.get("max_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mut requests = self.0.lock().expect("certification meter lock");
        if requests.len() >= MAX_REQUESTS
            || bytes > MAX_INPUT_BYTES
            || limit == 0
            || limit > u64::from(MAX_OUTPUT_TOKENS)
        {
            return Err(AppError::Message(
                "certification request budget exceeded before send".into(),
            ));
        }
        let index = requests.len();
        requests.push(RequestUsage {
            serialized_input_bytes: bytes,
            output_token_limit: limit,
            http_status: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
        Ok(index)
    }

    pub(super) fn status(&self, index: usize, status: u16) {
        self.0.lock().expect("certification meter lock")[index].http_status = Some(status);
    }

    pub(super) fn usage(&self, index: usize, body: &Value) {
        let mut requests = self.0.lock().expect("certification meter lock");
        let request = &mut requests[index];
        let usage = &body["usage"];
        request.input_tokens = usage
            .get("input_tokens")
            .or_else(|| usage.get("prompt_tokens"))
            .and_then(Value::as_u64);
        request.output_tokens = usage
            .get("output_tokens")
            .or_else(|| usage.get("completion_tokens"))
            .and_then(Value::as_u64);
        request.cache_read_input_tokens = usage["cache_read_input_tokens"]
            .as_u64()
            .or_else(|| usage["input_tokens_details"]["cached_tokens"].as_u64())
            .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_u64());
        request.cache_creation_input_tokens = usage["cache_creation_input_tokens"].as_u64();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn limits_reject_before_counting_and_clones_share_budget() {
        let meter = CertificationMeter::default();
        let payload = json!({"max_tokens":512});
        assert!(meter.begin(&payload, MAX_INPUT_BYTES + 1).is_err());
        assert!(meter.begin(&json!({"max_tokens":513}), 1).is_err());
        assert!(meter.begin(&json!({}), 1).is_err());
        assert!(meter.snapshot().is_empty());
        meter.begin(&payload, MAX_INPUT_BYTES).unwrap();
        meter.clone().begin(&payload, 1).unwrap();
        assert!(meter.begin(&payload, 1).is_err());
        assert_eq!(meter.snapshot().len(), 2);
    }

    #[test]
    fn only_unsigned_usage_is_recorded_and_missing_usage_stays_unknown() {
        let meter = CertificationMeter::default();
        meter.begin(&json!({"max_tokens":512}), 5).unwrap();
        meter.usage(0, &json!({"usage":{"input_tokens":-1,"output_tokens":"secret", "cache_read_input_tokens":3}}));
        let request = &meter.snapshot()[0];
        assert_eq!(request.input_tokens, None);
        assert_eq!(request.output_tokens, None);
        assert_eq!(request.cache_read_input_tokens, Some(3));
        assert!(!serde_json::to_string(request).unwrap().contains("secret"));
    }
}
