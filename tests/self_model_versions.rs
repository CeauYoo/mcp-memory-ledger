//! Global snapshots are internal state. Rollback is a new, evidence-backed write,
//! never a rewrite of history or an authorization route around scoped provenance.
use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::run_reflection::{self, ReflectionInput, ReflectionResult},
    domain::{
        claim::ClaimDraft,
        commitment::Commitment,
        event::{Event, EventReference},
        reflection::Reflection,
        self_model_version::{
            SelfModelComponent, SelfModelRollbackRequest, SelfModelVersion, SelfModelVersionKind,
        },
        self_revision::TriggerType,
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    interfaces::mcp::dto::RunReflectionParams,
    ports::*,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
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
                .join("versions.sqlite")
                .to_string_lossy()
                .replace('\\', "/")
        );
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        // Do not seed global projections after bootstrap: baseline 0 must describe
        // precisely the initialized database, and drift is deliberately fail-closed.
        for (name, namespace) in [
            ("self", Namespace::self_()),
            ("a", Namespace::for_project("a")),
            ("b", Namespace::for_project("b")),
            ("world", Namespace::world()),
        ] {
            for suffix in ["1", "2", "new"] {
                store
                    .append_event(StoredEvent::new(
                        format!("{name}-{suffix}"),
                        Utc::now(),
                        Event::new_with_namespace(
                            namespace.derived_owner(),
                            namespace.clone(),
                            EventKind::Observation,
                            format!("{name} evidence {suffix}"),
                        )
                        .unwrap(),
                    ))
                    .await
                    .unwrap();
            }
            store
                .upsert_claim(StoredClaim::new(
                    format!("anchor-{name}"),
                    ClaimDraft::new_with_namespace(
                        namespace.derived_owner(),
                        namespace,
                        "setting",
                        "is",
                        "old",
                        Mode::Observed,
                    ),
                    ClaimStatus::Active,
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
    async fn head(&self) -> SelfModelVersion {
        let mut tx = self.store.begin_reflection_transaction().await.unwrap();
        let head = tx.load_current_self_model_version().await.unwrap();
        tx.commit().await.unwrap();
        head
    }
    async fn version(&self, number: u64) -> Option<SelfModelVersion> {
        let mut tx = self.store.begin_reflection_transaction().await.unwrap();
        let version = tx.load_self_model_version(number).await.unwrap();
        tx.commit().await.unwrap();
        version
    }
    async fn counts(&self) -> (i64, i64, i64, i64, i64) {
        sqlx::query_as("SELECT (SELECT count(*) FROM reflections), (SELECT count(*) FROM claims), (SELECT count(*) FROM evidence_links), (SELECT count(*) FROM reflection_trigger_ledger), (SELECT count(*) FROM operation_log WHERE actor_id='durable_write_receipt_v1')")
            .fetch_one(&self.pool).await.unwrap()
    }
    async fn status(&self, id: &str) -> String {
        sqlx::query_scalar("SELECT status FROM claims WHERE claim_id=?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
    async fn write(
        &self,
        scope: &str,
        identity: Option<&str>,
        commitment: Option<&str>,
    ) -> ReflectionResult {
        let mut input = update(scope);
        if let Some(value) = identity {
            input = input.with_identity_update(vec![value.into()]);
        }
        if let Some(value) = commitment {
            input = input.with_commitment_updates(vec![Commitment::new(Owner::Self_, value)]);
        }
        run_reflection::execute(self, input).await.unwrap()
    }
    async fn prepare_rollback(&self) {
        self.write("self", Some("identity-one"), Some("commitment-one"))
            .await;
        self.write("self", Some("identity-two"), Some("commitment-two"))
            .await;
    }
}
#[async_trait]
impl IdGenerator for Deps {
    async fn next_id(&self) -> Result<String, AppError> {
        Ok(format!(
            "version-reflection-{}",
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
fn namespace(scope: &str) -> Namespace {
    if scope == "self" {
        Namespace::self_()
    } else {
        Namespace::for_project(scope)
    }
}
fn update(scope: &str) -> ReflectionInput {
    ReflectionInput::new(
        Reflection::new("Evidence-backed global update"),
        format!("anchor-{scope}"),
        None,
        vec![format!("{scope}-1")],
    )
    .with_origin_scope(MemoryScope::for_namespace(namespace(scope)))
}
fn rollback_json(scope: &str, target: u64, expected: u64, components: &[&str], key: &str) -> Value {
    json!({
        "reflection": {"summary":"Restore the selected prior global component"},
        "supersede_claim_id": format!("anchor-{scope}"),
        "origin_namespace": namespace(scope).as_str(),
        "replacement_claim": null,
        "replacement_evidence_event_ids": [format!("{scope}-new")],
        "expected_self_model_version": expected,
        "self_model_rollback": {"target_version": target, "components": components, "confirm": true},
        "request_id": key
    })
}
fn from_json(value: Value) -> Result<ReflectionInput, AppError> {
    serde_json::from_value::<RunReflectionParams>(value)
        .map_err(|error| AppError::InvalidParams(error.to_string()))?
        .try_into()
}
async fn execute_json(deps: &Deps, value: Value) -> Result<ReflectionResult, AppError> {
    run_reflection::execute(deps, from_json(value)?).await
}

#[tokio::test]
async fn initialization_is_baseline_zero_and_explicit_no_op_global_writes_append() {
    let deps = Deps::new().await;
    let baseline = deps.head().await;
    assert_eq!(baseline.version, 0);
    assert_eq!(baseline.kind, SelfModelVersionKind::InitializationBaseline);
    assert_eq!(baseline.previous_version, None);
    assert_eq!(baseline.reflection_id, None);
    assert!(!baseline.identity_written && !baseline.commitments_written);
    assert_eq!(
        (
            baseline.identity_source_version,
            baseline.commitment_source_version
        ),
        (0, 0)
    );
    assert_eq!(baseline.identity, deps.store.load_identity().await.unwrap());
    assert_eq!(
        baseline.commitments,
        deps.store.list_commitments().await.unwrap()
    );

    let first = deps.write("self", Some("same identity"), None).await;
    let second = deps.write("self", Some("same identity"), None).await;
    let third = deps.write("self", None, Some("same commitment")).await;
    assert_eq!(
        (
            first.self_model_version,
            second.self_model_version,
            third.self_model_version
        ),
        (Some(1), Some(2), Some(3))
    );
    let no_op = deps.version(2).await.unwrap();
    assert_eq!(no_op.identity, deps.version(1).await.unwrap().identity);
    assert!(no_op.identity_written);
    assert_eq!(no_op.identity_source_version, 2);
    let head = deps.head().await;
    assert_eq!(head.previous_version, Some(2));
    assert_eq!(
        (head.identity_source_version, head.commitment_source_version),
        (2, 3)
    );
    assert!(!head.identity_written && head.commitments_written);
    assert_eq!(deps.version(0).await.unwrap(), baseline);
}

#[tokio::test]
async fn claim_only_and_record_only_reflections_never_append_global_versions() {
    let deps = Deps::new().await;
    let baseline = deps.head().await;
    let claim_only = run_reflection::execute(&deps, update("self"))
        .await
        .unwrap();
    let record_only = run_reflection::execute(
        &deps,
        ReflectionInput::record_only(Reflection::new("Record evidence"), vec!["self-1".into()])
            .with_origin_scope(MemoryScope::self_()),
    )
    .await
    .unwrap();
    assert_eq!(claim_only.self_model_version, None);
    assert_eq!(record_only.self_model_version, None);
    assert_eq!(deps.head().await, baseline);
    assert_eq!(deps.version(1).await, None);
    assert!(
        !serde_json::to_string(&claim_only)
            .unwrap()
            .contains("self_model_version")
    );
}

#[tokio::test]
async fn selective_rollback_restores_only_selected_component_and_preserves_history_and_baseline() {
    let deps = Deps::new().await;
    deps.prepare_rollback().await;
    let target = deps.version(1).await.unwrap();
    let before = deps.head().await;
    let result = execute_json(
        &deps,
        rollback_json("self", 1, 2, &["identity"], "rollback-identity"),
    )
    .await
    .unwrap();
    assert_eq!(result.self_model_version, Some(3));
    assert_eq!(result.replacement_claim_id, None);
    assert_eq!(deps.status("anchor-self").await, "disputed");
    let head = deps.head().await;
    assert_eq!(head.kind, SelfModelVersionKind::Rollback);
    assert_eq!(head.previous_version, Some(2));
    assert_eq!(head.rollback_target_version, Some(1));
    assert_eq!(head.identity, target.identity);
    assert_eq!(head.commitments, before.commitments);
    assert_eq!(
        (head.identity_source_version, head.commitment_source_version),
        (3, 2)
    );
    assert_eq!(deps.version(1).await.unwrap(), target);
    assert_eq!(deps.version(2).await.unwrap(), before);

    execute_json(
        &deps,
        rollback_json("self", 1, 3, &["commitments"], "rollback-commitments"),
    )
    .await
    .unwrap();
    let final_head = deps.head().await;
    assert_eq!(final_head.identity, target.identity);
    assert_eq!(final_head.commitments, target.commitments);
    assert!(
        final_head
            .commitments
            .iter()
            .any(|entry| entry.description() == "forbid:write_identity_core_directly")
    );
    assert_eq!(
        (
            final_head.identity_source_version,
            final_head.commitment_source_version
        ),
        (3, 4)
    );
}

#[tokio::test]
async fn rollback_with_replacement_keeps_normal_claim_supersession_and_evidence_links() {
    let deps = Deps::new().await;
    deps.prepare_rollback().await;
    let mut request = rollback_json(
        "self",
        1,
        2,
        &["identity", "commitments"],
        "replacement-rollback",
    );
    request["replacement_claim"] = json!({"owner":"Self_", "namespace":"self", "subject":"setting", "predicate":"is", "object":"restored", "mode":"Observed"});
    let result = execute_json(&deps, request).await.unwrap();
    assert_eq!(deps.status("anchor-self").await, "superseded");
    let replacement = result.replacement_claim_id.unwrap();
    assert_eq!(deps.status(&replacement).await, "active");
    let links: Vec<String> =
        sqlx::query_scalar("SELECT event_id FROM evidence_links WHERE claim_id=?")
            .bind(&replacement)
            .fetch_all(&deps.pool)
            .await
            .unwrap();
    assert_eq!(links, vec!["self-new"]);
    assert_eq!(
        deps.head().await.identity,
        deps.version(1).await.unwrap().identity
    );
}

#[tokio::test]
async fn rollback_replay_precedes_stale_head_and_claim_guards_and_payload_changes_conflict() {
    let deps = Deps::new().await;
    deps.prepare_rollback().await;
    let mut request = rollback_json("self", 1, 2, &["identity"], "durable-replay");
    request["replacement_claim"] = json!({"owner":"Self_", "namespace":"self", "subject":"setting", "predicate":"is", "object":"restored", "mode":"Observed"});
    let first = execute_json(&deps, request.clone()).await.unwrap();
    let counts = deps.counts().await;
    let head = deps.head().await;
    assert_eq!(deps.status("anchor-self").await, "superseded");
    let replay = execute_json(&deps, request.clone()).await.unwrap();
    assert_eq!(replay, first);
    assert_eq!(
        serde_json::to_vec(&replay).unwrap(),
        serde_json::to_vec(&first).unwrap()
    );
    assert_eq!(deps.counts().await, counts);
    assert_eq!(deps.head().await, head);
    request["reflection"]["summary"] = json!("Different request under same durable key");
    assert!(matches!(
        execute_json(&deps, request).await,
        Err(AppError::InvalidParams(_))
    ));
    assert_eq!(deps.counts().await, counts);
    assert_eq!(deps.head().await, head);
}

#[tokio::test]
async fn rollback_requires_confirmation_expected_version_key_scope_target_and_new_evidence() {
    let deps = Deps::new().await;
    deps.prepare_rollback().await;
    let counts = deps.counts().await;
    let head = deps.head().await;
    let base = rollback_json("self", 1, 2, &["identity"], "invalid");
    for field in [
        "expected_self_model_version",
        "request_id",
        "origin_namespace",
        "supersede_claim_id",
    ] {
        let mut request = base.clone();
        request.as_object_mut().unwrap().remove(field);
        assert!(
            execute_json(&deps, request).await.is_err(),
            "missing {field} must fail"
        );
    }
    for (field, value) in [
        ("replacement_evidence_event_ids", json!([])),
        ("replacement_evidence_event_ids", json!(["missing"])),
        ("replacement_evidence_event_ids", json!(["world-new"])),
        ("identity_update", json!({"canonical_claims":["smuggled"]})),
        ("commitment_updates", json!([])),
        ("expected_self_model_version", json!(1)),
        ("origin_namespace", json!("project/a")),
    ] {
        let mut request = base.clone();
        request[field] = value;
        assert!(
            execute_json(&deps, request).await.is_err(),
            "invalid {field} must fail"
        );
    }
    for change in [
        json!({"target_version":1,"components":["identity"],"confirm":false}),
        json!({"target_version":1,"components":[],"confirm":true}),
        json!({"target_version":1,"components":["identity","identity"],"confirm":true}),
    ] {
        let mut request = base.clone();
        request["self_model_rollback"] = change;
        assert!(execute_json(&deps, request).await.is_err());
    }
    assert_eq!(deps.counts().await, counts);
    assert_eq!(deps.head().await, head);
}

#[tokio::test]
async fn rollback_rejects_baseline_current_future_missing_and_inherited_target_versions() {
    let deps = Deps::new().await;
    deps.write("self", Some("first identity"), None).await;
    deps.write("self", None, Some("second commitments")).await;
    deps.write("self", Some("third identity"), None).await;
    let counts = deps.counts().await;
    for target in [0, 2, 3, 4, 999] {
        assert!(
            execute_json(
                &deps,
                rollback_json(
                    "self",
                    target,
                    3,
                    &["identity"],
                    &format!("target-{target}")
                )
            )
            .await
            .is_err()
        );
    }
    assert_eq!(deps.head().await.version, 3);
    assert_eq!(deps.counts().await, counts);
}

#[tokio::test]
async fn rollback_checks_component_source_scope_even_when_latest_reflection_matches_scope() {
    let deps = Deps::new().await;
    deps.write("a", Some("project-a identity"), None).await;
    deps.write("b", Some("project-b identity"), None).await;
    deps.write("a", None, Some("project-a commitment")).await;
    let head = deps.head().await;
    assert_eq!(head.identity_source_version, 2);
    assert!(
        execute_json(
            &deps,
            rollback_json("a", 1, 3, &["identity"], "foreign-current-source")
        )
        .await
        .is_err()
    );
    assert_eq!(deps.head().await, head);
}

#[tokio::test]
async fn rollback_accepts_same_scope_inherited_current_component_source() {
    let deps = Deps::new().await;
    deps.write("self", Some("first"), None).await;
    deps.write("self", Some("second"), None).await;
    deps.write("self", None, Some("untouched")).await;
    execute_json(
        &deps,
        rollback_json("self", 1, 3, &["identity"], "inherited-source"),
    )
    .await
    .unwrap();
    let head = deps.head().await;
    assert_eq!(head.identity.canonical_claims(), &["first"]);
    assert_eq!(head.commitment_source_version, 3);
}

#[tokio::test]
async fn rollback_rejects_foreign_target_and_unknown_or_missing_source_evidence() {
    let deps = Deps::new().await;
    deps.write("a", Some("foreign target"), None).await;
    deps.write("self", Some("self current"), None).await;
    assert!(
        execute_json(
            &deps,
            rollback_json("self", 1, 2, &["identity"], "foreign-target")
        )
        .await
        .is_err()
    );

    for mutation in [
        "UPDATE events SET owner='unknown', namespace='world' WHERE event_id='self-1'",
        "UPDATE events SET namespace='project/b', owner='world' WHERE event_id='self-1'",
        "DELETE FROM reflection_evidence",
    ] {
        let local = Deps::new().await;
        local.prepare_rollback().await;
        sqlx::query(mutation).execute(&local.pool).await.unwrap();
        let counts = local.counts().await;
        assert!(
            execute_json(
                &local,
                rollback_json("self", 1, 2, &["identity"], "invalid-provenance")
            )
            .await
            .is_err(),
            "must revalidate durable evidence: {mutation}"
        );
        assert_eq!(local.counts().await, counts);
        assert_eq!(local.head().await.version, 2);
    }
}

#[tokio::test]
async fn unverified_mixed_legacy_source_cannot_be_used_as_a_rollback_target() {
    let deps = Deps::new().await;
    run_reflection::execute(
        &deps,
        ReflectionInput::new(
            Reflection::new("Legacy mixed source"),
            "anchor-self",
            None,
            vec!["world-1".into()],
        )
        .with_identity_update(vec!["legacy mixed".into()]),
    )
    .await
    .unwrap();
    deps.write("self", Some("verified current"), None).await;
    assert!(
        execute_json(
            &deps,
            rollback_json("self", 1, 2, &["identity"], "unknown-target")
        )
        .await
        .is_err()
    );
    assert_eq!(deps.head().await.version, 2);
}

#[tokio::test]
async fn concurrent_expected_version_writers_have_exactly_one_success() {
    let deps = Deps::new().await;
    let first = update("self")
        .with_identity_update(vec!["one".into()])
        .with_expected_self_model_version(0);
    let second = update("self")
        .with_identity_update(vec!["two".into()])
        .with_expected_self_model_version(0);
    let (a, b) = tokio::join!(
        run_reflection::execute(&deps, first),
        run_reflection::execute(&deps, second)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(deps.head().await.version, 1);
    assert_eq!(deps.counts().await.0, 1);
    assert_eq!(deps.counts().await.4, 1);
}

#[tokio::test]
async fn direct_projection_drift_blocks_global_writes_without_rebaselining() {
    for mutation in [
        "INSERT INTO identity_claims (position,claim) VALUES (999,'out-of-band')",
        "INSERT INTO commitments (description,owner) VALUES ('out-of-band','self')",
    ] {
        let deps = Deps::new().await;
        let baseline = deps.head().await;
        sqlx::query(mutation).execute(&deps.pool).await.unwrap();
        let counts = deps.counts().await;
        assert!(
            run_reflection::execute(
                &deps,
                update("self").with_identity_update(vec!["must not hide drift".into()])
            )
            .await
            .is_err()
        );
        assert_eq!(deps.version(0).await.unwrap(), baseline);
        assert_eq!(deps.version(1).await, None);
        assert_eq!(deps.counts().await, counts);
    }
}

#[test]
fn absent_version_fields_preserve_legacy_dto_input_and_result_bytes() {
    let legacy_dto = r#"{"reflection":{"summary":"legacy"},"supersede_claim_id":"anchor-self","origin_namespace":null,"replacement_claim":null,"replacement_evidence_event_ids":[],"replacement_evidence_query":null,"identity_update":null,"commitment_updates":null}"#;
    let params: RunReflectionParams = serde_json::from_str(legacy_dto).unwrap();
    assert_eq!(serde_json::to_string(&params).unwrap(), legacy_dto);
    let legacy_input = r#"{"reflection":{"summary":"legacy"},"target_claim_id":"anchor-self","replacement_claim":null,"replacement_evidence_event_ids":[],"replacement_evidence_query":null,"identity_update":null,"commitment_updates":null,"handled_trigger_ledger_entry":null,"strict_evidence_scope":false}"#;
    let input: ReflectionInput = serde_json::from_str(legacy_input).unwrap();
    assert_eq!(serde_json::to_string(&input).unwrap(), legacy_input);
    let legacy_result = r#"{"reflection_id":"legacy-result","replacement_claim_id":null}"#;
    let result: ReflectionResult = serde_json::from_str(legacy_result).unwrap();
    assert_eq!(result.self_model_version, None);
    assert_eq!(serde_json::to_string(&result).unwrap(), legacy_result);
    let receipt = StoredWriteReceipt {
        request_hash: "unchanged".into(),
        result_json: legacy_result.into(),
    };
    let request = WriteReceiptRequest {
        operation_id: "legacy-operation".into(),
        namespace: "self".into(),
        operation: "run_reflection".into(),
        request_hash: "unchanged".into(),
    };
    assert_eq!(
        receipt.replay::<ReflectionResult>(&request).unwrap(),
        result
    );
}

#[test]
fn rollback_components_are_typed_and_unknown_fields_are_rejected() {
    let request = SelfModelRollbackRequest {
        target_version: 1,
        components: vec![SelfModelComponent::Identity],
        confirm: true,
    };
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({"target_version":1,"components":["identity"],"confirm":true})
    );
    assert!(
        serde_json::from_value::<SelfModelRollbackRequest>(
            json!({"target_version":1,"components":["global"],"confirm":true})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<SelfModelRollbackRequest>(
            json!({"target_version":1,"components":["identity"],"confirm":true,"force":true})
        )
        .is_err()
    );
}

#[tokio::test]
async fn version_reflection_trigger_receipt_and_commit_failures_roll_back_every_write() {
    for rollback in [false, true] {
        for stage in ["version", "reflection", "trigger", "receipt", "commit"] {
            let deps = Deps::new().await;
            if rollback {
                deps.prepare_rollback().await;
            }
            let before = deps.head().await;
            let counts = deps.counts().await;
            let claim_status = deps.status("anchor-self").await;
            let injection = match stage {
                "version" => {
                    "CREATE TRIGGER fail_version BEFORE INSERT ON self_model_versions WHEN NEW.version > 0 BEGIN SELECT RAISE(ABORT,'injected version failure'); END"
                }
                "reflection" => {
                    "CREATE TRIGGER fail_reflection BEFORE INSERT ON reflections BEGIN SELECT RAISE(ABORT,'injected reflection failure'); END"
                }
                "trigger" => {
                    "CREATE TRIGGER fail_trigger BEFORE INSERT ON reflection_trigger_ledger BEGIN SELECT RAISE(ABORT,'injected trigger failure'); END"
                }
                "receipt" => {
                    "CREATE TRIGGER fail_receipt BEFORE INSERT ON operation_log WHEN NEW.actor_id='durable_write_receipt_v1' BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END"
                }
                "commit" => {
                    // A deferred foreign-key error fires at COMMIT, after every
                    // application write and receipt have already succeeded.
                    sqlx::query("CREATE TABLE commit_parent (id INTEGER PRIMARY KEY)")
                        .execute(&deps.pool)
                        .await
                        .unwrap();
                    sqlx::query("CREATE TABLE commit_failure (parent INTEGER REFERENCES commit_parent(id) DEFERRABLE INITIALLY DEFERRED)").execute(&deps.pool).await.unwrap();
                    "CREATE TRIGGER fail_commit AFTER INSERT ON self_model_versions WHEN NEW.version > 0 BEGIN INSERT INTO commit_failure(parent) VALUES (1); END"
                }
                _ => unreachable!(),
            };
            sqlx::query(injection).execute(&deps.pool).await.unwrap();
            let mut request = if rollback {
                rollback_json(
                    "self",
                    1,
                    2,
                    &["identity", "commitments"],
                    "atomic-rollback",
                )
            } else {
                json!({"reflection":{"summary":"Atomic global update"}, "supersede_claim_id":"anchor-self", "origin_namespace":"self", "replacement_evidence_event_ids":["self-new"], "expected_self_model_version":0, "request_id":"atomic-update", "identity_update":{"canonical_claims":["changed"]}, "commitment_updates":[{"owner":"Self_","description":"changed"}]})
            };
            request["replacement_claim"] = json!({"owner":"Self_", "namespace":"self", "subject":"setting", "predicate":"is", "object":"replacement", "mode":"Observed"});
            let input = from_json(request)
                .unwrap()
                .with_handled_trigger_ledger_entry(StoredTriggerLedgerEntry::new(
                    "atomic-trigger",
                    TriggerType::Failure,
                    Namespace::self_(),
                    "atomic-key",
                    TriggerLedgerStatus::Handled,
                ));
            let error = run_reflection::execute(&deps, input)
                .await
                .expect_err(stage);
            if stage == "commit" {
                assert!(
                    error.to_string().to_lowercase().contains("foreign key"),
                    "commit injection must reach deferred constraint: {error}"
                );
            } else {
                assert!(
                    error.to_string().contains("injected"),
                    "must reach {stage} injection: {error}"
                );
            }
            assert_eq!(deps.head().await, before, "{stage}, rollback={rollback}");
            assert_eq!(deps.counts().await, counts, "{stage}, rollback={rollback}");
            assert_eq!(deps.status("anchor-self").await, claim_status);
            assert_eq!(deps.version(before.version + 1).await, None);
        }
    }
}

#[tokio::test]
async fn scoped_history_never_serializes_foreign_inherited_snapshot_contents() {
    let deps = Deps::new().await;
    deps.write("a", Some("project-a-private-identity"), None)
        .await;
    deps.write("self", None, Some("self-only-commitment")).await;
    deps.write("b", Some("project-b-private-identity"), None)
        .await;
    for scope in [
        Namespace::self_(),
        Namespace::for_project("a"),
        Namespace::for_project("b"),
    ] {
        let export = agent_llm_mm::application::export_memory::export_memory(
            &deps.store,
            agent_llm_mm::domain::ledger_export::ExportMemoryRequest::new(scope.as_str()),
        )
        .await
        .unwrap();
        let exported = serde_json::to_string(&export).unwrap();
        for excluded in [
            "project-a-private-identity",
            "project-b-private-identity",
            "self-only-commitment",
            "identity_json",
            "commitments_json",
            "identity_source_version",
            "commitment_source_version",
        ] {
            assert!(
                !exported.contains(excluded),
                "scoped export must exclude global snapshot content: {excluded}"
            );
        }
        for history_kind in [
            SelfModelHistoryKind::Identity,
            SelfModelHistoryKind::Commitment,
        ] {
            let page = agent_llm_mm::application::get_self_model_history::execute(
                &deps.store,
                agent_llm_mm::application::get_self_model_history::GetSelfModelHistoryInput {
                    namespace: scope.clone(),
                    history_kind,
                    limit: 100,
                },
            )
            .await
            .unwrap();
            let encoded = serde_json::to_string(&page).unwrap();
            assert!(!encoded.contains("identity_json"));
            assert!(!encoded.contains("commitments_json"));
            assert!(!encoded.contains("identity_source_version"));
            if scope != Namespace::for_project("a") {
                assert!(!encoded.contains("project-a-private-identity"));
            }
            if scope != Namespace::for_project("b") {
                assert!(!encoded.contains("project-b-private-identity"));
            }
        }
    }
}

#[tokio::test]
async fn commitment_order_is_preserved_and_duplicate_descriptions_are_atomic_errors() {
    let deps = Deps::new().await;
    let requested = vec![
        Commitment::new(Owner::Self_, "zzz-first"),
        Commitment::new(Owner::Self_, "forbid:write_identity_core_directly"),
        Commitment::new(Owner::World, "aaa-last"),
    ];
    run_reflection::execute(
        &deps,
        update("self").with_commitment_updates(requested.clone()),
    )
    .await
    .unwrap();
    assert_eq!(deps.head().await.commitments, requested);
    assert_eq!(deps.store.list_commitments().await.unwrap(), requested);
    deps.write("self", None, Some("intermediate")).await;
    execute_json(
        &deps,
        rollback_json("self", 1, 2, &["commitments"], "restore-order"),
    )
    .await
    .unwrap();
    assert_eq!(deps.head().await.commitments, requested);
    assert_eq!(deps.store.list_commitments().await.unwrap(), requested);
    let before = deps.head().await;
    let counts = deps.counts().await;
    for owner in [Owner::Self_, Owner::World] {
        let duplicate = vec![
            Commitment::new(Owner::Self_, "duplicate"),
            Commitment::new(owner, "duplicate"),
        ];
        assert!(
            run_reflection::execute(&deps, update("self").with_commitment_updates(duplicate))
                .await
                .is_err()
        );
        assert_eq!(deps.head().await, before);
        assert_eq!(deps.store.list_commitments().await.unwrap(), requested);
        assert_eq!(deps.counts().await, counts);
    }
}

#[tokio::test]
async fn rollback_rejects_audit_patch_drift_for_target_or_current_component_source() {
    for source in [1, 2] {
        let deps = Deps::new().await;
        deps.prepare_rollback().await;
        let source_version = deps.version(source).await.unwrap();
        sqlx::query("UPDATE reflections SET requested_identity_update = ? WHERE reflection_id = ?")
            .bind(r#"{"canonical_claims":["spoofed audit patch"]}"#)
            .bind(source_version.reflection_id.unwrap())
            .execute(&deps.pool)
            .await
            .unwrap();
        let before = deps.head().await;
        let counts = deps.counts().await;
        assert!(
            execute_json(
                &deps,
                rollback_json("self", 1, 2, &["identity"], "audit-drift")
            )
            .await
            .is_err()
        );
        assert_eq!(deps.head().await, before);
        assert_eq!(deps.counts().await, counts);
    }
}

#[tokio::test]
async fn concurrent_rollbacks_share_the_global_expected_version_guard() {
    let deps = Deps::new().await;
    deps.prepare_rollback().await;
    let first = rollback_json("self", 1, 2, &["identity"], "concurrent-one");
    let second = rollback_json("self", 1, 2, &["commitments"], "concurrent-two");
    let (a, b) = tokio::join!(execute_json(&deps, first), execute_json(&deps, second));
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert_eq!(deps.head().await.version, 3);
    assert_eq!(deps.counts().await.0, 3);
    assert_eq!(deps.counts().await.4, 3);
}

#[tokio::test]
async fn whole_database_backup_restore_preserves_version_chain_and_rollback_receipt_replay() {
    let original = Deps::new().await;
    original.prepare_rollback().await;
    let request = rollback_json(
        "self",
        1,
        2,
        &["identity", "commitments"],
        "restored-replay",
    );
    let result = execute_json(&original, request.clone()).await.unwrap();
    assert_eq!(result.self_model_version, Some(3));
    let counts = original.counts().await;
    let head = original.head().await;
    let restored_dir = tempfile::tempdir().unwrap();
    let restored_path = restored_dir.path().join("restored.sqlite");
    // SQLite's consistent whole-database snapshot retains the immutable ledger
    // and receipts, unlike the deliberately narrower scoped interchange export.
    sqlx::query("VACUUM INTO ?")
        .bind(restored_path.to_str().unwrap())
        .execute(&original.pool)
        .await
        .unwrap();
    let restored_url = format!(
        "sqlite://{}",
        restored_path.to_string_lossy().replace('\\', "/")
    );
    let restored = Deps {
        store: agent_llm_mm::adapters::sqlite::open_current_database(&restored_url)
            .await
            .unwrap(),
        pool: SqlitePool::connect(&restored_url).await.unwrap(),
        ids: AtomicU64::new(100),
        _dir: restored_dir,
    };
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(&restored.pool)
            .await
            .unwrap(),
        "ok"
    );
    for version in 0..=3 {
        assert_eq!(
            restored.version(version).await,
            original.version(version).await
        );
    }
    assert_eq!(restored.head().await, head);
    assert_eq!(restored.counts().await, counts);
    // The original guard expected version 2; restored head is already 3. Replay
    // must return the exact old result before stale-version validation.
    let replay = execute_json(&restored, request).await.unwrap();
    assert_eq!(replay, result);
    assert_eq!(
        serde_json::to_vec(&replay).unwrap(),
        serde_json::to_vec(&result).unwrap()
    );
    assert_eq!(restored.head().await, head);
    assert_eq!(restored.counts().await, counts);
    assert_eq!(restored.version(4).await, None);
    let next = restored
        .write("self", Some("after restored database"), None)
        .await;
    assert_eq!(next.self_model_version, Some(4));
    assert_eq!(original.head().await, head);
}
