use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::run_reflection::{self, ReflectionInput},
    domain::{
        claim::ClaimDraft,
        event::{Event, EventReference},
        reflection::Reflection,
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::*,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

struct Deps {
    store: SqliteStore,
    pool: SqlitePool,
    mutation_before_transaction: Mutex<Option<&'static str>>,
    ids: AtomicU64,
    _directory: tempfile::TempDir,
}
impl Deps {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}",
            directory.path().join("test.sqlite").display()
        );
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        store
            .upsert_claim(StoredClaim::new(
                "original".into(),
                claim("old"),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();
        store
            .append_event(StoredEvent::new(
                "evidence".into(),
                Utc::now(),
                Event::new_with_namespace(
                    Owner::World,
                    Namespace::parse("project/alpha").unwrap(),
                    EventKind::Observation,
                    "updated fact",
                )
                .unwrap(),
            ))
            .await
            .unwrap();
        Self {
            store,
            pool,
            mutation_before_transaction: Mutex::new(None),
            ids: AtomicU64::new(0),
            _directory: directory,
        }
    }
    async fn assert_no_correction(&self) {
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT count(*) FROM reflections), (SELECT count(*) FROM claims)",
        )
        .fetch_one(&self.pool)
        .await
        .unwrap();
        assert_eq!(counts, (0, 1));
    }
}
fn claim(value: &str) -> ClaimDraft {
    ClaimDraft::new(Owner::World, "project.setting", "is", value, Mode::Observed)
        .with_namespace(Namespace::parse("project/alpha").unwrap())
}
fn input() -> ReflectionInput {
    ReflectionInput::new(
        Reflection::new("Correct setting"),
        "original",
        Some(claim("new")),
        vec!["evidence".into()],
    )
    .with_strict_evidence_scope()
}
#[async_trait]
impl IdGenerator for Deps {
    async fn next_id(&self) -> Result<String, AppError> {
        Ok(format!(
            "reflection-{}",
            self.ids.fetch_add(1, Ordering::SeqCst)
        ))
    }
}
#[async_trait]
impl Clock for Deps {
    async fn now(&self) -> Result<DateTime<Utc>, AppError> {
        Ok(Utc::now())
    }
}
#[async_trait]
impl ReflectionTransactionRunner for Deps {
    async fn begin_reflection_transaction(
        &self,
    ) -> Result<Box<dyn ReflectionTransaction + Send + '_>, AppError> {
        let mutation = self.mutation_before_transaction.lock().unwrap().take();
        assert_ne!(
            mutation,
            Some("query_must_run_first"),
            "evidence query must run before writer reservation"
        );
        if let Some(sql) = mutation {
            sqlx::query(sql)
                .execute(&self.pool)
                .await
                .map_err(|e| AppError::Message(e.to_string()))?;
        }
        self.store.begin_reflection_transaction().await
    }
}
#[async_trait]
impl EventStore for Deps {
    async fn append_event(&self, event: StoredEvent) -> Result<(), AppError> {
        self.store.append_event(event).await
    }
    async fn list_event_references(&self) -> Result<Vec<String>, AppError> {
        self.store.list_event_references().await
    }
    async fn list_recorded_at_for_snapshot_manifest(
        &self,
        scope: &MemoryScope,
        manifest: &[EventReference],
    ) -> Result<Vec<DateTime<Utc>>, AppError> {
        self.store
            .list_recorded_at_for_snapshot_manifest(scope, manifest)
            .await
    }
    async fn query_evidence_event_ids(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        {
            let mut marker = self.mutation_before_transaction.lock().unwrap();
            if *marker == Some("query_must_run_first") {
                *marker = None;
            }
        }
        self.store.query_evidence_event_ids(query).await
    }
    async fn query_evidence_event_ids_unbounded(
        &self,
        query: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        self.store.query_evidence_event_ids_unbounded(query).await
    }
    async fn has_event(&self, id: &str) -> Result<bool, AppError> {
        self.store.has_event(id).await
    }
}

