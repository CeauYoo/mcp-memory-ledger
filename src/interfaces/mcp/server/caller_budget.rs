//! Transport hooks for the optional, stateless caller operation budget.
use rmcp::{
    ErrorData as McpError,
    model::{CallToolResult, JsonObject},
};
use serde::Serialize;
use serde_json::json;

use crate::domain::caller_budget::{CallerBudget, CallerBudgetDecision, CallerOperation};

pub(super) fn admit_raw(
    params: &JsonObject,
    operation: CallerOperation,
) -> Result<Option<CallerBudgetDecision>, McpError> {
    // Gate the valid envelope before unrelated DTO validation or diagnostic writes.
    // Absent and explicit null preserve the legacy route exactly.
    let budget: Option<CallerBudget> = params
        .get("caller_budget")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|error| McpError::invalid_params(format!("invalid caller_budget: {error}"), None))?
        .flatten();
    let decision = budget
        .map(|budget| budget.evaluate(operation))
        .transpose()
        .map_err(|error| McpError::invalid_params(error.to_string(), None))?;
    if let Some(decision) = &decision
        && let Some(reason) = decision.stop_reason
    {
        return Err(McpError::invalid_params(
            format!("caller operation stopped: {}", reason.as_str()),
            Some(json!({ "caller_budget": decision })),
        ));
    }
    Ok(decision)
}

/// Preserve the operation's error and attach admitted-attempt accounting if present.
pub(super) fn attach_error(
    mut error: McpError,
    decision: &Option<CallerBudgetDecision>,
) -> McpError {
    if let Some(decision) = decision {
        let mut data = match error.data.take() {
            Some(serde_json::Value::Object(data)) => data,
            Some(value) => serde_json::Map::from_iter([("operation_error_data".into(), value)]),
            None => serde_json::Map::new(),
        };
        data.insert("caller_budget".into(), json!(decision));
        error.data = Some(serde_json::Value::Object(data));
    }
    error
}

/// Context results use their own byte-accounted packing hook instead of this helper.
pub(super) fn structured<T: Serialize>(
    value: T,
    decision: Option<CallerBudgetDecision>,
) -> Result<CallToolResult, McpError> {
    let mut result =
        super::transport::structured(value).map_err(|error| attach_error(error, &decision))?;
    if let Some(decision) = decision {
        let object = result
            .structured_content
            .as_mut()
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| McpError::internal_error("expected object tool result", None))?;
        object.insert("caller_budget".into(), json!(decision));
    }
    Ok(result)
}
