use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::{
        ingest_interaction::{self, IngestInput, IngestResult},
        search_memory::SearchMemoryRecord,
    },
    domain::{
        claim::ClaimDraft,
        event::Event,
        feedback_candidate::claim_version,
        temporal::validate_observed_at,
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    interfaces::mcp::dto::IngestInteractionParams,
    ports::{
        ClaimReadRecord, ClaimRecordQuery, ClaimRevisionLinks, ClaimStatus, ClaimStore, Clock,
        EventRecordQuery, IdGenerator, IngestTransaction, IngestTransactionRunner, MemoryReadStore,
        StoredClaim, StoredEvent, WriteReceiptRequest,
    },
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

fn timestamp(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}

fn namespace() -> Namespace {
    Namespace::parse("project/temporal").unwrap()
}

fn event() -> Event {
    Event::new_with_namespace(
        Owner::World,
        namespace(),
        EventKind::Observation,
        "A clock-independent observation",
    )
    .unwrap()
}

fn draft() -> ClaimDraft {
    ClaimDraft::new_with_namespace(
        Owner::World,
        namespace(),
        "clock",
        "has",
        "nanosecond precision",
        Mode::Observed,
    )
}

fn input(key: &str) -> IngestInput {
    IngestInput::new(event(), vec![draft()], None).with_request_id(key.into())
}

struct Deps {
    store: SqliteStore,
    now: Mutex<DateTime<Utc>>,
    ids: AtomicU64,
    _directory: tempfile::TempDir,
}

impl Deps {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}",
            directory.path().join("temporal.db").display()
        );
        Self {
            store: SqliteStore::bootstrap(&url).await.unwrap(),
            now: Mutex::new(timestamp("2026-10-09T10:00:00.123456789Z")),
            ids: AtomicU64::new(0),
            _directory: directory,
        }
    }

    async fn records(&self) -> (Vec<StoredEvent>, Vec<StoredClaim>) {
        let scope = MemoryScope::for_namespace(namespace());
        let events = self
            .store
            .query_event_records(EventRecordQuery {
                scope: scope.clone(),
                event_reference: None,
                kind: None,
                recorded_after: None,
                recorded_before: None,
                limit: 100,
            })
            .await
            .unwrap()
            .into_iter()
            .map(|record| record.event)
            .collect();
        let claims = self
            .store
            .query_claim_records(ClaimRecordQuery {
                scope,
                claim_reference: None,
                status: None,
                mode: None,
                limit: 100,
            })
            .await
            .unwrap()
            .into_iter()
            .map(|record| record.claim)
            .collect();
        (events, claims)
    }
}

#[async_trait]
impl Clock for Deps {
    async fn now(&self) -> Result<DateTime<Utc>, AppError> {
        Ok(*self.now.lock().unwrap())
    }
}

#[async_trait]
impl IdGenerator for Deps {
    async fn next_id(&self) -> Result<String, AppError> {
        Ok(format!(
            "temporal-{}",
            self.ids.fetch_add(1, Ordering::SeqCst)
        ))
    }
}

