//! Normalize model evidence and constrain it to the frozen trigger window.
use crate::{
    domain::{event::EventReference, self_revision::SelfRevisionProposal},
    error::AppError,
    ports::{EventStore, EvidenceQuery},
};

/// The model proposal boundary follows the same raw-or-`event:<id>` contract as
/// explicit reflection input. Downstream store calls and diagnostic `*_event_ids`
/// stay raw for compatibility.
pub(super) fn normalize_event_ids(event_ids: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut normalized = Vec::new();
    for event_id in event_ids {
        let event_id = EventReference::parse(event_id)
            .map_err(AppError::from)?
            .event_id()
            .to_string();
        if !normalized.contains(&event_id) {
            normalized.push(event_id);
        }
    }
    Ok(normalized)
}

pub(super) async fn resolve_governed_evidence_window<D>(
    deps: &D,
    candidate_evidence_event_ids: &[String],
    proposal: &SelfRevisionProposal,
) -> Result<Vec<String>, AppError>
where
    D: EventStore + Sync,
{
    let candidate_evidence_event_ids = normalize_event_ids(candidate_evidence_event_ids.to_vec())?;
    let query_constrained_candidate_ids =
        if let Some(proposed_evidence_query) = proposal.proposed_evidence_query.clone() {
            let query_limit = proposed_evidence_query.limit;
            // 与 DTO / SQLite chokepoint 的 limit==0 早拒对称：model 提议的 evidence
            // query limit==0 必须在解析入口拒绝为 InvalidParams，不得被后置 take(0)
            // 静默收窄为空集、当成确定性空匹配掩盖（违反 C1「不得被当成空匹配掩盖」意图）。
            if query_limit == Some(0) {
                return Err(AppError::InvalidParams(
                    "proposed evidence query limit must be at least 1".to_string(),
                ));
            }
            let proposed_query_event_ids = normalize_event_ids(
                deps.query_evidence_event_ids_unbounded(EvidenceQuery {
                    namespace: proposed_evidence_query.namespace,
                    owner: proposed_evidence_query.owner,
                    kind: proposed_evidence_query.kind,
                    limit: None,
                    recorded_after: proposed_evidence_query.recorded_after,
                    recorded_before: proposed_evidence_query.recorded_before,
                    event_id_prefix: proposed_evidence_query.event_id_prefix,
                })
                .await?,
            );
            let proposed_query_event_ids = proposed_query_event_ids?;
            let filtered_candidate_ids = candidate_evidence_event_ids
                .iter()
                .filter(|event_id| proposed_query_event_ids.contains(event_id))
                .cloned()
                .collect::<Vec<_>>();

            Some((filtered_candidate_ids, query_limit))
        } else {
            None
        };

    if proposal.proposed_evidence_event_ids.is_empty() {
        let Some((filtered_candidate_ids, query_limit)) = query_constrained_candidate_ids else {
            return Ok(candidate_evidence_event_ids);
        };

        if filtered_candidate_ids.is_empty() {
            return Err(AppError::InvalidParams(
                "proposed evidence query did not match the current trigger window".to_string(),
            ));
        }

        let governed_evidence_event_ids = if let Some(limit) = query_limit {
            filtered_candidate_ids.into_iter().take(limit).collect()
        } else {
            filtered_candidate_ids
        };
        return Ok(governed_evidence_event_ids);
    }

    let proposed_evidence_event_ids =
        normalize_event_ids(proposal.proposed_evidence_event_ids.clone())?;
    if proposed_evidence_event_ids
        .iter()
        .any(|event_id| !candidate_evidence_event_ids.contains(event_id))
    {
        return Err(AppError::InvalidParams(
            "model proposed evidence outside the current trigger window".to_string(),
        ));
    }

    if let Some((eligible_query_event_ids, _)) = query_constrained_candidate_ids
        && proposed_evidence_event_ids
            .iter()
            .any(|event_id| !eligible_query_event_ids.contains(event_id))
    {
        return Err(AppError::InvalidParams(
            "model proposed evidence ids do not satisfy the proposed evidence query within the current trigger window".to_string(),
        ));
    }

    Ok(proposed_evidence_event_ids)
}
