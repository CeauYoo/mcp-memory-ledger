//! Pure domain-to-prompt and text-to-domain transformations, independent of HTTP.
use crate::{
    domain::self_revision::{SelfRevisionProposal, SelfRevisionRequest},
    error::AppError,
    ports::ModelDecisionRequest,
};

pub(super) struct Prompt {
    pub instructions: &'static str,
    pub input: String,
}

pub(super) fn decision(request: ModelDecisionRequest) -> Result<Prompt, AppError> {
    Ok(Prompt {
        instructions: "Return only the next action name as plain text with no explanation.",
        input: format!(
            "Task: {}\nAction: {}\nSnapshot:\n{}",
            request.task,
            request.action,
            serde_json::to_string_pretty(&request.snapshot)
                .map_err(|_| AppError::Message("could not serialize model snapshot".into()))?
        ),
    })
}

pub(super) fn self_revision(request: SelfRevisionRequest) -> Result<Prompt, AppError> {
    Ok(Prompt {
        instructions: "Return only a JSON self-revision proposal with should_reflect, rationale, machine_patch.identity_patch, machine_patch.commitment_patch, proposed_evidence_event_ids, proposed_evidence_query, and confidence.",
        input: format!(
            "Self revision request:\n{}",
            serde_json::to_string_pretty(&request).map_err(|_| AppError::Message(
                "could not serialize self-revision request".into()
            ))?
        ),
    })
}

pub(super) fn nonempty(text: String, provider: &str, purpose: &str) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AppError::Message(format!(
            "{provider} response contained an empty {purpose}"
        )));
    }
    Ok(text.to_owned())
}

pub(super) fn proposal(text: String, provider: &str) -> Result<SelfRevisionProposal, AppError> {
    let text = nonempty(text, provider, "self-revision proposal")?;
    let start = text.find('{').ok_or_else(|| {
        AppError::Message(format!(
            "{provider} self-revision proposal did not contain a JSON object"
        ))
    })?;
    let end = text.rfind('}').filter(|end| *end > start).ok_or_else(|| {
        AppError::Message(format!(
            "{provider} self-revision proposal did not contain a JSON object"
        ))
    })?;
    // Do not echo serde's diagnostics: unknown enum values may contain private model text.
    serde_json::from_str(&text[start..=end]).map_err(|_| {
        AppError::Message(format!(
            "{provider} self-revision proposal could not be parsed"
        ))
    })
}