#[async_trait]
impl IngestTransactionRunner for Deps {
    async fn begin_ingest_transaction(
        &self,
    ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError> {
        self.store.begin_ingest_transaction().await
    }
}

#[test]
fn strict_observation_times_preserve_nanoseconds_and_reject_truncation() {
    let value = "2000-01-01T05:45:00.123456789+05:45";
    assert_eq!(
        validate_observed_at(value).unwrap(),
        timestamp("2000-01-01T00:00:00.123456789Z")
    );
    for invalid in [
        "",
        "2026-10-09",
        "2026-10-09T10:00:00",
        "2026-10-09 10:00:00Z",
        " 2026-10-09T10:00:00Z",
        "2026-10-09T10:00:00Z ",
        "2026-10-09T10:00:00.1234567890Z",
        "2026-10-09T10:00:00.Z",
        "2026-10-09T10:00:00+2400",
        "2026-10-09T10:00:00+24:00",
        "2026-02-30T10:00:00Z",
        "+12026-10-09T10:00:00Z",
    ] {
        assert!(
            validate_observed_at(invalid).is_err(),
            "accepted {invalid:?}"
        );
    }
    assert!(validate_observed_at(&format!("2026-10-09T10:00:00.{}Z", "1".repeat(1000))).is_err());
}

#[test]
fn observed_at_dto_is_optional_validated_and_bounded() {
    let base = serde_json::json!({
        "event": {"owner": "World", "namespace": "project/temporal", "kind": "Observation", "summary": "observation"},
        "claim_drafts": [], "episode_reference": null
    });
    let old: IngestInteractionParams = serde_json::from_value(base.clone()).unwrap();
    assert!(IngestInput::try_from(old).is_ok());
    let mut explicit = base.clone();
    explicit["observed_at"] = "2000-01-01T00:00:00.123456789Z".into();
    let params: IngestInteractionParams = serde_json::from_value(explicit).unwrap();
    assert!(IngestInput::try_from(params).is_ok());
    let mut invalid = base;
    invalid["observed_at"] = "yesterday".into();
    let params: IngestInteractionParams = serde_json::from_value(invalid).unwrap();
    assert!(IngestInput::try_from(params).is_err());
    let schema = schemars::schema_for!(IngestInteractionParams);
    let json = serde_json::to_value(schema).unwrap();
    assert_eq!(json["properties"]["observed_at"]["maxLength"], 35);
}

#[test]
fn legacy_claim_source_exposes_unknown_times_and_old_json_still_loads() {
    let claim = StoredClaim::new("legacy".into(), draft(), ClaimStatus::Active);
    assert!(claim.recorded_at.is_none());
    assert!(claim.observed_at.is_none());
    let mut old_json = serde_json::to_value(&claim).unwrap();
    old_json.as_object_mut().unwrap().remove("recorded_at");
    old_json.as_object_mut().unwrap().remove("observed_at");
    assert_eq!(
        serde_json::from_value::<StoredClaim>(old_json).unwrap(),
        claim
    );
    let source = SearchMemoryRecord::from(ClaimReadRecord::new(
        claim,
        vec![],
        vec![],
        ClaimRevisionLinks::default(),
    ));
    let json = serde_json::to_value(source).unwrap();
    assert!(json.get("recorded_at").unwrap().is_null());
    assert!(json.get("observed_at").unwrap().is_null());
}

#[test]
fn feedback_v1_fingerprint_remains_exactly_the_original_three_fields() {
    #[derive(Serialize)]
    struct LegacyStoredClaim<'a> {
        claim_id: &'a str,
        claim: &'a ClaimDraft,
        status: ClaimStatus,
    }
    let mut claim = StoredClaim::new("legacy".into(), draft(), ClaimStatus::Active);
    let original_bytes = serde_json::to_vec(&LegacyStoredClaim {
        claim_id: &claim.claim_id,
        claim: &claim.claim,
        status: claim.status,
    })
    .unwrap();
    let expected = format!("claim-version:v1:{:x}", Sha256::digest(original_bytes));
    assert_eq!(claim_version(&claim).unwrap(), expected);
    claim.recorded_at = Some(timestamp("2026-10-09T10:00:00.123456789Z"));
    claim.observed_at = Some("1999-12-31T23:00:00-01:00".into());
    assert_eq!(claim_version(&claim).unwrap(), expected);
    claim.status = ClaimStatus::Disputed;
    assert_ne!(claim_version(&claim).unwrap(), expected);
}

#[tokio::test]
async fn ingest_uses_application_clock_and_round_trips_exact_observation_time() {
    let deps = Deps::new().await;
    let observed = "2000-01-01T05:45:00.000000009+05:45";
    let recorded = *deps.now.lock().unwrap();
    ingest_interaction::execute(
        &deps,
        input("new-record")
            .with_observed_at(observed.into())
            .unwrap(),
    )
    .await
    .unwrap();
    let (events, claims) = deps.records().await;
    assert_eq!(events.len(), 1);
    assert_eq!(claims.len(), 1);
    assert_eq!(events[0].recorded_at, recorded);
    assert_eq!(claims[0].recorded_at, Some(recorded));
    assert_eq!(events[0].observed_at.as_deref(), Some(observed));
    assert_eq!(claims[0].observed_at.as_deref(), Some(observed));
    assert_ne!(
        Some(validate_observed_at(observed).unwrap()),
        claims[0].recorded_at
    );
    let active = deps.store.list_active_claims().await.unwrap();
    assert_eq!(active[0].recorded_at, Some(recorded));
    assert_eq!(active[0].observed_at.as_deref(), Some(observed));
}

