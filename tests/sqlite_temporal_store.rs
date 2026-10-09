use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::{
        recall_memory::{self, RecallMemoryInput},
        search_memory::{self, MemoryRecordType, SearchMemoryInput, SearchMemoryRecord},
    },
    domain::{
        claim::ClaimDraft,
        event::{Event, EventReference},
        reflection::Reflection,
        snapshot::SnapshotTimeWindow,
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    ports::{
        ClaimRecordQuery, ClaimStatus, ClaimStore, EpisodeStore, EventRecordQuery, EventStore,
        MemoryReadStore, ReflectionStore, StoredClaim, StoredEvent, StoredReflection,
    },
};
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

struct Fixture {
    _dir: tempfile::TempDir,
    store: SqliteStore,
    pool: SqlitePool,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("temporal.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        Self {
            _dir: dir,
            store,
            pool,
        }
    }
    async fn raw_event(&self, id: &str, raw: &str, ns: &str) {
        sqlx::query("INSERT INTO events(event_id, recorded_at, owner, namespace, kind, summary) VALUES (?, ?, 'world', ?, 'observation', 'coffee')")
            .bind(id).bind(raw).bind(ns).execute(&self.pool).await.unwrap();
    }
    async fn events(
        &self,
        after: Option<DateTime<Utc>>,
        before: Option<DateTime<Utc>>,
    ) -> Vec<StoredEvent> {
        self.store
            .query_event_records(EventRecordQuery {
                scope: scope(),
                event_reference: None,
                kind: None,
                recorded_after: after,
                recorded_before: before,
                limit: 100,
            })
            .await
            .unwrap()
            .into_iter()
            .map(|record| record.event)
            .collect()
    }
    async fn claims(&self) -> Vec<StoredClaim> {
        self.store
            .query_claim_records(ClaimRecordQuery {
                scope: scope(),
                claim_reference: None,
                status: None,
                mode: None,
                limit: 100,
            })
            .await
            .unwrap()
            .into_iter()
            .map(|record| record.claim)
            .collect()
    }
}
fn ns() -> Namespace {
    Namespace::parse("project/temporal-store").unwrap()
}
fn scope() -> MemoryScope {
    MemoryScope::for_namespace(ns())
}
fn time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .unwrap()
        .with_timezone(&Utc)
}
fn claim(id: &str) -> StoredClaim {
    StoredClaim::new(
        id.into(),
        ClaimDraft::new_with_namespace(
            Owner::World,
            ns(),
            "coffee",
            "is",
            "useful",
            Mode::Observed,
        ),
        ClaimStatus::Active,
    )
}

#[tokio::test]
async fn temporal_metadata_round_trips_and_low_level_upsert_preserves_creation_facts() {
    let f = Fixture::new().await;
    let recorded = time("2026-10-09T10:00:00.123456789Z");
    let observed = "1999-01-01T01:02:03.123456789+01:00".to_owned();
    f.store
        .append_event(
            StoredEvent::new(
                "event".into(),
                recorded,
                Event::new_with_namespace(Owner::World, ns(), EventKind::Observation, "coffee")
                    .unwrap(),
            )
            .with_observed_at(Some(observed.clone())),
        )
        .await
        .unwrap();
    f.store
        .upsert_claim(claim("known").with_temporal_metadata(Some(recorded), Some(observed.clone())))
        .await
        .unwrap();
    f.store.upsert_claim(claim("unknown")).await.unwrap();
    // An upsert changes claim fields/status, never its original temporal provenance.
    for id in ["known", "unknown"] {
        f.store
            .upsert_claim(claim(id).with_temporal_metadata(
                Some(recorded + chrono::Duration::days(1)),
                Some("2020-01-01T00:00:00Z".into()),
            ))
            .await
            .unwrap();
    }
    let event = &f.events(None, None).await[0];
    assert_eq!(event.recorded_at, recorded);
    assert_eq!(event.observed_at.as_deref(), Some(observed.as_str()));
    let claims = f.claims().await;
    assert_eq!(claims[0].recorded_at, Some(recorded));
    assert_eq!(claims[0].observed_at.as_deref(), Some(observed.as_str()));
    assert_eq!(claims[1].recorded_at, None);
    assert_eq!(claims[1].observed_at, None);
    let active = f.store.list_active_claims_in_scope(&scope()).await.unwrap();
    assert_eq!(active[0].recorded_at, Some(recorded));
    assert_eq!(
        f.store.list_active_claims().await.unwrap()[0].recorded_at,
        Some(recorded)
    );
}

