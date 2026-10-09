//! Freeze the trigger window, narrow it to scope, and build the proposal snapshot.
use super::{
    AutoReflectInput, DEFAULT_SNAPSHOT_BUDGET, FAILURE_TRIGGER_THRESHOLD,
    evidence::normalize_event_ids,
};
use crate::{
    application::build_self_snapshot::{self, BuildSelfSnapshotInput},
    domain::{
        event::EventReference,
        self_revision::TriggerType,
        snapshot::SnapshotTimeWindow,
        types::{MemoryScope, Namespace},
    },
    error::AppError,
    ports::{ClaimStore, CommitmentStore, EpisodeStore, EventStore, EvidenceQuery, IdentityStore},
};

#[derive(Debug, Clone)]
pub(super) struct TriggerCandidate {
    pub(super) trigger_type: TriggerType,
    pub(super) namespace: Namespace,
    pub(super) scope: MemoryScope,
    pub(super) trigger_hints: Vec<String>,
    pub(super) trigger_key: String,
    pub(super) evidence_event_ids: Vec<String>,
    pub(super) time_window: SnapshotTimeWindow,
    pub(super) should_consider: bool,
    pub(super) episode_watermark: Option<u64>,
}

pub(super) async fn detect_trigger_candidate<D>(
    deps: &D,
    input: &AutoReflectInput,
) -> Result<TriggerCandidate, AppError>
where
    D: EventStore + EpisodeStore + Sync,
{
    let scope = MemoryScope::for_namespace(input.namespace.clone());
    // Freeze the trigger window before applying the authorized scope. Querying
    // the scope first would let older in-scope rows refill a five-row window
    // after newer out-of-scope rows were excluded.
    let trigger_window_event_ids = match input.trigger_type {
        TriggerType::Failure => {
            deps.query_evidence_event_ids(EvidenceQuery {
                namespace: None,
                owner: None,
                kind: Some(crate::domain::types::EventKind::Action),
                limit: Some(5),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            })
            .await?
        }
        TriggerType::Conflict | TriggerType::Periodic => {
            deps.query_evidence_event_ids(EvidenceQuery {
                namespace: None,
                owner: None,
                kind: None,
                limit: Some(5),
                recorded_after: None,
                recorded_before: None,
                event_id_prefix: None,
            })
            .await?
        }
    };
    let trigger_window_event_ids = normalize_event_ids(trigger_window_event_ids)?;
    let trigger_manifest = trigger_window_event_ids
        .iter()
        .cloned()
        .map(EventReference::from_event_id)
        .collect::<Vec<_>>();
    let evidence_event_ids = normalize_event_ids(
        deps.list_event_references_for_snapshot(
            &scope,
            Some(&trigger_manifest),
            &SnapshotTimeWindow::unbounded(),
        )
        .await?,
    )?;
    let evidence_manifest = evidence_event_ids
        .iter()
        .cloned()
        .map(EventReference::from_event_id)
        .collect::<Vec<_>>();
    let timestamps = deps
        .list_recorded_at_for_snapshot_manifest(&scope, &evidence_manifest)
        .await?;
    let time_window = match (timestamps.iter().min(), timestamps.iter().max()) {
        (Some(recorded_after), Some(recorded_before)) => {
            SnapshotTimeWindow::new(Some(*recorded_after), Some(*recorded_before))
                .map_err(AppError::from)?
        }
        _ => SnapshotTimeWindow::unbounded(),
    };
    let episode_watermark = dedupe_strings(
        deps.list_episode_references_for_snapshot(&scope, &time_window)
            .await?,
    )
    .len() as u64;
    let should_consider = match input.trigger_type {
        TriggerType::Failure => {
            has_any_hint(&input.trigger_hints, &["failure", "rollback"])
                && evidence_event_ids.len() >= FAILURE_TRIGGER_THRESHOLD
        }
        TriggerType::Conflict => {
            !evidence_event_ids.is_empty()
                && has_any_hint(&input.trigger_hints, &["conflict", "rollback", "identity"])
        }
        TriggerType::Periodic => !evidence_event_ids.is_empty() && episode_watermark > 0,
    };

    Ok(TriggerCandidate {
        trigger_type: input.trigger_type,
        namespace: input.namespace.clone(),
        scope,
        trigger_hints: input.trigger_hints.clone(),
        trigger_key: canonical_trigger_key(&input.namespace, input.trigger_type),
        evidence_event_ids,
        time_window,
        should_consider,
        episode_watermark: Some(episode_watermark),
    })
}

pub(super) async fn build_revision_snapshot<D>(
    deps: &D,
    candidate: &TriggerCandidate,
) -> Result<crate::domain::snapshot::SelfSnapshot, AppError>
where
    D: ClaimStore + CommitmentStore + IdentityStore + EventStore + EpisodeStore + Sync,
{
    Ok(build_self_snapshot::execute(
        deps,
        BuildSelfSnapshotInput {
            scope: candidate.scope.clone(),
            evidence_manifest: Some(
                candidate
                    .evidence_event_ids
                    .iter()
                    .cloned()
                    .map(EventReference::from_event_id)
                    .collect(),
            ),
            time_window: candidate.time_window.clone(),
            budget: crate::domain::snapshot::SnapshotBudget::new(
                candidate
                    .evidence_event_ids
                    .len()
                    .max(DEFAULT_SNAPSHOT_BUDGET),
            ),
        },
    )
    .await?
    .snapshot)
}

pub(super) fn canonical_trigger_key(namespace: &Namespace, trigger_type: TriggerType) -> String {
    format!(
        "{}:{}",
        namespace.as_str(),
        trigger_type_label(trigger_type)
    )
}

fn trigger_type_label(trigger_type: TriggerType) -> &'static str {
    match trigger_type {
        TriggerType::Conflict => "conflict",
        TriggerType::Failure => "failure",
        TriggerType::Periodic => "periodic",
    }
}

fn has_any_hint(hints: &[String], expected: &[&str]) -> bool {
    hints.iter().any(|hint| {
        expected
            .iter()
            .any(|expected_hint| hint.eq_ignore_ascii_case(expected_hint))
    })
}

pub(super) fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        if !deduped.contains(&value) {
            deduped.push(value);
        }
    }
    deduped
}
