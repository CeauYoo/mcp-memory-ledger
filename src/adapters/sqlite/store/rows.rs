//! Ledger codecs, timestamp keys, and database row mapping.

use chrono::{DateTime, Utc};
use sqlx::Row;

use crate::{
    domain::{
        claim::ClaimDraft,
        event::Event,
        operation_log::{ActorKind, OperationLogKind, OperationLogStatus},
        self_revision::TriggerType,
        types::{EventKind, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::{ClaimStatus, StoredClaim, StoredEvent, StoredTriggerLedgerEntry, TriggerLedgerStatus},
};

pub(super) fn utc_timestamp_sort_key(timestamp: &DateTime<Utc>) -> String {
    // Match the persisted fixed-width seconds/nanoseconds pair, including
    // Chrono's nanosecond >= 1e9 representation of an RFC3339 leap second.
    format!(
        "{:020}:{:010}",
        timestamp.timestamp() + 10_000_000_000_000_i64,
        timestamp.timestamp_subsec_nanos()
    )
}

pub(super) fn serialize_json<T>(value: &T) -> Result<String, AppError>
where
    T: serde::Serialize,
{
    serde_json::to_string(value).map_err(|error| AppError::Message(error.to_string()))
}

pub(super) fn serialize_optional_json<T>(value: &Option<T>) -> Result<Option<String>, AppError>
where
    T: serde::Serialize,
{
    value.as_ref().map(serialize_json).transpose()
}

pub(super) fn deserialize_json<T>(value: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(value).map_err(|error| AppError::Message(error.to_string()))
}

pub(super) fn serialize_episode_watermark(value: Option<u64>) -> Result<Option<i64>, AppError> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| {
                AppError::InvalidParams(format!(
                    "episode watermark {value} exceeds sqlite INTEGER range"
                ))
            })
        })
        .transpose()
}

pub(super) fn map_sqlite<T>(result: Result<T, sqlx::Error>) -> Result<T, AppError> {
    result.map_err(|error| AppError::Message(error.to_string()))
}

pub(super) fn stored_claim_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<StoredClaim, AppError> {
    let claim = ClaimDraft::new(
        parse_owner(&row.get::<String, _>("owner"))?,
        row.get::<String, _>("subject"),
        row.get::<String, _>("predicate"),
        row.get::<String, _>("object"),
        parse_mode(&row.get::<String, _>("mode"))?,
    )
    .with_namespace(parse_namespace(&row.get::<String, _>("namespace"))?);
    claim.validate_namespace_owner().map_err(|error| {
        AppError::Message(format!("invalid stored claim namespace mapping: {error:?}"))
    })?;

    Ok(StoredClaim::new(
        row.get("claim_id"),
        claim,
        parse_claim_status(&row.get::<String, _>("status"))?,
    )
    .with_temporal_metadata(
        parse_optional_timestamp(row.get::<Option<String>, _>("recorded_at"))?,
        row.get("observed_at"),
    ))
}

#[allow(dead_code)]
pub(super) fn stored_event_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<StoredEvent, AppError> {
    let mut event = Event::new_with_namespace(
        parse_owner(&row.get::<String, _>("owner"))?,
        parse_namespace(&row.get::<String, _>("namespace"))?,
        parse_event_kind(&row.get::<String, _>("kind"))?,
        row.get::<String, _>("summary"),
    )
    .map_err(|error| {
        AppError::Message(format!("invalid stored event namespace mapping: {error:?}"))
    })?;
    if let Some(json) = row.get::<Option<String>, _>("feedback_json") {
        event = event
            .with_feedback(deserialize_json(&json)?)
            .map_err(|error| {
                AppError::Message(format!("invalid stored feedback metadata: {error:?}"))
            })?;
    }
    Ok(StoredEvent::new(
        row.get("event_id"),
        parse_timestamp(&row.get::<String, _>("recorded_at"))?,
        event,
    )
    .with_observed_at(row.get("observed_at")))
}

pub(super) fn stored_trigger_ledger_entry_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<StoredTriggerLedgerEntry, AppError> {
    let evidence_window = deserialize_json(&row.get::<String, _>("evidence_window"))?;
    let handled_at = parse_optional_timestamp(row.get::<Option<String>, _>("handled_at"))?;
    let cooldown_until = parse_optional_timestamp(row.get::<Option<String>, _>("cooldown_until"))?;
    let episode_watermark = row
        .get::<Option<i64>, _>("episode_watermark")
        .map(|value| {
            u64::try_from(value).map_err(|_| {
                AppError::Message(format!("invalid negative episode watermark: {value}"))
            })
        })
        .transpose()?;

    Ok(StoredTriggerLedgerEntry {
        ledger_id: row.get("ledger_id"),
        trigger_type: parse_trigger_type(&row.get::<String, _>("trigger_type"))?,
        namespace: parse_namespace(&row.get::<String, _>("namespace"))?,
        trigger_key: row.get("trigger_key"),
        status: parse_trigger_ledger_status(&row.get::<String, _>("status"))?,
        evidence_window,
        handled_at,
        cooldown_until,
        episode_watermark,
        reflection_id: row.get("reflection_id"),
    })
}

pub(super) fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, AppError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| AppError::Message(error.to_string()))
}

fn parse_optional_timestamp(value: Option<String>) -> Result<Option<DateTime<Utc>>, AppError> {
    value.as_deref().map(parse_timestamp).transpose()
}

