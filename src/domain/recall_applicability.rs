//! Caller-declared validity metadata, never inferred from recording timestamps.
use crate::error::AppError;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub const MAX_METADATA_BYTES: usize = 16384;
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
}
impl RecallApplicability {
    pub fn validate(&self) -> Result<(), AppError> {
        crate::domain::experience::validate_namespace(&self.namespace)?;
        if self.reference.len() > 256
            || !(self.reference.starts_with("claim:") || self.reference.starts_with("event:"))
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
