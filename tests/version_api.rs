use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::get_self_model_versions::{self, GetSelfModelVersionsInput},
    application::run_reflection::{self, ReflectionInput},
    domain::commitment::Commitment,
    domain::{
        event::{Event, EventReference},
        reflection::Reflection,
        types::{EventKind, MemoryScope, Namespace},
    },
    error::AppError,
    interfaces::mcp::dto::{GetSelfModelVersionsParams, RunReflectionParams},
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
        let url = format!(
            "sqlite://{}",
            dir.path()
                .join("scope.sqlite")
                .to_string_lossy()
                .replace('\\', "/")
        );
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

async fn versions(
    deps: &Deps,
    namespace: &str,
    limit: usize,
    before_version: Option<u64>,
) -> get_self_model_versions::GetSelfModelVersionsResult {
    get_self_model_versions::execute(
        &deps.store,
        GetSelfModelVersionsInput {
            namespace: Namespace::parse(namespace).unwrap(),
            allow_global_version_metadata: true,
            limit,
            before_version,
        },
    )
    .await
    .unwrap()
}
async fn identity(deps: &Deps, namespace: &str, evidence: &str, claims: &[&str]) {
    run_reflection::execute(
        deps,
        scoped_record(Namespace::parse(namespace).unwrap(), &[evidence])
            .with_identity_update(claims.iter().map(|claim| claim.to_string()).collect()),
    )
    .await
    .unwrap();
}

#[test]
fn versions_require_explicit_global_metadata_opt_in_and_bounded_limit() {
    for value in [
        serde_json::json!({"namespace":"project/a"}),
        serde_json::json!({"namespace":"project/a", "allow_global_version_metadata":false}),
        serde_json::json!({"namespace":"project/a", "allow_global_version_metadata":true, "limit":0}),
        serde_json::json!({"namespace":"project/a", "allow_global_version_metadata":true, "limit":101}),
        serde_json::json!({"namespace":"project/a", "allow_global_version_metadata":true, "before_version":u64::MAX}),
    ] {
        let params: GetSelfModelVersionsParams = serde_json::from_value(value).unwrap();
        assert!(GetSelfModelVersionsInput::try_from(params).is_err());
    }
    let params: GetSelfModelVersionsParams = serde_json::from_value(serde_json::json!({
        "namespace":"project/a", "allow_global_version_metadata":true
    }))
    .unwrap();
    assert_eq!(
        GetSelfModelVersionsInput::try_from(params).unwrap().limit,
        20
    );
}

#[test]
fn omitted_new_reflection_fields_preserve_legacy_typed_payload_bytes() {
    let old = r#"{"reflection":{"summary":"old"},"supersede_claim_id":"claim","origin_namespace":null,"replacement_claim":null,"replacement_evidence_event_ids":[],"replacement_evidence_query":null,"identity_update":null,"commitment_updates":null}"#;
    let params: RunReflectionParams = serde_json::from_str(old).unwrap();
    assert_eq!(serde_json::to_string(&params).unwrap(), old);
    let mut value = serde_json::from_str::<serde_json::Value>(old).unwrap();
    value["expected_self_model_version"] = 8.into();
    value["request_id"] = "idempotent-rollback".into();
    value["origin_namespace"] = "project/a".into();
    value["self_model_rollback"] =
        serde_json::json!({"target_version":1,"components":["identity"],"confirm":true});
    let converted =
        ReflectionInput::try_from(serde_json::from_value::<RunReflectionParams>(value).unwrap())
            .unwrap();
    let encoded = serde_json::to_value(converted).unwrap();
    assert_eq!(encoded["expected_self_model_version"], 8);
    assert_eq!(encoded["self_model_rollback"]["target_version"], 1);
}