pub(super) fn owner_as_str(owner: Owner) -> &'static str {
    match owner {
        Owner::Self_ => "self",
        Owner::User => "user",
        Owner::World => "world",
        Owner::Unknown => "unknown",
    }
}

pub(super) fn parse_owner(value: &str) -> Result<Owner, AppError> {
    match value {
        "self" => Ok(Owner::Self_),
        "user" => Ok(Owner::User),
        "world" => Ok(Owner::World),
        "unknown" => Ok(Owner::Unknown),
        _ => Err(AppError::Message(format!("unknown owner: {value}"))),
    }
}

pub(super) fn mode_as_str(mode: Mode) -> &'static str {
    match mode {
        Mode::Observed => "observed",
        Mode::Said => "said",
        Mode::Acted => "acted",
        Mode::Inferred => "inferred",
        Mode::Draft => "draft",
    }
}

fn parse_mode(value: &str) -> Result<Mode, AppError> {
    match value {
        "observed" => Ok(Mode::Observed),
        "said" => Ok(Mode::Said),
        "acted" => Ok(Mode::Acted),
        "inferred" => Ok(Mode::Inferred),
        "draft" => Ok(Mode::Draft),
        _ => Err(AppError::Message(format!("unknown mode: {value}"))),
    }
}

fn parse_namespace(value: &str) -> Result<Namespace, AppError> {
    Namespace::parse(value).map_err(|error| {
        AppError::Message(format!("invalid stored namespace `{value}`: {error:?}"))
    })
}

pub(super) fn trigger_type_as_str(trigger_type: TriggerType) -> &'static str {
    match trigger_type {
        TriggerType::Conflict => "conflict",
        TriggerType::Failure => "failure",
        TriggerType::Periodic => "periodic",
    }
}

fn parse_trigger_type(value: &str) -> Result<TriggerType, AppError> {
    match value {
        "conflict" => Ok(TriggerType::Conflict),
        "failure" => Ok(TriggerType::Failure),
        "periodic" => Ok(TriggerType::Periodic),
        _ => Err(AppError::Message(format!("unknown trigger type: {value}"))),
    }
}

pub(super) fn event_kind_as_str(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Observation => "observation",
        EventKind::Conversation => "conversation",
        EventKind::Action => "action",
        EventKind::Reflection => "reflection",
    }
}

fn parse_event_kind(value: &str) -> Result<EventKind, AppError> {
    match value {
        "observation" => Ok(EventKind::Observation),
        "conversation" => Ok(EventKind::Conversation),
        "action" => Ok(EventKind::Action),
        "reflection" => Ok(EventKind::Reflection),
        _ => Err(AppError::Message(format!("unknown event kind: {value}"))),
    }
}

fn parse_claim_status(value: &str) -> Result<ClaimStatus, AppError> {
    match value {
        "active" => Ok(ClaimStatus::Active),
        "disputed" => Ok(ClaimStatus::Disputed),
        "superseded" => Ok(ClaimStatus::Superseded),
        _ => Err(AppError::Message(format!("unknown claim status: {value}"))),
    }
}

fn parse_trigger_ledger_status(value: &str) -> Result<TriggerLedgerStatus, AppError> {
    match value {
        "pending" => Ok(TriggerLedgerStatus::Pending),
        "handled" => Ok(TriggerLedgerStatus::Handled),
        "rejected" => Ok(TriggerLedgerStatus::Rejected),
        "suppressed" => Ok(TriggerLedgerStatus::Suppressed),
        _ => Err(AppError::Message(format!(
            "unknown trigger ledger status: {value}"
        ))),
    }
}

pub(super) fn parse_actor_kind(value: &str) -> Result<ActorKind, AppError> {
    match value {
        "system" => Ok(ActorKind::System),
        "user" => Ok(ActorKind::User),
        "model" => Ok(ActorKind::Model),
        "hook" => Ok(ActorKind::Hook),
        _ => Err(AppError::Message(format!("unknown actor kind: {value}"))),
    }
}

pub(super) fn parse_operation_log_kind(value: &str) -> Result<OperationLogKind, AppError> {
    match value {
        "startup" => Ok(OperationLogKind::Startup),
        "tool" => Ok(OperationLogKind::Tool),
        "trigger" => Ok(OperationLogKind::Trigger),
        "reflection" => Ok(OperationLogKind::Reflection),
        "decision" => Ok(OperationLogKind::Decision),
        "snapshot" => Ok(OperationLogKind::Snapshot),
        "doctor" => Ok(OperationLogKind::Doctor),
        "error" => Ok(OperationLogKind::Error),
        _ => Err(AppError::Message(format!(
            "unknown operation log kind: {value}"
        ))),
    }
}

pub(super) fn parse_operation_log_status(value: &str) -> Result<OperationLogStatus, AppError> {
    match value {
        "started" => Ok(OperationLogStatus::Started),
        "ok" => Ok(OperationLogStatus::Ok),
        "handled" => Ok(OperationLogStatus::Handled),
        "suppressed" => Ok(OperationLogStatus::Suppressed),
        "rejected" => Ok(OperationLogStatus::Rejected),
        "failed" => Ok(OperationLogStatus::Failed),
        _ => Err(AppError::Message(format!(
            "unknown operation log status: {value}"
        ))),
    }
}
