use crate::error::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Private immutable write receipt identity. Raw keys and payloads are never persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteReceiptRequest {
    pub operation_id: String,
    pub namespace: String,
    pub operation: String,
    pub request_hash: String,
}
impl WriteReceiptRequest {
    pub fn new<T: Serialize>(
        operation: &str,
        namespace: &str,
        key: &str,
        payload: &T,
    ) -> Result<Self, AppError> {
        if key.is_empty()
            || key.len() > 128
            || key.trim() != key
            || key.chars().any(char::is_control)
        {
            return Err(AppError::InvalidParams("request_id must contain 1 to 128 bytes, without surrounding whitespace or control characters".into()));
        }
        let identity = serde_json::to_vec(&(operation, namespace, key))
            .map_err(|e| AppError::Message(e.to_string()))?;
        let payload = serde_json::to_vec(payload).map_err(|e| AppError::Message(e.to_string()))?;
        Ok(Self {
            operation_id: format!("write-receipt:v1:{:x}", Sha256::digest(identity)),
            namespace: namespace.into(),
            operation: operation.into(),
            request_hash: format!("{:x}", Sha256::digest(payload)),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredWriteReceipt {
    pub request_hash: String,
    pub result_json: String,
}
impl StoredWriteReceipt {
    pub fn replay<T: serde::de::DeserializeOwned>(
        &self,
        request: &WriteReceiptRequest,
    ) -> Result<T, AppError> {
        if self.request_hash != request.request_hash {
            return Err(AppError::InvalidParams("request_id was already used with a different payload in this operation and namespace".into()));
        }
        serde_json::from_str(&self.result_json)
            .map_err(|e| AppError::Message(format!("invalid durable write receipt: {e}")))
    }
}

pub fn receipt_result<T: Serialize>(
    request: &WriteReceiptRequest,
    result: &T,
) -> Result<StoredWriteReceipt, AppError> {
    Ok(StoredWriteReceipt {
        request_hash: request.request_hash.clone(),
        result_json: serde_json::to_string(result).map_err(|e| AppError::Message(e.to_string()))?,
    })
}
