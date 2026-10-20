//! Caller-declared validity metadata, never inferred from recording timestamps.
use crate::error::AppError;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Exact, typed caller declarations. These values are not authenticated host evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EnvironmentValue {
    Text(String),
    Boolean(bool),
    Integer(i64),
}
pub type EnvironmentFacts = BTreeMap<String, EnvironmentValue>;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentMatch {
    NotDeclared,
    MatchesReportedValues,
    Mismatch,
    Unknown,
}

pub fn validate_environment(values: &EnvironmentFacts) -> Result<(), AppError> {
    if values.len() > 16 || values.iter().any(|(key, value)| {
        key.is_empty() || key.len() > 64
            || !key.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || matches!(value, EnvironmentValue::Text(s) if s.is_empty() || s.len() > 256 || s.chars().any(char::is_control))
    }) { return invalid(); }
    Ok(())
}
/// All requirements are conjunctive; missing facts never count as matches.
/// A known mismatch takes precedence over missing facts, independent of key order.
pub fn match_environment(
    required: &EnvironmentFacts,
    observed: &EnvironmentFacts,
) -> Result<EnvironmentMatch, AppError> {
    validate_environment(required)?;
    validate_environment(observed)?;
    if required.is_empty() {
        return Ok(EnvironmentMatch::NotDeclared);
    }
    let mut missing = false;
    for (key, expected) in required {
        match observed.get(key) {
            Some(actual) if actual != expected => return Ok(EnvironmentMatch::Mismatch),
            None => missing = true,
            _ => {}
        }
    }
    Ok(if missing {
        EnvironmentMatch::Unknown
    } else {
        EnvironmentMatch::MatchesReportedValues
    })
}
pub const MAX_METADATA_BYTES: usize = 16384;
pub const MAX_REFERENCE_BYTES: usize = 1024;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RecallApplicability {
    pub namespace: String,
    pub reference: String,
    pub content_digest: String,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub conditions: Vec<String>,
    pub environment: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub typed_environment: EnvironmentFacts,
}
impl RecallApplicability {
    pub fn validate(&self) -> Result<(), AppError> {
        crate::domain::experience::validate_namespace(&self.namespace)?;
        validate_environment(&self.typed_environment)?;
        if self.reference.len() > MAX_REFERENCE_BYTES
            || ![
                "claim:",
                "event:",
                "experience_episode:",
                "experience_candidate:",
            ]
            .iter()
            .any(|prefix| self.reference.starts_with(prefix) && self.reference.len() > prefix.len())
            || self.reference.contains('\0')
            || self.reference.chars().any(char::is_whitespace)
            || self.reference == "claim:"
            || self.reference == "event:"
            || self.content_digest.len() != 64
            || !self
                .content_digest
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        {
            return invalid();
        }
        if self.conditions.len() > 16
            || self.environment.len() > 16
            || self
                .conditions
                .iter()
                .chain(&self.environment)
                .any(|s| s.is_empty() || s.len() > 512 || s.contains('\0'))
        {
            return invalid();
        }
        let from = parse(self.valid_from.as_deref())?;
        let until = parse(self.valid_until.as_deref())?;
        if from.zip(until).is_some_and(|(a, b)| a >= b)
            || serde_json::to_vec(self)
                .map_err(|_| AppError::Message("metadata encoding failed".into()))?
                .len()
                > MAX_METADATA_BYTES
        {
            return invalid();
        }
        Ok(())
    }
    pub fn time_state(&self, now: DateTime<Utc>) -> Result<&'static str, AppError> {
        if parse(self.valid_from.as_deref())?.is_some_and(|t| now < t) {
            return Ok("not_yet_valid");
        }
        if parse(self.valid_until.as_deref())?.is_some_and(|t| now >= t) {
            return Ok("expired");
        }
        Ok(if self.valid_from.is_none() && self.valid_until.is_none() {
            "validity_unknown"
        } else {
            "within_declared_interval"
        })
    }
}
fn parse(value: Option<&str>) -> Result<Option<DateTime<Utc>>, AppError> {
    value
        .map(|s| {
            if s.len() > 40 {
                return Err(AppError::InvalidParams("invalid validity timestamp".into()));
            }
            DateTime::parse_from_rfc3339(s)
                .map(|t| t.with_timezone(&Utc))
                .map_err(|_| AppError::InvalidParams("invalid validity timestamp".into()))
        })
        .transpose()
}
fn invalid<T>() -> Result<T, AppError> {
    Err(AppError::InvalidParams(
        "invalid bounded recall applicability declaration".into(),
    ))
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeclareRecallApplicabilityRequest {
    pub request_id: String,
    pub expected_revision: u64,
    pub declaration: RecallApplicability,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallApplicabilityRevision {
    pub revision: u64,
    pub declaration: RecallApplicability,
}

#[cfg(test)]
mod typed_environment_tests {
    use super::*;
    #[test]
    fn strict_types_missing_and_mismatch_precedence() {
        let required = [
            ("a".into(), EnvironmentValue::Boolean(true)),
            ("z".into(), EnvironmentValue::Integer(1)),
        ]
        .into();
        assert_eq!(
            match_environment(&required, &BTreeMap::new()).unwrap(),
            EnvironmentMatch::Unknown
        );
        let observed = [("z".into(), EnvironmentValue::Text("1".into()))].into();
        assert_eq!(
            match_environment(&required, &observed).unwrap(),
            EnvironmentMatch::Mismatch
        );
        assert_eq!(
            match_environment(&BTreeMap::new(), &observed).unwrap(),
            EnvironmentMatch::NotDeclared
        );
    }
    #[test]
    fn bounds_and_unsupported_values_fail_closed() {
        for key in ["", "os name", "os\0", "系统"] {
            assert!(
                validate_environment(&[(key.into(), EnvironmentValue::Boolean(true))].into())
                    .is_err()
            );
        }
        assert!(
            validate_environment(&[("os".into(), EnvironmentValue::Text("x".repeat(257)))].into())
                .is_err()
        );
        assert!(
            validate_environment(
                &(0..17)
                    .map(|i| (format!("k{i}"), EnvironmentValue::Boolean(true)))
                    .collect()
            )
            .is_err()
        );
        for json in ["null", "[]", "{}", "1.5"] {
            assert!(serde_json::from_str::<EnvironmentValue>(json).is_err());
        }
    }
    #[test]
    fn legacy_declarations_roundtrip_without_new_field() {
        let value = serde_json::json!({"namespace":"project/a", "reference":"event:a", "content_digest":"a".repeat(64), "valid_from":null, "valid_until":null, "conditions":[], "environment":[]});
        let declaration: RecallApplicability = serde_json::from_value(value.clone()).unwrap();
        declaration.validate().unwrap();
        assert_eq!(serde_json::to_value(declaration).unwrap(), value);
    }
}