#[tokio::test]
async fn scoped_versions_hide_baselines_and_cross_scope_previous_values_preserving_duplicates() {
    let deps = Deps::new().await;
    let baseline = versions(&deps, "project/a", 20, None).await;
    assert_eq!(baseline.current_version, 0);
    assert!(baseline.records.is_empty());
    identity(&deps, "project/a", "a", &["a", "a", "b"]).await;
    identity(&deps, "project/a", "a", &["a", "b", "a"]).await;
    identity(&deps, "project/b", "b", &["B-SECRET"]).await;
    identity(&deps, "project/a", "a", &["A-new"]).await;
    let page = versions(&deps, "project/a", 20, None).await;
    assert_eq!(page.current_version, 4);
    assert_eq!(
        page.records
            .iter()
            .map(|record| record.version)
            .collect::<Vec<_>>(),
        vec![4, 2, 1]
    );
    let latest = page.records[0].identity_update.as_ref().unwrap();
    assert!(latest.changed);
    assert!(latest.previous_patch.is_none());
    assert!(latest.previous_values_redacted.is_some());
    assert_eq!((latest.previous_count, latest.current_count), (1, 1));
    let ordered = page.records[1].identity_update.as_ref().unwrap();
    assert!(ordered.changed);
    assert_eq!(ordered.patch.canonical_claims, ["a", "b", "a"]);
    assert_eq!(
        ordered.previous_patch.as_ref().unwrap().canonical_claims,
        ["a", "a", "b"]
    );
    assert!(ordered.previous_values_redacted.is_none());
    assert!(!serde_json::to_string(&page).unwrap().contains("B-SECRET"));
    let first = versions(&deps, "project/a", 1, None).await;
    assert_eq!(first.next_before_version, Some(4));
    assert!(first.has_more);
    let next = versions(&deps, "project/a", 1, first.next_before_version).await;
    assert_eq!(next.records[0].version, 2);
    let last = versions(&deps, "project/a", 1, next.next_before_version).await;
    assert_eq!(last.records[0].version, 1);
    assert!(!last.has_more);
    assert_eq!(last.next_before_version, None);
    assert!(
        versions(&deps, "project/a", 20, Some(0))
            .await
            .records
            .is_empty()
    );
}

#[tokio::test]
async fn commitment_only_versions_never_expose_inherited_identity() {
    let deps = Deps::new().await;
    identity(&deps, "project/a", "a", &["A-PRIVATE-IDENTITY"]).await;
    run_reflection::execute(
        &deps,
        scoped_record(Namespace::for_project("b"), &["b"]).with_commitment_updates(vec![
            Commitment::new(agent_llm_mm::domain::types::Owner::Self_, "B commitment"),
        ]),
    )
    .await
    .unwrap();
    let b = versions(&deps, "project/b", 20, None).await;
    assert_eq!(b.records.len(), 1);
    assert!(b.records[0].identity_update.is_none());
    assert!(b.records[0].commitment_updates.is_some());
    assert!(
        !serde_json::to_string(&b)
            .unwrap()
            .contains("A-PRIVATE-IDENTITY")
    );
    let a = versions(&deps, "project/a", 20, None).await;
    assert_eq!(a.records.len(), 1);
    assert_eq!(a.records[0].version, 1);
}

#[tokio::test]
async fn source_scope_and_durable_evidence_are_rechecked_and_drift_errors_are_generic() {
    let deps = Deps::new().await;
    identity(&deps, "project/a", "a", &["A-visible"]).await;
    assert_eq!(
        versions(&deps, "project/a", 20, None).await.records.len(),
        1
    );
    sqlx::query("UPDATE reflections SET scope_status = 'legacy_unambiguous'")
        .execute(&deps.pool)
        .await
        .unwrap();
    assert!(
        versions(&deps, "project/a", 20, None)
            .await
            .records
            .is_empty()
    );
    sqlx::query("UPDATE reflections SET scope_status = 'verified'")
        .execute(&deps.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE events SET namespace = 'project/b' WHERE event_id = 'a'")
        .execute(&deps.pool)
        .await
        .unwrap();
    assert!(
        versions(&deps, "project/a", 20, None)
            .await
            .records
            .is_empty()
    );
    assert!(
        versions(&deps, "project/b", 20, None)
            .await
            .records
            .is_empty()
    );
    sqlx::query("UPDATE events SET namespace = 'project/a' WHERE event_id = 'a'")
        .execute(&deps.pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM reflection_evidence")
        .execute(&deps.pool)
        .await
        .unwrap();
    assert!(
        versions(&deps, "project/a", 20, None)
            .await
            .records
            .is_empty()
    );
    sqlx::query("UPDATE identity_claims SET claim = 'SECRET-DRIFT'")
        .execute(&deps.pool)
        .await
        .unwrap();
    let error = get_self_model_versions::execute(
        &deps.store,
        GetSelfModelVersionsInput {
            namespace: Namespace::for_project("a"),
            allow_global_version_metadata: true,
            limit: 20,
            before_version: None,
        },
    )
    .await
    .unwrap_err()
    .to_string();
    assert_eq!(
        error,
        "self-model version read unavailable: stored state failed validation"
    );
    assert!(!error.contains("SECRET-DRIFT"));
}

#[tokio::test]
async fn mixed_or_unknown_provenance_is_not_promoted_by_current_values() {
    let deps = Deps::new().await;
    run_reflection::execute(
        &deps,
        ReflectionInput::record_only(Reflection::new("mixed"), vec!["a".into(), "b".into()])
            .with_identity_update(vec!["MIXED-SECRET".into()]),
    )
    .await
    .unwrap();
    for namespace in ["project/a", "project/b", "self"] {
        let page = versions(&deps, namespace, 20, None).await;
        assert_eq!(page.current_version, 1);
        assert!(page.records.is_empty());
    }
}
