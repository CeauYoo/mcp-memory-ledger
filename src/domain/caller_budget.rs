//! Cooperative, caller-owned attempt budgets. No durable quota or autonomous loop.
//!
//! Counts are supplied by the caller, not inferred from unrelated requests. A decision
//! admits at most one requested operation; it does not authorize writes or validate evidence.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

pub const MAX_CALLER_OPERATION_COUNT: u32 = 1_000;
pub const CALLER_BUDGET_CONTRACT: &str = "caller_operation_budget_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallerOperationCounts {
    #[schemars(range(min = 0, max = 1000))]
    pub retrievals: u32,
    #[schemars(range(min = 0, max = 1000))]
    pub reflections: u32,
    #[schemars(range(min = 0, max = 1000))]
    pub retries: u32,
}

impl CallerOperationCounts {
    fn validate(self, field: &str) -> Result<(), AppError> {
        if [self.retrievals, self.reflections, self.retries]
            .into_iter()
            .any(|value| value > MAX_CALLER_OPERATION_COUNT)
        {
            return Err(AppError::InvalidParams(format!(
                "caller_budget.{field} counts must be integers in 0..={MAX_CALLER_OPERATION_COUNT}"
            )));
        }
        Ok(())
    }

    fn remaining(self, used: Self) -> Self {
        Self {
            retrievals: self.retrievals.saturating_sub(used.retrievals),
            reflections: self.reflections.saturating_sub(used.reflections),
            retries: self.retries.saturating_sub(used.retries),
        }
    }
}

/// A caller report only. `New` never bypasses the operation's evidence validation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallerEvidenceSignal {
    #[default]
    Unknown,
    New,
    Unchanged,
    Insufficient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallerBudget {
    pub limits: CallerOperationCounts,
    /// Attempts already admitted, before this call. Required to avoid an implicit reset.
    pub used: CallerOperationCounts,
    /// A retry consumes both its operation count and one retry count.
    #[serde(default)]
    pub is_retry: bool,
    #[serde(default)]
    pub evidence: CallerEvidenceSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallerOperation {
    Retrieval,
    Reflection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CallerStopReason {
    NoNewEvidence,
    InsufficientEvidence,
    RetryBudgetExhausted,
    RetrievalBudgetExhausted,
    ReflectionBudgetExhausted,
}

impl CallerStopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoNewEvidence => "no_new_evidence",
            Self::InsufficientEvidence => "insufficient_evidence",
            Self::RetryBudgetExhausted => "retry_budget_exhausted",
            Self::RetrievalBudgetExhausted => "retrieval_budget_exhausted",
            Self::ReflectionBudgetExhausted => "reflection_budget_exhausted",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CallerBudgetDecision {
    pub contract: &'static str,
    pub operation: CallerOperation,
    /// Admission only, not a claim that the operation succeeded or its evidence is true.
    pub allowed: bool,
    pub stop_reason: Option<CallerStopReason>,
    /// Use these counts on the next call, including after an admitted operation fails.
    pub next_used: CallerOperationCounts,
    pub remaining: CallerOperationCounts,
}

impl CallerBudget {
    /// Deterministic precedence: explicit reflection evidence stop, retry limit, operation
    /// limit. Retrieval may reread unchanged evidence or look for missing evidence.
    pub fn evaluate(self, operation: CallerOperation) -> Result<CallerBudgetDecision, AppError> {
        self.limits.validate("limits")?;
        self.used.validate("used")?;
        let stop_reason = if operation == CallerOperation::Reflection
            && self.evidence == CallerEvidenceSignal::Unchanged
        {
            Some(CallerStopReason::NoNewEvidence)
        } else if operation == CallerOperation::Reflection
            && self.evidence == CallerEvidenceSignal::Insufficient
        {
            Some(CallerStopReason::InsufficientEvidence)
        } else if self.is_retry && self.used.retries >= self.limits.retries {
            Some(CallerStopReason::RetryBudgetExhausted)
        } else {
            match operation {
                CallerOperation::Retrieval if self.used.retrievals >= self.limits.retrievals => {
                    Some(CallerStopReason::RetrievalBudgetExhausted)
                }
                CallerOperation::Reflection if self.used.reflections >= self.limits.reflections => {
                    Some(CallerStopReason::ReflectionBudgetExhausted)
                }
                _ => None,
            }
        };
        let mut next_used = self.used;
        if stop_reason.is_none() {
            // Admission guarantees each increment is strictly below its validated limit.
            match operation {
                CallerOperation::Retrieval => next_used.retrievals += 1,
                CallerOperation::Reflection => next_used.reflections += 1,
            }
            if self.is_retry {
                next_used.retries += 1;
            }
        }
        Ok(CallerBudgetDecision {
            contract: CALLER_BUDGET_CONTRACT,
            operation,
            allowed: stop_reason.is_none(),
            stop_reason,
            next_used,
            remaining: self.limits.remaining(next_used),
        })
    }
}
