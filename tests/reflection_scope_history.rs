use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::run_reflection::{self, ReflectionInput},
    domain::{
        claim::ClaimDraft,
        event::{Event, EventReference},
        reflection::Reflection,
        reflection_scope::ReflectionScopeStatus,
        types::{EventKind, MemoryScope, Mode, Namespace},
    },
    error::AppError,
    interfaces::mcp::dto::RunReflectionParams,
    ports::*,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use std::sync::atomic::{AtomicU64, Ordering};

struct Deps {
    store: SqliteStore,
    pool: SqlitePool,
    ids: AtomicU64,
    _dir: tempfile::TempDir,
}
impl Deps {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("scope.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        for (id, namespace) in [
            ("a", Namespace::for_project("a")),
            ("b", Namespace::for_project("b")),
            ("self", Namespace::self_()),
            ("world", Namespace::world()),
        ] {
            store
                .append_event(StoredEvent::new(
                    id.into(),
                    Utc::now(),
                    Event::new_with_namespace(
                        namespace.derived_owner(),
                        namespace,
                        EventKind::Observation,
                        id,
                    )
                    .unwrap(),
                ))
                .await
                .unwrap();
        }
        Self {
            store,
            pool,
            ids: AtomicU64::new(0),
            _dir: dir,
        }
    }
    async fn records(&self, namespace: Namespace) -> Vec<ReflectionReadRecord> {
        self.store
            .query_reflection_records(ReflectionRecordQuery {
                scope: MemoryScope::for_namespace(namespace),
                reflection_reference: None,
                limit: 100,
            })
            .await
            .unwrap()
    }
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
        self.store.begin_reflection_transaction().await
    }
}
#[async_trait]
impl EventStore for Deps {
    async fn append_event(&self, e: StoredEvent) -> Result<(), AppError> {
        self.store.append_event(e).await
    }
    async fn list_event_references(&self) -> Result<Vec<String>, AppError> {
        self.store.list_event_references().await
    }
    async fn list_recorded_at_for_snapshot_manifest(
        &self,
        s: &MemoryScope,
        m: &[EventReference],
    ) -> Result<Vec<DateTime<Utc>>, AppError> {
        self.store
            .list_recorded_at_for_snapshot_manifest(s, m)
            .await
    }
    async fn query_evidence_event_ids(&self, q: EvidenceQuery) -> Result<Vec<String>, AppError> {
        self.store.query_evidence_event_ids(q).await
    }
    async fn query_evidence_event_ids_unbounded(
        &self,
        q: EvidenceQuery,
    ) -> Result<Vec<String>, AppError> {
        self.store.query_evidence_event_ids_unbounded(q).await
    }
    async fn has_event(&self, id: &str) -> Result<bool, AppError> {
        self.store.has_event(id).await
    }
}
fn scoped_record(namespace: Namespace, evidence: &[&str]) -> ReflectionInput {
    ReflectionInput::record_only(
        Reflection::new("bounded record-only history"),
        evidence.iter().map(|s| s.to_string()).collect(),
    )
    .with_origin_scope(MemoryScope::for_namespace(namespace))
}

#[tokio::test]
async fn targetless_record_is_queryable_only_with_verified_origin_and_durable_evidence() {
    let deps = Deps::new().await;
    let result = run_reflection::execute(&deps, scoped_record(Namespace::for_project("a"), &["a"]))
        .await
        .unwrap();
    let records = deps.records(Namespace::for_project("a")).await;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reflection_id, result.reflection_id);
    assert_eq!(records[0].scope.status, ReflectionScopeStatus::Verified);
    assert_eq!(
        records[0].scope.origin_scopes,
        vec![MemoryScope::for_namespace(Namespace::for_project("a"))]
    );
    assert!(records[0].scope.affected_scopes.is_empty());
    assert_eq!(
        records[0].provenance.supporting_evidence_event_references[0].event_id(),
        "a"
    );
    assert!(deps.records(Namespace::for_project("b")).await.is_empty());
    assert!(deps.records(Namespace::self_()).await.is_empty());
    // Stale JSON is not authoritative after normalization.
    sqlx::query("UPDATE reflections SET supporting_evidence_event_ids = '[\"b\"]'")
        .execute(&deps.pool)
        .await
        .unwrap();
    assert_eq!(
        deps.records(Namespace::for_project("a")).await[0]
            .provenance
            .supporting_evidence_event_references[0]
            .event_id(),
        "a"
    );
    // Read authorization is rechecked against source scope even after persistence.
    sqlx::query("UPDATE events SET namespace = 'project/b' WHERE event_id = 'a'")
        .execute(&deps.pool)
        .await
        .unwrap();
    assert!(deps.records(Namespace::for_project("a")).await.is_empty());
    assert!(deps.records(Namespace::for_project("b")).await.is_empty());
}

