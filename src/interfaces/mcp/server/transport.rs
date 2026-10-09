//! Parameter decoding and structured transport results.

use anyhow::Result;
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

use crate::error::AppError;

pub(super) fn generated_mcp_correlation_id() -> String {
    format!("mcp-tool-call-{}", Uuid::new_v4())
}

pub(super) fn decode_tool_params<T: DeserializeOwned>(
    raw_params: JsonObject,
) -> Result<T, AppError> {
    serde_json::from_value(serde_json::Value::Object(raw_params)).map_err(|error| {
        AppError::InvalidParams(format!("failed to deserialize parameters: {error}"))
    })
}

pub(super) fn structured<T>(value: T) -> Result<CallToolResult, McpError>
where
    T: Serialize,
{
    let json = serde_json::to_value(value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::structured(json))
}