#[tokio::test]
async fn transaction_rechecks_evidence_removed_or_moved_after_preflight() {
    for mutation in [
        "DELETE FROM events WHERE event_id = 'evidence'",
        "UPDATE events SET namespace = 'project/beta' WHERE event_id = 'evidence'",
    ] {
        let deps = Deps::new().await;
        *deps.mutation_before_transaction.lock().unwrap() = Some(mutation);
        assert!(matches!(
            run_reflection::execute(&deps, input()).await,
            Err(AppError::InvalidParams(_))
        ));
        deps.assert_no_correction().await;
    }
}

#[tokio::test]
async fn transaction_rechecks_target_state_and_scope_after_preflight() {
    for mutation in [
        "UPDATE claims SET status = 'superseded' WHERE claim_id = 'original'",
        "UPDATE claims SET namespace = 'project/beta' WHERE claim_id = 'original'",
        "DELETE FROM claims WHERE claim_id = 'original'",
    ] {
        let deps = Deps::new().await;
        *deps.mutation_before_transaction.lock().unwrap() = Some(mutation);
        assert!(matches!(
            run_reflection::execute(&deps, input()).await,
            Err(AppError::InvalidParams(_))
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM reflections")
                .fetch_one(&deps.pool)
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn competing_corrections_create_only_one_successor_and_audit() {
    let deps = Deps::new().await;
    let (first, second) = tokio::join!(
        run_reflection::execute(&deps, input()),
        run_reflection::execute(&deps, input())
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM reflections), (SELECT count(*) FROM claims), (SELECT count(*) FROM evidence_links)").fetch_one(&deps.pool).await.unwrap();
    assert_eq!(counts, (1, 2, 1));
}

#[tokio::test]
async fn failed_compare_and_set_poisons_and_rolls_back_transaction() {
    let deps = Deps::new().await;
    let mut transaction = deps.store.begin_reflection_transaction().await.unwrap();
    transaction
        .upsert_claim(StoredClaim::new(
            "orphan".into(),
            claim("orphan"),
            ClaimStatus::Active,
        ))
        .await
        .unwrap();
    assert!(
        transaction
            .compare_and_set_claim_status(
                "original",
                ClaimStatus::Disputed,
                ClaimStatus::Superseded
            )
            .await
            .is_err()
    );
    assert!(transaction.commit().await.is_err());
    deps.assert_no_correction().await;
}

#[async_trait]
impl IngestTransactionRunner for Deps {
    async fn begin_ingest_transaction(
        &self,
    ) -> Result<Box<dyn IngestTransaction + Send + '_>, AppError> {
        self.store.begin_ingest_transaction().await
    }
}

fn correction_request(
    key: Option<&str>,
) -> agent_llm_mm::application::supersede_memory::SupersedeMemoryInput {
    agent_llm_mm::application::supersede_memory::SupersedeMemoryInput {
        namespace: Namespace::parse("project/alpha").unwrap(),
        claim_reference: agent_llm_mm::domain::claim::ClaimReference::parse("original").unwrap(),
        replacement_claim: claim("new"),
        evidence_event_ids: vec![EventReference::parse("evidence").unwrap()],
        summary: "Correct setting".into(),
        request_id: key.map(str::to_owned),
    }
}

#[tokio::test]
async fn correction_retry_after_reopen_returns_original_result_and_rejects_changed_payload() {
    use agent_llm_mm::application::supersede_memory::execute;
    let mut deps = Deps::new().await;
    let original = execute(&deps, correction_request(Some("private-retry-key")))
        .await
        .unwrap();
    let url = format!(
        "sqlite://{}",
        deps._directory.path().join("test.sqlite").display()
    );
    deps.store = SqliteStore::bootstrap(&url).await.unwrap();
    let retry = execute(&deps, correction_request(Some("private-retry-key")))
        .await
        .unwrap();
    assert_eq!(retry.reflection_id, original.reflection_id);
    assert_eq!(retry.replacement_claim_id, original.replacement_claim_id);
    let mut changed = correction_request(Some("private-retry-key"));
    changed.replacement_claim = claim("changed");
    assert!(matches!(
        execute(&deps, changed).await,
        Err(AppError::InvalidParams(_))
    ));
    let audit: Vec<(String, String, String)> = sqlx::query_as("SELECT operation_id, request_summary_json, response_summary_json FROM operation_log WHERE actor_id = 'durable_write_receipt_v1'").fetch_all(&deps.pool).await.unwrap();
    assert_eq!(audit.len(), 1);
    let text = format!("{audit:?}");
    assert!(!text.contains("private-retry-key"));
    assert!(!text.contains("Correct setting"));
    assert!(!text.contains("project.setting"));
}

#[tokio::test]
async fn concurrent_same_key_corrections_replay_without_duplicate_writes() {
    use agent_llm_mm::application::supersede_memory::execute;
    let deps = Deps::new().await;
    let (first, second) = tokio::join!(
        execute(&deps, correction_request(Some("same-key"))),
        execute(&deps, correction_request(Some("same-key")))
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first.reflection_id, second.reflection_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM reflections")
            .fetch_one(&deps.pool)
            .await
            .unwrap(),
        1
    );
}

fn ingest_request(
    key: &str,
    summary: &str,
) -> agent_llm_mm::application::ingest_interaction::IngestInput {
    agent_llm_mm::application::ingest_interaction::IngestInput::new(
        Event::new_with_namespace(
            Owner::World,
            Namespace::parse("project/alpha").unwrap(),
            EventKind::Observation,
            summary,
        )
        .unwrap(),
        vec![claim("ingested")],
        None,
    )
    .with_request_id(key.into())
}

#[tokio::test]
async fn ingest_retry_replays_original_event_and_changed_payload_fails() {
    use agent_llm_mm::application::ingest_interaction::execute;
    let deps = Deps::new().await;
    let (first, second) = tokio::join!(
        execute(&deps, ingest_request("ingest-key", "observation")),
        execute(&deps, ingest_request("ingest-key", "observation"))
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first.event_id, second.event_id);
    assert_ne!(first.replayed, second.replayed);
    assert!(matches!(
        execute(&deps, ingest_request("ingest-key", "changed")).await,
        Err(AppError::InvalidParams(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
            .fetch_one(&deps.pool)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn receipt_insert_failure_rolls_back_entire_correction_and_ingest() {
    let deps = Deps::new().await;
    sqlx::query("CREATE TRIGGER fail_receipt BEFORE INSERT ON operation_log WHEN NEW.actor_id = 'durable_write_receipt_v1' BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END").execute(&deps.pool).await.unwrap();
    assert!(
        agent_llm_mm::application::supersede_memory::execute(
            &deps,
            correction_request(Some("will-fail"))
        )
        .await
        .is_err()
    );
    deps.assert_no_correction().await;
    assert!(
        agent_llm_mm::application::ingest_interaction::execute(
            &deps,
            ingest_request("will-fail", "not durable")
        )
        .await
        .is_err()
    );
    deps.assert_no_correction().await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
            .fetch_one(&deps.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM claims WHERE claim_id = 'original'")
            .fetch_one(&deps.pool)
            .await
            .unwrap(),
        "active"
    );
}

#[tokio::test]
async fn unkeyed_writes_also_require_atomic_success_receipts() {
    let deps = Deps::new().await;
    run_reflection::execute(&deps, input()).await.unwrap();
    let ingest = agent_llm_mm::application::ingest_interaction::IngestInput::new(
        Event::new(Owner::World, EventKind::Observation, "unkeyed observation"),
        vec![],
        None,
    );
    agent_llm_mm::application::ingest_interaction::execute(&deps, ingest)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM operation_log WHERE actor_id = 'durable_write_receipt_v1'"
        )
        .fetch_one(&deps.pool)
        .await
        .unwrap(),
        2
    );
}

#[test]
fn receipt_identity_is_operation_and_namespace_scoped_and_keys_are_bounded() {
    let first = WriteReceiptRequest::new("ingest", "project/alpha", "key", &"payload").unwrap();
    let other_scope =
        WriteReceiptRequest::new("ingest", "project/beta", "key", &"payload").unwrap();
    let other_operation =
        WriteReceiptRequest::new("supersede_memory", "project/alpha", "key", &"payload").unwrap();
    assert_ne!(first.operation_id, other_scope.operation_id);
    assert_ne!(first.operation_id, other_operation.operation_id);
    for key in ["", " leading", "trailing ", "control\nkey"] {
        assert!(WriteReceiptRequest::new("ingest", "world", key, &"payload").is_err());
    }
    assert!(WriteReceiptRequest::new("ingest", "world", &"x".repeat(129), &"payload").is_err());
}

#[tokio::test]
async fn reflection_resolves_query_before_reserving_writer_connection() {
    let deps = Deps::new().await;
    *deps.mutation_before_transaction.lock().unwrap() = Some("query_must_run_first");
    let input = input().with_replacement_evidence_query(EvidenceQuery {
        namespace: Some(Namespace::parse("project/alpha").unwrap()),
        owner: Some(Owner::World),
        kind: None,
        limit: Some(1),
        recorded_after: None,
        recorded_before: None,
        event_id_prefix: None,
    });
    run_reflection::execute(&deps, input).await.unwrap();
}

#[tokio::test]
async fn retry_rejects_changed_trigger_hints() {
    use agent_llm_mm::application::ingest_interaction::{self, IngestInput};
    let deps = Deps::new().await;
    let input = IngestInput::new(
        Event::new(Owner::World, EventKind::Observation, "same fact"),
        vec![],
        None,
    )
    .with_request_id("hint-boundary".into());
    ingest_interaction::execute(&deps, input.clone())
        .await
        .unwrap();
    assert!(
        ingest_interaction::execute(&deps, input.with_trigger_hints(vec!["conflict".into()]))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn legacy_self_reflection_rejects_project_evidence_without_partial_mutation() {
    let deps = Deps::new().await;
    sqlx::query("UPDATE claims SET owner = 'self', namespace = 'self' WHERE claim_id = 'original'")
        .execute(&deps.pool)
        .await
        .unwrap();
    let input = ReflectionInput::new(
        Reflection::new("Project observations must not silently rewrite global self memory"),
        "original",
        Some(ClaimDraft::new(
            Owner::Self_,
            "self.role",
            "is",
            "architect",
            Mode::Observed,
        )),
        vec!["evidence".into()],
    );
    assert!(matches!(
        run_reflection::execute(&deps, input).await,
        Err(AppError::InvalidParams(_))
    ));
    deps.assert_no_correction().await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM operation_log WHERE actor_id = 'durable_write_receipt_v1'"
        )
        .fetch_one(&deps.pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM claims WHERE claim_id = 'original'")
            .fetch_one(&deps.pool)
            .await
            .unwrap(),
        "active"
    );
}

#[tokio::test]
async fn scoped_supersede_does_not_mutate_global_identity_or_commitments() {
    use agent_llm_mm::application::supersede_memory::{self, SupersedeMemoryInput};
    use agent_llm_mm::domain::claim::ClaimReference;
    let deps = Deps::new().await;
    let before_identity: Vec<(i64, String)> =
        sqlx::query_as("SELECT position, claim FROM identity_claims ORDER BY position")
            .fetch_all(&deps.pool)
            .await
            .unwrap();
    let before_commitments: Vec<(String, String)> =
        sqlx::query_as("SELECT description, owner FROM commitments ORDER BY description")
            .fetch_all(&deps.pool)
            .await
            .unwrap();
    supersede_memory::execute(
        &deps,
        SupersedeMemoryInput {
            namespace: Namespace::parse("project/alpha").unwrap(),
            claim_reference: ClaimReference::parse("original").unwrap(),
            replacement_claim: claim("new"),
            evidence_event_ids: vec![EventReference::parse("evidence").unwrap()],
            summary: "Scoped correction only".into(),
            request_id: Some("scoped-only".into()),
        },
    )
    .await
    .unwrap();
    let after_identity: Vec<(i64, String)> =
        sqlx::query_as("SELECT position, claim FROM identity_claims ORDER BY position")
            .fetch_all(&deps.pool)
            .await
            .unwrap();
    let after_commitments: Vec<(String, String)> =
        sqlx::query_as("SELECT description, owner FROM commitments ORDER BY description")
            .fetch_all(&deps.pool)
            .await
            .unwrap();
    assert_eq!(before_identity, after_identity);
    assert_eq!(before_commitments, after_commitments);
}