#[tokio::test]
async fn targetless_explicit_scope_rejects_missing_mixed_and_unknown_owner_sources() {
    let deps = Deps::new().await;
    for evidence in [vec![], vec!["a", "b"], vec!["missing"]] {
        assert!(
            run_reflection::execute(&deps, scoped_record(Namespace::for_project("a"), &evidence))
                .await
                .is_err()
        );
    }
    sqlx::query("UPDATE events SET owner = 'unknown' WHERE event_id = 'a'")
        .execute(&deps.pool)
        .await
        .unwrap();
    assert!(
        run_reflection::execute(&deps, scoped_record(Namespace::for_project("a"), &["a"]))
            .await
            .is_err()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM reflections")
            .fetch_one(&deps.pool)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn project_origin_global_effect_stays_hidden_without_revoking_existing_internal_mutation() {
    let deps = Deps::new().await;
    run_reflection::execute(
        &deps,
        scoped_record(Namespace::for_project("a"), &["a"])
            .with_identity_update(vec!["global identity".into()]),
    )
    .await
    .unwrap();
    assert_eq!(
        deps.store.load_identity().await.unwrap().canonical_claims(),
        &["global identity"]
    );
    for namespace in [
        Namespace::for_project("a"),
        Namespace::for_project("b"),
        Namespace::self_(),
    ] {
        assert!(deps.records(namespace.clone()).await.is_empty());
        assert!(
            deps.store
                .query_self_model_history(SelfModelHistoryQuery {
                    scope: MemoryScope::for_namespace(namespace),
                    history_kind: SelfModelHistoryKind::Identity,
                    limit: 100
                })
                .await
                .unwrap()
                .records
                .is_empty()
        );
    }
    let scopes: Vec<(String, String)> =
        sqlx::query_as("SELECT role, namespace FROM reflection_scopes ORDER BY role")
            .fetch_all(&deps.pool)
            .await
            .unwrap();
    assert_eq!(
        scopes,
        vec![
            ("affected".into(), "self".into()),
            ("origin".into(), "project/a".into())
        ]
    );
}

#[tokio::test]
async fn project_correction_affects_project_and_mixed_legacy_self_read_does_not_widen() {
    let deps = Deps::new().await;
    for (id, namespace) in [
        ("project", Namespace::for_project("a")),
        ("self-claim", Namespace::self_()),
    ] {
        deps.store
            .upsert_claim(StoredClaim::new(
                id.into(),
                ClaimDraft::new_with_namespace(
                    namespace.derived_owner(),
                    namespace,
                    "x",
                    "is",
                    "old",
                    Mode::Observed,
                ),
                ClaimStatus::Active,
            ))
            .await
            .unwrap();
    }
    run_reflection::execute(
        &deps,
        ReflectionInput::new(
            Reflection::new("project correction"),
            "project",
            None,
            vec![],
        ),
    )
    .await
    .unwrap();
    let record = deps.records(Namespace::for_project("a")).await.remove(0);
    assert_eq!(
        record.scope.affected_scopes,
        vec![MemoryScope::for_namespace(Namespace::for_project("a"))]
    );
    // The existing self-governance/world-evidence exception is unchanged.
    run_reflection::execute(
        &deps,
        ReflectionInput::new(
            Reflection::new("legacy global governance"),
            "self-claim",
            None,
            vec!["world".into()],
        )
        .with_identity_update(vec!["identity retained".into()]),
    )
    .await
    .unwrap();
    assert_eq!(deps.records(Namespace::self_()).await.len(), 1);
    assert_eq!(
        deps.records(Namespace::self_()).await[0].scope.status,
        ReflectionScopeStatus::Unknown
    );
    assert!(deps.records(Namespace::world()).await.is_empty());
}

#[tokio::test]
async fn evidence_relation_failure_rolls_back_global_mutations_reflection_and_audit() {
    let deps = Deps::new().await;
    let baseline = deps.store.load_identity().await.unwrap();
    sqlx::query("CREATE TRIGGER fail_reflection_evidence BEFORE INSERT ON reflection_evidence BEGIN SELECT RAISE(ABORT, 'test relation failure'); END").execute(&deps.pool).await.unwrap();
    let input = scoped_record(Namespace::self_(), &["self"])
        .with_identity_update(vec!["must roll back".into()]);
    assert!(run_reflection::execute(&deps, input).await.is_err());
    assert_eq!(deps.store.load_identity().await.unwrap(), baseline);
    for table in [
        "reflections",
        "reflection_scopes",
        "reflection_evidence",
        "operation_log",
    ] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&deps.pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table}");
    }
}

