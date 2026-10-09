//! Bounded, caller-supplied provenance. These labels authenticate no producer
//! and do not establish truth, trust, or permission to take an action.
use crate::domain::{DomainError, event::EventReference};
use serde::{Deserialize, Serialize};

pub const MAX_FEEDBACK_TEXT_BYTES: usize = 4096;
pub const MAX_FEEDBACK_ITEMS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSourceKind {
    CallerReported,
    ToolReported,
    ModelAsserted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackVerificationResult {
    Passed,
    Failed,
    Inconclusive,
    NotPerformed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackMetadata {
    pub source_kind: FeedbackSourceKind,
    pub producer: String,
    pub observed_target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_version: Option<String>,
    pub expected: String,
    pub actual: String,
    pub verification_method: String,
    pub verification_result: FeedbackVerificationResult,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    #[schemars(with = "Vec<String>")]
    pub evidence_refs: Vec<EventReference>,
}

impl FeedbackMetadata {
    pub fn validate(&self) -> Result<(), DomainError> {
        let valid_text = |text: &str| {
            !text.is_empty()
                && text.trim() == text
                && text.len() <= MAX_FEEDBACK_TEXT_BYTES
                && !text
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
        };
        if [
            &self.producer,
            &self.observed_target,
            &self.expected,
            &self.actual,
            &self.verification_method,
        ]
        .into_iter()
        .any(|text| !valid_text(text))
            || self
                .observed_version
                .as_deref()
                .is_some_and(|text| !valid_text(text))
            || self.limitations.len() > MAX_FEEDBACK_ITEMS
            || self.limitations.iter().any(|text| !valid_text(text))
            || self.evidence_refs.len() > MAX_FEEDBACK_ITEMS
            || self
                .evidence_refs
                .iter()
                .any(|reference| reference.event_id().len() > MAX_FEEDBACK_TEXT_BYTES)
            || self
                .evidence_refs
                .iter()
                .enumerate()
                .any(|(i, reference)| self.evidence_refs[..i].contains(reference))
        {
            return Err(DomainError::InvalidFeedbackMetadata);
        }
        Ok(())
    }
}
