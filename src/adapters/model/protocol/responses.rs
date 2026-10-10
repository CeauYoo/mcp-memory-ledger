use super::{super::prompt::Prompt, invalid};
use crate::{error::AppError, support::config::NativeModelConfig};
use serde_json::{Value, json};

pub(in crate::adapters::model) fn request(config: &NativeModelConfig, prompt: Prompt) -> Value {
    let mut body = json!({
        "model": config.model, "instructions": prompt.instructions,
        "input": [{"role":"user", "content":[{"type":"input_text", "text":prompt.input}]}],
        "max_output_tokens": config.max_tokens, "store":false, "stream":false,
    });
    if let Some(temperature) = config.temperature {
        body["temperature"] = json!(temperature);
    }
    body
}

pub(in crate::adapters::model) fn text(body: Value) -> Result<String, AppError> {
    let bad = |reason| invalid("openai-responses", reason);
    if body.get("error").is_some_and(|error| !error.is_null()) {
        return Err(bad("contained an API error"));
    }
    if body["status"] != "completed" {
        return Err(bad("was incomplete or failed"));
    }
    let output = body["output"]
        .as_array()
        .ok_or_else(|| bad("has no output array"))?;
    let mut text = String::new();
    for item in output {
        match item["type"].as_str() {
            Some("reasoning") => {}
            Some("message") => {
                if item["role"] != "assistant" || item["status"] != "completed" {
                    return Err(bad("contained an incomplete assistant message"));
                }
                let content = item["content"]
                    .as_array()
                    .ok_or_else(|| bad("has invalid message content"))?;
                for block in content {
                    match block["type"].as_str() {
                        Some("output_text") => text.push_str(
                            block["text"]
                                .as_str()
                                .ok_or_else(|| bad("has invalid text"))?,
                        ),
                        Some("refusal") => return Err(bad("contained a refusal")),
                        _ => return Err(bad("contained unsupported content")),
                    }
                }
            }
            _ => {
                return Err(bad(
                    "contained unsupported output (tool calls are not enabled)",
                ));
            }
        }
    }
    Ok(text)
}
