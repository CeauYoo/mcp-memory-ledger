use super::{super::prompt::Prompt, invalid};
use crate::error::AppError;
use serde_json::{Value, json};

pub(in crate::adapters::model) fn request(model: &str, prompt: Prompt) -> Value {
    json!({"model":model,"temperature":0.0,"messages":[
        {"role":"system","content":prompt.instructions},
        {"role":"user","content":prompt.input}
    ]})
}

pub(in crate::adapters::model) fn text(body: Value, provider: &str) -> Result<String, AppError> {
    let bad = |reason| invalid(provider, reason);
    if body.get("error").is_some_and(|error| !error.is_null()) {
        return Err(bad("contained an API error"));
    }
    let choices = body["choices"]
        .as_array()
        .ok_or_else(|| bad("has no choices array"))?;
    let Some(choice) = choices.first() else {
        return Ok(String::new());
    };
    if choice
        .get("finish_reason")
        .is_some_and(|reason| !reason.is_null() && reason != "stop")
    {
        return Err(bad("was incomplete or used unsupported output"));
    }
    let message = &choice["message"];
    if message.get("role").is_some_and(|role| role != "assistant") {
        return Err(bad("did not contain an assistant message"));
    }
    if message
        .get("refusal")
        .is_some_and(|v| !v.is_null() && v != "")
    {
        return Err(bad("contained a refusal"));
    }
    if message
        .get("tool_calls")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
        || message.get("function_call").is_some_and(|v| !v.is_null())
    {
        return Err(bad("contained unsupported tool calls"));
    }
    match &message["content"] {
        Value::Null => Ok(String::new()),
        Value::String(text) => Ok(text.to_owned()),
        Value::Array(blocks) => {
            let mut text = String::new();
            for block in blocks {
                if block["type"] != "text" {
                    return Err(bad("contained unsupported content or a refusal"));
                }
                text.push_str(
                    block["text"]
                        .as_str()
                        .ok_or_else(|| bad("has invalid text"))?,
                );
            }
            Ok(text)
        }
        _ => Err(bad("has invalid message content")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(message: Value, finish_reason: Value) -> Value {
        json!({"choices":[{"message":message,"finish_reason":finish_reason}]})
    }

    #[test]
    fn nullable_compatibility_fields_and_ordered_text_blocks_are_accepted() {
        let body = response(
            json!({"role":"assistant","refusal":null,"tool_calls":null,
            "function_call":null,"content":[{"type":"text","text":"read_"},
                {"type":"text","text":"identity_core"}]}),
            json!("stop"),
        );
        assert_eq!(text(body, "openrouter").unwrap(), "read_identity_core");
    }

    #[test]
    fn rejects_refusal_truncation_and_unhandled_calls_even_with_text() {
        for message in [
            json!({"content":"action","refusal":"private refusal"}),
            json!({"content":"action","tool_calls":[{"type":"function"}]}),
            json!({"content":"action","tool_calls":{}}),
            json!({"content":"action","function_call":{"name":"tool"}}),
            json!({"content":[{"type":"refusal","refusal":"private"}]}),
        ] {
            assert!(text(response(message, json!("stop")), "openrouter").is_err());
        }
        for reason in ["length", "content_filter", "tool_calls", "function_call"] {
            assert!(
                text(
                    response(json!({"content":"partial"}), json!(reason)),
                    "openrouter"
                )
                .is_err()
            );
        }
    }
}
