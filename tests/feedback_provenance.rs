#[path = "support/legacy_schema.rs"]
mod legacy_schema;
use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, inspect_database, migrate_database},
    application::{
        get_memory::{self, GetMemoryInput, MemoryRecordReference},
        ingest_interaction::{self, IngestInput},
    },
    domain::{
        event::{Event, EventReference},
        feedback::{
            FeedbackMetadata, FeedbackSourceKind, FeedbackVerificationResult, MAX_FEEDBACK_ITEMS,
            MAX_FEEDBACK_TEXT_BYTES,
        },
        types::{EventKind, Namespace, Owner},
    },
    error::AppError,
    ports::{
        Clock, EventStore, IdGenerator, IngestTransaction, IngestTransactionRunner, StoredEvent,
    },
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::sync::atomic::{AtomicU64, Ordering};

fn feedback() -> FeedbackMetadata {
    FeedbackMetadata {
        source_kind: FeedbackSourceKind::ToolReported,
        producer: "test runner (unverified caller label)".into(),
        observed_target: "ledger".into(),
        observed_version: Some("abc123".into()),
        expected: "all checks pass".into(),
        actual: "one check failed".into(),
        verification_method: "cargo test".into(),
        verification_result: FeedbackVerificationResult::Failed,
        limitations: vec!["No independent authentication or verification".into()],
        evidence_refs: vec![],
    }
}
fn event(namespace: &str) -> Event {
    Event::new_with_namespace(
        Owner::World,
        Namespace::parse(namespace).unwrap(),
        EventKind::Observation,
        "reported test result",
    )
    .unwrap()
}
struct Deps {
    store: SqliteStore,
    ids: AtomicU64,
}
#[async_trait]
impl Clock for Deps {
    async fn now(&self) -> Result<DateTime<Utc>, AppError> {
        Ok(Utc::now())
    }
}
#[async_trait]
impl IdGenerator for Deps {
    async fn next_id(&self) -> Result<String, AppError> {
        Ok(format!(
            "feedback-{}",
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
async fn setup() -> (tempfile::TempDir, String, Deps) {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("feedback.sqlite").display());
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    (
        dir,
        url,
        Deps {
            store,
            ids: AtomicU64::new(0),
        },
    )
}
#[test]
fn feedback_validation_is_bounded_and_legacy_events_remain_compatible() {
    let old = Event::new(Owner::World, EventKind::Observation, "old");
    assert!(old.feedback().is_none());
    let value = serde_json::to_value(&old).unwrap();
    assert!(value.get("feedback").is_none());
    assert_eq!(serde_json::from_value::<Event>(value).unwrap(), old);
    let mut f = feedback();
    f.producer = " ".into();
    assert!(f.validate().is_err());
    f = feedback();
    f.actual = "x".repeat(MAX_FEEDBACK_TEXT_BYTES + 1);
    assert!(f.validate().is_err());
    f = feedback();
    f.limitations = vec!["limited".into(); MAX_FEEDBACK_ITEMS + 1];
    assert!(f.validate().is_err());
    f = feedback();
    f.evidence_refs = vec![
        EventReference::parse("event:e1").unwrap(),
        EventReference::parse("e1").unwrap(),
    ];
    assert!(f.validate().is_err());
    let mut unknown = serde_json::to_value(feedback()).unwrap();
    unknown["authenticated"] = true.into();
    assert!(serde_json::from_value::<FeedbackMetadata>(unknown).is_err());
    assert!(
        Event::new(Owner::World, EventKind::Observation, "new")
            .with_feedback(feedback())
            .is_ok()
    );
}
#[tokio::test]
async fn feedback_is_durable_typed_readback_and_retry_payload() {
    let (_dir, _url, deps) = setup().await;
    deps.store
        .append_event(StoredEvent::new(
            "e1".into(),
            Utc::now(),
            event("project/alpha"),
        ))
        .await
        .unwrap();
    let mut f = feedback();
    f.evidence_refs.push(EventReference::parse("e1").unwrap());
    let input = IngestInput::new(
        event("project/alpha").with_feedback(f.clone()).unwrap(),
        vec![],
        None,
    )
    .with_request_id("report-1".into());
    let first = ingest_interaction::execute(&deps, input.clone())
        .await
        .unwrap();
    let retry = ingest_interaction::execute(&deps, input).await.unwrap();
    assert!(retry.replayed);
    assert_eq!(first.event_id, retry.event_id);
    let read = get_memory::execute(
        &deps.store,
        GetMemoryInput {
            namespace: Namespace::parse("project/alpha").unwrap(),
            id: MemoryRecordReference::Event(EventReference::parse(&first.event_id).unwrap()),
        },
    )
    .await
    .unwrap();
    let record = serde_json::to_value(read.record.unwrap()).unwrap();
    assert_eq!(record["feedback"], serde_json::to_value(&f).unwrap());
    f.actual = "a changed claim".into();
    assert!(
        ingest_interaction::execute(
            &deps,
            IngestInput::new(
                event("project/alpha").with_feedback(f).unwrap(),
                vec![],
                None
            )
            .with_request_id("report-1".into())
        )
        .await
        .is_err()
    );
    assert_eq!(deps.store.list_event_references().await.unwrap().len(), 2);
}
#[tokio::test]
async fn feedback_rejects_missing_and_cross_scope_evidence_atomically() {
    let (_dir, _url, deps) = setup().await;
    deps.store
        .append_event(StoredEvent::new(
            "foreign".into(),
            Utc::now(),
            event("project/beta"),
        ))
        .await
        .unwrap();
    for reference in ["missing", "foreign"] {
        let mut f = feedback();
        f.evidence_refs
            .push(EventReference::parse(reference).unwrap());
        let result = ingest_interaction::execute(
            &deps,
            IngestInput::new(
                event("project/alpha").with_feedback(f).unwrap(),
                vec![],
                Some("episode".into()),
            ),
        )
        .await;
        assert!(result.is_err(), "{reference} evidence must not be accepted");
    }
    assert_eq!(
        deps.store.list_event_references().await.unwrap(),
        vec!["event:foreign"]
    );
}
#[tokio::test]
async fn version_three_migration_preserves_events_and_canonical_structure() {
    let (_dir, url, deps) = setup().await;
    deps.store
        .append_event(StoredEvent::new(
            "old".into(),
            Utc::now(),
            event("project/alpha"),
        ))
        .await
        .unwrap();
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query(
        "INSERT INTO episode_events (episode_reference, event_id) VALUES ('legacy-episode', 'old')",
    )
    .execute(&pool)
    .await
    .unwrap();
    {
        let mut connection = pool.acquire().await.unwrap();
        legacy_schema::remove_v5_objects(&mut connection).await;
    }
    sqlx::raw_sql("ALTER TABLE events DROP COLUMN feedback_json; DELETE FROM schema_migrations WHERE version >= 4; PRAGMA user_version = 3;").execute(&pool).await.unwrap();
    pool.close().await;
    drop(deps);
    let before = inspect_database(&url).await.unwrap();
    assert!(before.migration_required);
    let migrated = migrate_database(&url).await.unwrap();
    assert_eq!(migrated.status, "current");
    assert!(migrated.preserved_row_counts);
    let current = inspect_database(&url).await.unwrap();
    assert!(current.schema_structure_valid);
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    let read = get_memory::execute(
        &store,
        GetMemoryInput {
            namespace: Namespace::parse("project/alpha").unwrap(),
            id: MemoryRecordReference::Event(EventReference::parse("old").unwrap()),
        },
    )
    .await
    .unwrap();
    let value = serde_json::to_value(read.record.unwrap()).unwrap();
    assert!(value.get("feedback").is_none());
}