#[tokio::test]
async fn omitted_observation_is_unknown_and_keyed_replay_never_updates_timestamps() {
    let deps = Deps::new().await;
    let first = ingest_interaction::execute(&deps, input("retry"))
        .await
        .unwrap();
    let before = deps.records().await;
    assert_eq!(before.0[0].observed_at, None);
    assert_eq!(before.1[0].observed_at, None);
    assert!(before.1[0].recorded_at.is_some());
    *deps.now.lock().unwrap() = timestamp("2030-01-01T00:00:00.999999999Z");
    let replay = ingest_interaction::execute(&deps, input("retry"))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.event_id, first.event_id);
    assert_eq!(deps.records().await, before);
    assert!(
        ingest_interaction::execute(
            &deps,
            input("retry")
                .with_observed_at("1990-01-01T00:00:00Z".into())
                .unwrap(),
        )
        .await
        .is_err()
    );
    assert_eq!(deps.records().await, before);
}

#[tokio::test]
async fn changing_explicit_observation_time_conflicts_with_receipt() {
    let deps = Deps::new().await;
    let make = |value: &str| {
        input("observed-retry")
            .with_observed_at(value.into())
            .unwrap()
    };
    let first = ingest_interaction::execute(&deps, make("2000-01-01T00:00:00Z"))
        .await
        .unwrap();
    let original = deps.records().await;
    *deps.now.lock().unwrap() = timestamp("2030-01-01T00:00:00Z");
    let replay = ingest_interaction::execute(&deps, make("2000-01-01T00:00:00Z"))
        .await
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(first.event_id, replay.event_id);
    assert!(
        ingest_interaction::execute(&deps, make("2001-01-01T00:00:00Z"))
            .await
            .is_err()
    );
    assert!(
        ingest_interaction::execute(&deps, input("observed-retry"))
            .await
            .is_err()
    );
    assert_eq!(deps.records().await, original);
}

#[tokio::test]
async fn old_four_tuple_receipt_replays_without_fabricating_legacy_claim_time() {
    let deps = Deps::new().await;
    let key = "v5-key";
    let request = WriteReceiptRequest::new(
        "ingest",
        namespace().as_str(),
        key,
        &(event(), vec![draft()], None::<String>, Vec::<String>::new()),
    )
    .unwrap();
    let original = IngestResult {
        event_id: "legacy-event".into(),
        replayed: false,
    };
    let mut transaction = deps.store.begin_ingest_transaction().await.unwrap();
    transaction
        .append_event(StoredEvent::new(
            original.event_id.clone(),
            timestamp("2020-01-01T00:00:00.000000001Z"),
            event(),
        ))
        .await
        .unwrap();
    transaction
        .upsert_claim(StoredClaim::new(
            "legacy-event:claim:0".into(),
            draft(),
            ClaimStatus::Active,
        ))
        .await
        .unwrap();
    transaction
        .append_write_receipt(
            &request,
            agent_llm_mm::ports::write_receipt::receipt_result(&request, &original).unwrap(),
            timestamp("2020-01-01T00:00:00.000000001Z"),
        )
        .await
        .unwrap();
    transaction.commit().await.unwrap();
    let before = deps.records().await;
    let result = ingest_interaction::execute(&deps, input(key))
        .await
        .unwrap();
    assert!(result.replayed);
    assert_eq!(result.event_id, "legacy-event");
    assert_eq!(deps.records().await, before);
    assert_eq!(before.1[0].recorded_at, None);
}

