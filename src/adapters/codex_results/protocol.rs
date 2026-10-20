//! Pinned public hook input. Raw tool values are transient and never serialized by this adapter.
use crate::adapters::codex_context::protocol::digest;
use anyhow::{Result, ensure};
use serde::{
    Deserialize,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value, json};
use std::{fmt, path::PathBuf};
pub const MAX_INPUT: usize = 2 * 1024 * 1024;
pub const MAX_RECORD: usize = 16 * 1024;

pub fn token(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s.len() <= 256
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(&c)),
        "invalid metadata identifier"
    );
    Ok(())
}
// Reject duplicate object keys at EVERY level rather than accepting last-key-wins.
struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: de::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("bounded JSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Unique(Value::Number(n)))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> std::result::Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut v = Vec::new();
                while let Some(x) = a.next_element::<Unique>()? {
                    if v.len() >= 4096 {
                        return Err(de::Error::custom("too many entries"));
                    }
                    v.push(x.0)
                }
                Ok(Unique(Value::Array(v)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Unique, A::Error> {
                let mut v = Map::new();
                while let Some(k) = a.next_key::<String>()? {
                    if v.len() >= 4096 || v.contains_key(&k) {
                        return Err(de::Error::custom("duplicate or excess entries"));
                    }
                    v.insert(k, a.next_value::<Unique>()?.0);
                }
                Ok(Unique(Value::Object(v)))
            }
        }
        d.deserialize_any(V)
    }
}
fn depth(v: &Value, n: usize) -> bool {
    n <= 32
        && match v {
            Value::Array(a) => a.iter().all(|v| depth(v, n + 1)),
            Value::Object(a) => a.values().all(|v| depth(v, n + 1)),
            _ => true,
        }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub session_id: String,
    pub turn_id: String,
    pub cwd: PathBuf,
    pub hook_event_name: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: Value,
    #[serde(default)]
    pub tool_response: Option<Value>,
    // Accepted to match the pinned schema, NEVER read or persisted.
    #[serde(rename = "transcript_path")]
    pub _transcript_path: Option<String>,
    #[serde(rename = "model")]
    pub _model: String,
    #[serde(rename = "permission_mode")]
    pub _permission_mode: String,
    #[serde(rename = "agent_id")]
    pub _agent_id: Option<String>,
    #[serde(rename = "agent_type")]
    pub _agent_type: Option<String>,
}
impl Input {
    pub fn parse(bytes: &[u8], event: &str) -> Result<Self> {
        ensure!(bytes.len() <= MAX_INPUT, "input too large");
        let Unique(v) =
            serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("invalid hook JSON"))?;
        ensure!(depth(&v, 0), "input too deep");
        ensure!(v.get("transcript_path").is_some(), "missing public field");
        if event == "PostToolUse" {
            ensure!(v.get("tool_response").is_some(), "missing response");
        } else {
            ensure!(
                event == "PreToolUse" && v.get("tool_response").is_none(),
                "unsupported pre-tool fields"
            );
        }
        let mut r: Self = serde_json::from_value(v.clone())
            .map_err(|_| anyhow::anyhow!("invalid hook contract"))?;
        // Option<Value> distinguishes neither absent nor explicit null; retain null output.
        if event == "PostToolUse" {
            r.tool_response = Some(v["tool_response"].clone());
        }
        ensure!(
            r.hook_event_name == event && r.cwd.is_absolute(),
            "wrong event or cwd"
        );
        ensure!(
            matches!(
                r._permission_mode.as_str(),
                "default" | "acceptEdits" | "plan" | "dontAsk" | "bypassPermissions"
            ),
            "unsupported permission-mode wire value"
        );
        for s in [&r.session_id, &r.turn_id, &r.tool_name, &r.tool_use_id] {
            token(s)?;
        }
        Ok(r)
    }
    pub fn input_digest(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(
            &json!({"tool":self.tool_name,"cwd":self.cwd,"input":self.tool_input}),
        )?))
    }
    pub fn response_digest(&self) -> Result<String> {
        Ok(digest(&serde_json::to_vec(&self.tool_response)?))
    }
    pub fn response_bytes(&self) -> Result<usize> {
        Ok(serde_json::to_vec(&self.tool_response)?.len())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn input() -> Value {
        json!({"session_id":"s","turn_id":"t","cwd":if cfg!(windows) {"C:\\fixture"} else {"/tmp"},"hook_event_name":"PostToolUse","tool_name":"Bash","tool_use_id":"call","tool_input":{},"tool_response":null,"transcript_path":null,"model":"fixture","permission_mode":"default"})
    }
    #[test]
    fn contract_and_null() {
        let v = input();
        assert!(
            Input::parse(&serde_json::to_vec(&v).unwrap(), "PostToolUse")
                .unwrap()
                .tool_response
                .unwrap()
                .is_null()
        );
        assert!(Input::parse(&serde_json::to_vec(&v).unwrap(), "PreToolUse").is_err());
    }
    #[test]
    fn duplicate_depth_size() {
        let valid = serde_json::to_string(&input()).unwrap();
        let duplicate = valid.replace(
            "\"tool_response\":null",
            "\"tool_response\":{\"key\":1,\"key\":2}",
        );
        assert_ne!(valid, duplicate);
        assert!(Input::parse(duplicate.as_bytes(), "PostToolUse").is_err());
        let duplicate = valid.replace(
            "\"session_id\":\"s\"",
            "\"session_id\":\"s\",\"session_id\":\"other\"",
        );
        assert!(Input::parse(duplicate.as_bytes(), "PostToolUse").is_err());
        let mut v = input();
        let mut deep = json!(0);
        for _ in 0..34 {
            deep = json!([deep]);
        }
        v["tool_response"] = deep;
        assert!(Input::parse(&serde_json::to_vec(&v).unwrap(), "PostToolUse").is_err());
        assert!(Input::parse(&vec![b' '; MAX_INPUT + 1], "PostToolUse").is_err());
    }
}
