//! Record trigger outcomes and commit approved revisions through run_reflection.
use super::{
    AUTO_REFLECTION_COOLDOWN_HOURS, AutoReflectResult, candidate::TriggerCandidate,
    policy::ValidatedSelfRevision,
};
use crate::{
    application::run_reflection::{self, ReflectionInput},
    domain::{reflection::Reflection, self_revision::SelfRevisionProposal},
    error::AppError,
    ports::{
        Clock, EventStore, IdGenerator, ReflectionTransactionRunner, StoredTriggerLedgerEntry,
        TriggerLedgerStatus, TriggerLedgerStore,
    },
};
use chrono::{Duration, Utc};

/// 统一收口治理校验失败的拒绝语义：先按既有规则落一条 Rejected ledger 记录
/// （保持 reflection_id 为 None 的现有约定），再把携带原始可解释原因的 error 原样向上传播。
/// 三条治理校验失败路径共用这一个出口，避免重复并保证失败语义一致。
pub(super) async fn record_rejection_then_propagate<D>(
    deps: &D,
    candidate: &TriggerCandidate,
    error: AppError,
) -> Result<AutoReflectResult, AppError>
where
    D: TriggerLedgerStore + Clock + IdGenerator + Sync,
{
    record_rejected_trigger(deps, candidate, None).await?;
    Err(error)
}

pub(super) async fn apply_validated_self_revision<D>(
    deps: &D,
    candidate: &TriggerCandidate,
    proposal: &SelfRevisionProposal,
    governed_evidence_event_ids: &[String],
    validated: ValidatedSelfRevision,
) -> Result<AutoReflectResult, AppError>
where
    D: EventStore + ReflectionTransactionRunner + Clock + IdGenerator + Sync,
{
    let handled_trigger_ledger_entry = build_trigger_entry(
        deps,
        candidate,
        &candidate.evidence_event_ids,
        TriggerLedgerStatus::Handled,
        None,
        None,
    )
    .await?;
    let reflection_input = ReflectionInput::record_only(
        Reflection::new(proposal.rationale.clone()),
        governed_evidence_event_ids.to_vec(),
    )
    .with_optional_replacement_evidence_query(None)
    .with_handled_trigger_ledger_entry(handled_trigger_ledger_entry.clone());
    let reflection_input = if let Some(identity_claims) = validated.identity_claims {
        reflection_input.with_identity_update(identity_claims)
    } else {
        reflection_input
    };
    let reflection_input = if let Some(commitments) = validated.commitments {
        reflection_input.with_commitment_updates(commitments)
    } else {
        reflection_input
    };
    let reflection = run_reflection::execute(deps, reflection_input).await?;

    Ok(AutoReflectResult::handled(
        candidate,
        governed_evidence_event_ids.to_vec(),
        reflection.reflection_id,
        handled_trigger_ledger_entry.cooldown_until,
    ))
}

pub(super) async fn record_suppressed_trigger<D>(
    deps: &D,
    candidate: &TriggerCandidate,
) -> Result<StoredTriggerLedgerEntry, AppError>
where
    D: TriggerLedgerStore + Clock + IdGenerator + Sync,
{
    let preserved_reflection_id =
        latest_suppression_reflection_id(deps, &candidate.trigger_key).await?;
    let preserved_entry = latest_live_suppression_entry(deps, &candidate.trigger_key).await?;
    record_trigger_entry(
        deps,
        candidate,
        TriggerLedgerStatus::Suppressed,
        preserved_reflection_id,
        preserved_entry.and_then(|entry| entry.cooldown_until),
    )
    .await
}

pub(super) async fn record_rejected_trigger<D>(
    deps: &D,
    candidate: &TriggerCandidate,
    reflection_id: Option<String>,
) -> Result<StoredTriggerLedgerEntry, AppError>
where
    D: TriggerLedgerStore + Clock + IdGenerator + Sync,
{
    record_trigger_entry(
        deps,
        candidate,
        TriggerLedgerStatus::Rejected,
        reflection_id,
        None,
    )
    .await
}

async fn build_trigger_entry<D>(
    deps: &D,
    candidate: &TriggerCandidate,
    evidence_event_ids: &[String],
    status: TriggerLedgerStatus,
    reflection_id: Option<String>,
    cooldown_until_override: Option<chrono::DateTime<Utc>>,
) -> Result<StoredTriggerLedgerEntry, AppError>
where
    D: Clock + IdGenerator + Sync,
{
    let now = deps.now().await?;
    let handled_at = (status == TriggerLedgerStatus::Handled).then_some(now);
    let cooldown_until = cooldown_until_override.or(match status {
        TriggerLedgerStatus::Handled | TriggerLedgerStatus::Suppressed => {
            Some(now + Duration::hours(AUTO_REFLECTION_COOLDOWN_HOURS))
        }
        TriggerLedgerStatus::Pending | TriggerLedgerStatus::Rejected => None,
    });

    Ok(StoredTriggerLedgerEntry {
        ledger_id: deps.next_id().await?,
        trigger_type: candidate.trigger_type,
        namespace: candidate.namespace.clone(),
        trigger_key: candidate.trigger_key.clone(),
        status,
        evidence_window: evidence_event_ids.to_vec(),
        handled_at,
        cooldown_until,
        episode_watermark: candidate.episode_watermark,
        reflection_id,
    })
}

async fn record_trigger_entry<D>(
    deps: &D,
    candidate: &TriggerCandidate,
    status: TriggerLedgerStatus,
    reflection_id: Option<String>,
    cooldown_until_override: Option<chrono::DateTime<Utc>>,
) -> Result<StoredTriggerLedgerEntry, AppError>
where
    D: TriggerLedgerStore + Clock + IdGenerator + Sync,
{
    let entry = build_trigger_entry(
        deps,
        candidate,
        &candidate.evidence_event_ids,
        status,
        reflection_id,
        cooldown_until_override,
    )
    .await?;
    deps.record_trigger_attempt(entry.clone()).await?;
    Ok(entry)
}

async fn latest_live_suppression_entry<D>(
    deps: &D,
    trigger_key: &str,
) -> Result<Option<StoredTriggerLedgerEntry>, AppError>
where
    D: TriggerLedgerStore + Clock + Sync,
{
    let Some(latest) = deps.latest_trigger_entry(trigger_key).await? else {
        return Ok(None);
    };
    let now = deps.now().await?;

    Ok(matches!(
        latest.status,
        TriggerLedgerStatus::Handled | TriggerLedgerStatus::Suppressed
    )
    .then_some(latest)
    .filter(|entry| {
        entry
            .cooldown_until
            .is_some_and(|cooldown_until| cooldown_until > now)
    }))
}

async fn latest_suppression_reflection_id<D>(
    deps: &D,
    trigger_key: &str,
) -> Result<Option<String>, AppError>
where
    D: TriggerLedgerStore + Sync,
{
    let latest_reflection_id = deps
        .latest_trigger_entry(trigger_key)
        .await?
        .filter(|entry| {
            matches!(
                entry.status,
                TriggerLedgerStatus::Handled | TriggerLedgerStatus::Suppressed
            )
        })
        .and_then(|entry| entry.reflection_id);

    if latest_reflection_id.is_some() {
        return Ok(latest_reflection_id);
    }

    Ok(deps
        .latest_handled_trigger_entry(trigger_key)
        .await?
        .and_then(|entry| entry.reflection_id))
}