#[tokio::test]
async fn deserialized_invalid_observation_is_rejected_before_writing() {
    let deps = Deps::new().await;
    let mut json = serde_json::to_value(input("invalid")).unwrap();
    json["observed_at"] = "2026-10-09T10:00:00.1234567890Z".into();
    let input: IngestInput = serde_json::from_value(json).unwrap();
    assert!(ingest_interaction::execute(&deps, input).await.is_err());
    let (events, claims) = deps.records().await;
    assert!(events.is_empty());
    assert!(claims.is_empty());
}

#[tokio::test]
async fn union_orders_known_claim_recording_times_and_keeps_unknown_last() {
    use agent_llm_mm::{
        application::search_memory::{self, MemoryRecordType, SearchMemoryInput},
        ports::EventStore,
    };
    let deps = Deps::new().await;
    for (id, time) in [
        ("zzzz-unknown", None),
        ("zz-new", Some("2026-10-09T10:00:00.000000009Z")),
        ("aa-old", Some("2026-10-09T10:00:00.000000001Z")),
    ] {
        deps.store
            .upsert_claim(
                StoredClaim::new(id.into(), draft(), ClaimStatus::Active)
                    .with_temporal_metadata(time.map(timestamp), None),
            )
            .await
            .unwrap();
    }
    deps.store
        .append_event(StoredEvent::new(
            "between".into(),
            timestamp("2026-10-09T10:00:00.000000005Z"),
            event(),
        ))
        .await
        .unwrap();
    let query = SearchMemoryInput {
        namespace: namespace(),
        record_types: vec![MemoryRecordType::Event, MemoryRecordType::Claim],
        event_reference: None,
        kind: None,
        recorded_after: None,
        recorded_before: None,
        claim_reference: None,
        claim_status: None,
        mode: None,
        episode_reference: None,
        reflection_reference: None,
        limit: 4,
    };
    let result = search_memory::execute(&deps.store, query.clone())
        .await
        .unwrap();
    let ids = result
        .records
        .iter()
        .map(|record| match record {
            SearchMemoryRecord::Event { id, .. } | SearchMemoryRecord::Claim { id, .. } => {
                id.as_str()
            }
            _ => panic!("unexpected record type"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "claim:zz-new",
            "event:between",
            "claim:aa-old",
            "claim:zzzz-unknown"
        ]
    );
    let limited = search_memory::execute(
        &deps.store,
        SearchMemoryInput {
            limit: 1,
            ..query.clone()
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(&limited.records[0], SearchMemoryRecord::Claim { id, .. } if id == "claim:zz-new")
    );
    // Per-type selection must also retain the union's ID tie-break before LIMIT.
    deps.store
        .upsert_claim(
            StoredClaim::new("zzz-same-time".into(), draft(), ClaimStatus::Active)
                .with_recorded_at(timestamp("2026-10-09T10:00:00.000000009Z")),
        )
        .await
        .unwrap();
    let limited = search_memory::execute(&deps.store, SearchMemoryInput { limit: 1, ..query })
        .await
        .unwrap();
    assert!(
        matches!(&limited.records[0], SearchMemoryRecord::Claim { id, .. } if id == "claim:zzz-same-time")
    );
}

#[test]
fn event_source_keeps_nanoseconds_and_exact_observation_while_legacy_json_loads() {
    let recorded = timestamp("2026-10-09T15:45:00.000000009+05:45");
    let legacy = StoredEvent::new("legacy-event".into(), recorded, event());
    let mut old_json = serde_json::to_value(&legacy).unwrap();
    old_json.as_object_mut().unwrap().remove("observed_at");
    assert_eq!(
        serde_json::from_value::<StoredEvent>(old_json).unwrap(),
        legacy
    );
    let observed = "2000-01-01T05:45:00.123456789+05:45";
    let source = SearchMemoryRecord::from(agent_llm_mm::ports::EventReadRecord::new(
        legacy.with_observed_at(Some(observed.into())),
        vec![],
        vec![],
    ));
    let json = serde_json::to_value(source).unwrap();
    assert_eq!(json["recorded_at"], "2026-10-09T10:00:00.000000009Z");
    assert_eq!(json["observed_at"], observed);
}