#[tokio::test]
async fn normalized_time_is_offset_equivalent_nanosecond_exact_and_raw_preserving() {
    let f = Fixture::new().await;
    for (id, raw) in [
        ("prior", "2026-10-09T10:00:00.999999998Z"),
        ("zulu", "2026-10-09T10:00:00.999999999Z"),
        ("offset", "2026-10-09T11:00:00.999999999+01:00"),
        ("next", "2026-10-09T10:00:01.000000000Z"),
    ] {
        f.raw_event(id, raw, ns().as_str()).await;
    }
    f.raw_event(
        "other-scope",
        "2026-10-09T10:00:00.999999999Z",
        "project/other",
    )
    .await;
    let exact = time("2026-10-09T10:00:00.999999999Z");
    let records = f.events(Some(exact), Some(exact)).await;
    assert_eq!(
        records
            .iter()
            .map(|e| e.event_id.as_str())
            .collect::<Vec<_>>(),
        ["offset", "zulu"]
    );
    assert!(records.iter().all(|e| e.recorded_at == exact));
    let row = sqlx::query("SELECT recorded_at, recorded_at_seconds, recorded_at_nanos, recorded_at_sort_key FROM events WHERE event_id='offset'")
        .fetch_one(&f.pool).await.unwrap();
    assert_eq!(
        row.get::<String, _>("recorded_at"),
        "2026-10-09T11:00:00.999999999+01:00"
    );
    assert_eq!(row.get::<i64, _>("recorded_at_seconds"), exact.timestamp());
    assert_eq!(row.get::<i64, _>("recorded_at_nanos"), 999_999_999);
    assert_eq!(
        row.get::<String, _>("recorded_at_sort_key"),
        format!(
            "{:020}:{:010}",
            exact.timestamp() + 10_000_000_000_000_i64,
            exact.timestamp_subsec_nanos()
        )
    );
    sqlx::query(
        "UPDATE events SET recorded_at='2026-10-09T09:00:00.000000001Z' WHERE event_id='offset'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(f.events(Some(exact), Some(exact)).await.len(), 1);
}

#[tokio::test]
async fn empty_precise_snapshot_window_and_manifest_never_widen() {
    let f = Fixture::new().await;
    f.raw_event("event", "2026-10-09T10:00:00.000000001Z", ns().as_str())
        .await;
    let absent = time("2026-10-09T10:00:00.000000002Z");
    let window = SnapshotTimeWindow {
        recorded_after: Some(absent),
        recorded_before: Some(absent),
    };
    let manifest = vec![EventReference::parse("event:event").unwrap()];
    for selection in [None, Some(manifest.as_slice()), Some(&[][..])] {
        assert!(
            f.store
                .list_event_references_for_snapshot(&scope(), selection, &window)
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert!(f.events(Some(absent), Some(absent)).await.is_empty());
}

#[tokio::test]
async fn invalid_legacy_timestamps_remain_raw_unknown_and_outside_time_windows() {
    let f = Fixture::new().await;
    let invalid = [
        "not-a-date",
        "2026-02-30T10:00:00Z",
        "2026-10-09T25:00:00Z",
        "2026-10-09T10:00:00.1234567890Z",
    ];
    for (i, raw) in invalid.into_iter().enumerate() {
        f.raw_event(&format!("invalid-{i}"), raw, ns().as_str())
            .await;
    }
    let rows = sqlx::query("SELECT recorded_at, recorded_at_seconds, recorded_at_nanos, recorded_at_sort_key FROM events ORDER BY rowid").fetch_all(&f.pool).await.unwrap();
    for (row, raw) in rows.iter().zip(invalid) {
        assert_eq!(row.get::<String, _>("recorded_at"), raw);
        assert_eq!(row.get::<Option<i64>, _>("recorded_at_seconds"), None);
        assert_eq!(row.get::<Option<i64>, _>("recorded_at_nanos"), None);
        assert_eq!(row.get::<Option<String>, _>("recorded_at_sort_key"), None);
    }
    assert!(
        f.events(
            Some(time("2020-01-01T00:00:00Z")),
            Some(time("2030-01-01T00:00:00Z"))
        )
        .await
        .is_empty()
    );
    // Unbounded hydration still reports an explicit parse error; no fabricated date.
    assert!(
        f.store
            .query_event_records(EventRecordQuery {
                scope: scope(),
                event_reference: None,
                kind: None,
                recorded_after: None,
                recorded_before: None,
                limit: 100,
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn namespace_time_query_plan_uses_materialized_time_index() {
    let f = Fixture::new().await;
    for i in 0..20 {
        f.raw_event(
            &format!("event-{i}"),
            "2026-10-09T10:00:00.000000001Z",
            ns().as_str(),
        )
        .await;
    }
    let rows = sqlx::query("EXPLAIN QUERY PLAN SELECT event_id, recorded_at, observed_at, owner, namespace, kind, summary, feedback_json FROM events WHERE owner=? AND namespace=? AND recorded_at_sort_key>=? AND recorded_at_sort_key<=? ORDER BY recorded_at_sort_key DESC, rowid DESC LIMIT 100")
        .bind("world").bind(ns().as_str())
        .bind(format!("{:020}:0000000001", time("2026-10-09T10:00:00Z").timestamp() + 10_000_000_000_000_i64))
        .bind(format!("{:020}:0000000002", time("2026-10-09T10:00:00Z").timestamp() + 10_000_000_000_000_i64))
        .fetch_all(&f.pool).await.unwrap();
    let plan: Vec<String> = rows.iter().map(|row| row.get("detail")).collect();
    assert!(
        plan.iter().any(|detail| detail
            .contains("SEARCH events USING INDEX idx_events_scope_recorded")
            && detail.contains("recorded_at_sort_key>?")
            && detail.contains("recorded_at_sort_key<?")),
        "{plan:?}"
    );
    assert!(
        !plan.iter().any(|detail| detail == "SCAN events"),
        "{plan:?}"
    );
}

#[tokio::test]
async fn recall_uses_known_claim_recency_without_inventing_unknown_times() {
    let f = Fixture::new().await;
    for (id, recorded) in [
        ("a-unknown", None),
        ("b-old", Some("2026-10-09T10:00:00.000000001Z")),
        ("z-new", Some("2026-10-09T10:00:00.000000002Z")),
    ] {
        f.store
            .upsert_claim(claim(id).with_temporal_metadata(recorded.map(time), None))
            .await
            .unwrap();
    }
    for (id, raw) in [
        ("a-event-old", "2026-10-09T11:00:00.000000001+01:00"),
        ("z-event-new", "2026-10-09T10:00:00.000000002Z"),
    ] {
        f.raw_event(id, raw, ns().as_str()).await;
    }
    let result = recall_memory::execute(
        &f.store,
        RecallMemoryInput {
            namespace: ns(),
            query: "coffee".into(),
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.strategy, "fts5_trigram");
    assert_eq!(result.index_warning, None);
    let ids: Vec<_> = result
        .records
        .iter()
        .map(|hit| match &hit.record {
            SearchMemoryRecord::Claim { id, .. } | SearchMemoryRecord::Event { id, .. } => {
                id.as_str()
            }
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        ids,
        [
            "claim:z-new",
            "event:z-event-new",
            "claim:b-old",
            "event:a-event-old",
            "claim:a-unknown"
        ]
    );
    assert_eq!(
        result.records[0].explanation.time_basis,
        "claim_recorded_at_desc_after_term_count"
    );
    assert_eq!(
        result.records[4].explanation.time_basis,
        "claim_creation_time_unknown_no_recency_assumed"
    );
}

#[tokio::test]
async fn temporal_trigger_updates_do_not_duplicate_or_churn_fts_documents() {
    let f = Fixture::new().await;
    f.raw_event("event", "2026-10-09T10:00:00.999999999Z", ns().as_str())
        .await;
    f.store
        .upsert_claim(claim("claim").with_recorded_at(time("2026-10-09T10:00:00.999999999Z")))
        .await
        .unwrap();
    let before: Vec<(i64, String)> =
        sqlx::query_as("SELECT doc_id, record_id FROM text_recall_documents ORDER BY doc_id")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(before.len(), 2);
    for table in ["events", "claims"] {
        sqlx::query(&format!("UPDATE {table} SET recorded_at='2026-10-10T10:00:00.000000001Z', observed_at='2000-01-01T00:00:00Z'"))
            .execute(&f.pool).await.unwrap();
    }
    let after: Vec<(i64, String)> =
        sqlx::query_as("SELECT doc_id, record_id FROM text_recall_documents ORDER BY doc_id")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let result = recall_memory::execute(
        &f.store,
        RecallMemoryInput {
            namespace: ns(),
            query: "coffee".into(),
            limit: 10,
        },
    )
    .await
    .unwrap();
    assert_eq!(result.strategy, "fts5_trigram");
    assert_eq!(result.records.len(), 2);
    assert_eq!(result.index_warning, None);
}

#[tokio::test]
async fn lowercase_extreme_offsets_and_leap_seconds_have_exact_snapshot_keys() {
    let f = Fixture::new().await;
    for (id, raw, utc) in [
        (
            "lowercase",
            "2026-10-09t10:00:00.123456789z",
            "2026-10-09T10:00:00.123456789Z",
        ),
        (
            "offset-positive",
            "2026-10-10T09:00:00.000000001+23:00",
            "2026-10-09T10:00:00.000000001Z",
        ),
        (
            "offset-negative",
            "2026-10-08T11:00:00.000000002-23:00",
            "2026-10-09T10:00:00.000000002Z",
        ),
        (
            "leap",
            "2016-12-31T23:59:60.123456789Z",
            "2016-12-31T23:59:60.123456789Z",
        ),
        (
            "year-zero-offset",
            "0000-01-01T00:00:00+23:00",
            "0000-01-01T00:00:00+23:00",
        ),
        (
            "year-end-offset",
            "9999-12-31T23:59:59-23:00",
            "9999-12-31T23:59:59-23:00",
        ),
    ] {
        f.raw_event(id, raw, ns().as_str()).await;
        let instant = time(utc);
        let records = f.events(Some(instant), Some(instant)).await;
        assert_eq!(records.len(), 1, "raw={raw}");
        assert_eq!(records[0].event_id, id);
        assert_eq!(records[0].recorded_at, instant);
        let manifest = vec![EventReference::parse(format!("event:{id}")).unwrap()];
        let window = SnapshotTimeWindow {
            recorded_after: Some(instant),
            recorded_before: Some(instant),
        };
        assert_eq!(
            f.store
                .list_event_references_for_snapshot(&scope(), Some(&manifest), &window)
                .await
                .unwrap(),
            [format!("event:{id}")]
        );
    }
}

fn browse_input(record_type: MemoryRecordType, union: bool) -> SearchMemoryInput {
    let other = match record_type {
        MemoryRecordType::Event | MemoryRecordType::Reflection => MemoryRecordType::Episode,
        MemoryRecordType::Claim | MemoryRecordType::Episode => MemoryRecordType::Reflection,
    };
    SearchMemoryInput {
        namespace: ns(),
        record_types: if union {
            vec![record_type, other]
        } else {
            vec![record_type]
        },
        event_reference: None,
        kind: None,
        recorded_after: None,
        recorded_before: None,
        claim_reference: None,
        claim_status: None,
        mode: None,
        episode_reference: None,
        reflection_reference: None,
        limit: 2,
    }
}
fn record_id(record: &SearchMemoryRecord) -> &str {
    match record {
        SearchMemoryRecord::Event { id, .. }
        | SearchMemoryRecord::Claim { id, .. }
        | SearchMemoryRecord::Episode { id, .. }
        | SearchMemoryRecord::Reflection { id, .. } => id,
    }
}

#[tokio::test]
async fn union_prelimit_ties_match_final_order_without_changing_standalone_browse() {
    let instant = time("2026-10-09T10:00:00.123456789Z");
    for kind in [
        MemoryRecordType::Event,
        MemoryRecordType::Claim,
        MemoryRecordType::Episode,
        MemoryRecordType::Reflection,
    ] {
        let f = Fixture::new().await;
        if kind == MemoryRecordType::Reflection {
            f.store.upsert_claim(claim("anchor")).await.unwrap();
        }
        for id in ["z", "m", "a"] {
            match kind {
                MemoryRecordType::Event => {
                    f.raw_event(id, "2026-10-09T10:00:00.123456789Z", ns().as_str())
                        .await
                }
                MemoryRecordType::Claim => f
                    .store
                    .upsert_claim(claim(id).with_recorded_at(instant))
                    .await
                    .unwrap(),
                MemoryRecordType::Episode => {
                    f.raw_event(id, "2026-10-09T10:00:00.123456789Z", ns().as_str())
                        .await;
                    f.store
                        .record_event_in_episode(format!("episode:{id}"), id.into())
                        .await
                        .unwrap();
                }
                MemoryRecordType::Reflection => f
                    .store
                    .append_reflection(StoredReflection::new(
                        id.into(),
                        instant,
                        Reflection::new("scoped audit"),
                        Some("anchor".into()),
                        None,
                    ))
                    .await
                    .unwrap(),
            }
        }
        let prefix = match kind {
            MemoryRecordType::Event => "event:",
            MemoryRecordType::Claim => "claim:",
            MemoryRecordType::Episode => "episode:",
            MemoryRecordType::Reflection => "",
        };
        let union = search_memory::execute(&f.store, browse_input(kind, true))
            .await
            .unwrap();
        assert_eq!(
            union.records.iter().map(record_id).collect::<Vec<_>>(),
            [format!("{prefix}z"), format!("{prefix}m")],
            "union {kind:?}"
        );
        let standalone = search_memory::execute(&f.store, browse_input(kind, false))
            .await
            .unwrap();
        assert_eq!(
            standalone.records.iter().map(record_id).collect::<Vec<_>>(),
            [format!("{prefix}a"), format!("{prefix}m")],
            "standalone {kind:?}"
        );
    }
    // Legacy Claims have no date, but their union ID tie still must precede LIMIT.
    let f = Fixture::new().await;
    for id in ["z", "m", "a"] {
        f.store.upsert_claim(claim(id)).await.unwrap();
    }
    let union = search_memory::execute(&f.store, browse_input(MemoryRecordType::Claim, true))
        .await
        .unwrap();
    assert_eq!(
        union.records.iter().map(record_id).collect::<Vec<_>>(),
        ["claim:z", "claim:m"]
    );
    let standalone = search_memory::execute(&f.store, browse_input(MemoryRecordType::Claim, false))
        .await
        .unwrap();
    assert_eq!(
        standalone.records.iter().map(record_id).collect::<Vec<_>>(),
        ["claim:a", "claim:m"]
    );
}