#[test]
fn mcp_targetless_scope_metadata_grants_no_global_mutation_entrypoint() {
    let base = serde_json::json!({"reflection":{"summary":"record"},"origin_namespace":"project/a","replacement_evidence_event_ids":["a"]});
    let valid: RunReflectionParams = serde_json::from_value(base.clone()).unwrap();
    assert!(ReflectionInput::try_from(valid).is_ok());
    for update in [
        serde_json::json!({"identity_update":{"canonical_claims":["bad"]}}),
        serde_json::json!({"commitment_updates":[]}),
        serde_json::json!({"replacement_claim":{"owner":"World","subject":"x","predicate":"is","object":"y","mode":"Observed"}}),
    ] {
        let mut value = base.clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(update.as_object().unwrap().clone());
        let result = serde_json::from_value::<RunReflectionParams>(value)
            .map_err(|e| e.to_string())
            .and_then(|dto| ReflectionInput::try_from(dto).map_err(|e| e.to_string()));
        assert!(result.is_err());
    }
    let no_scope = serde_json::json!({"reflection":{"summary":"record"},"replacement_evidence_event_ids":["a"]});
    assert!(
        ReflectionInput::try_from(serde_json::from_value::<RunReflectionParams>(no_scope).unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn existing_internal_self_patch_has_scope_bounded_targetless_history() {
    let deps = Deps::new().await;
    let result = run_reflection::execute(
        &deps,
        scoped_record(Namespace::self_(), &["self"])
            .with_identity_update(vec!["bounded self identity".into()]),
    )
    .await
    .unwrap();
    let records = deps
        .store
        .query_self_model_history(SelfModelHistoryQuery {
            scope: MemoryScope::self_(),
            history_kind: SelfModelHistoryKind::Identity,
            limit: 100,
        })
        .await
        .unwrap()
        .records;
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].reflection_id, result.reflection_id);
    assert_eq!(records[0].scope.origin_scopes, vec![MemoryScope::self_()]);
    assert_eq!(records[0].scope.affected_scopes, vec![MemoryScope::self_()]);
    assert!(records[0].superseded_claim_reference.is_none());
    assert!(deps.records(Namespace::world()).await.is_empty());
    assert!(deps.records(Namespace::for_project("a")).await.is_empty());
}

#[tokio::test]
async fn standalone_reflection_append_enforces_evidence_fk_atomically() {
    let deps = Deps::new().await;
    let reflection = StoredReflection::new(
        "orphan-new-write".into(),
        Utc::now(),
        Reflection::new("missing evidence must fail"),
        None,
        None,
    )
    .with_supporting_evidence_event_ids(vec!["missing".into()]);
    assert!(deps.store.append_reflection(reflection).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM reflections")
            .fetch_one(&deps.pool)
            .await
            .unwrap(),
        0
    );
}
