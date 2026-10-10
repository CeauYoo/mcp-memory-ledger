//! Pure provider-specific JSON contracts; no network or environment access.
pub(super) mod anthropic;
pub(super) mod chat_completions;
pub(super) mod responses;

use crate::error::AppError;
pub(super) fn invalid(provider: &str, reason: &str) -> AppError {
    AppError::Message(format!("{provider} response {reason}"))
}
