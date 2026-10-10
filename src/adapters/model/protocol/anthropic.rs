use super::{super::prompt::Prompt, invalid};
use crate::{error::AppError, support::config::NativeModelConfig};
use serde_json::{Value, json};

pub(in crate::adapters::model) fn request(config: &NativeModelConfig, prompt: Prompt) -> Value {
    let mut body = json!({
        "model": config.model, "system": prompt.instructions,
        "messages": [{"role":"user", "content":prompt.input}],
        "max_tokens": config.max_tokens, "stream":false,
    });
    if let Some(temperature) = config.temperature {
        body["temperature"] = json!(temperature);
    }
    body
}

pub(in crate::adapters::model) fn text(body: Value) -> Result<String, AppError> {
    let bad = |reason| invalid("anthropic", reason);
    if body["type"] == "error" || body.get("error").is_some_and(|error| !error.is_null()) {
        return Err(bad("contained an API error"));
    }
    if body["stop_reason"] == "refusal" || body["stop_details"]["type"] == "refusal" {
        return Err(bad("contained a refusal"));
    }
    if body["type"] != "message"
        || body["role"] != "assistant"
        || !matches!(
            body["stop_reason"].as_str(),
            Some("end_turn" | "stop_sequence")
        )
    {
        return Err(bad("was incomplete or used unsupported output"));
    }
    let blocks = body["content"]
        .as_array()
        .ok_or_else(|| bad("has no content array"))?;
    let mut text = String::new();
    for block in blocks {
        match block["type"].as_str() {
            Some("text") => text.push_str(
                block["text"]
                    .as_str()
                    .ok_or_else(|| bad("has invalid text"))?,
            ),
            Some("thinking" | "redacted_thinking") => {}
            Some("refusal") => return Err(bad("contained a refusal")),
            _ => {
                return Err(bad(
                    "contained unsupported content (tool calls are not enabled)",
                ));
            }
        }
    }
    Ok(text)
}
