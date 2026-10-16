use agent_llm_mm::{
    adapters::sqlite::{
        CURRENT_DATABASE_SCHEMA_VERSION, SqliteStore, migrate_database, open_current_database,
    },
    application::task,
    domain::{
        feedback::{FeedbackSourceKind, FeedbackVerificationResult},
        task::*,
        types::Mode,
    },
};
use sqlx::SqlitePool;
struct Fixture {
    store: SqliteStore,
    url: String,
    _dir: tempfile::TempDir,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("tasks.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        Self {
            store,
            url,
            _dir: dir,
        }
    }
    async fn create(&self) {
        task::create_task(&self.store, create()).await.unwrap();
    }
}
fn source() -> TaskSource {
    TaskSource {
        kind: FeedbackSourceKind::CallerReported,
        reference: None,
    }
}
fn requirement(id: &str) -> TaskRequirement {
    TaskRequirement {
        requirement_id: id.into(),
        kind: RequirementKind::Prohibition,
        text: "Never deploy without explicit approval".into(),
        state: RequirementState::Reported,
        source: source(),
        replaces: None,
    }
}
fn create() -> CreateTaskRequest {
    CreateTaskRequest {
        namespace: "project/alpha".into(),
        task_id: "task-one".into(),
        request_id: "create-one".into(),
        title: "Task continuity".into(),
        requirements: vec![requirement("initial")],
        source: source(),
        reason: "Initial request".into(),
    }
}
fn revise() -> ReviseTaskRequest {
    ReviseTaskRequest {
        namespace: "project/alpha".into(),
        task_id: "task-one".into(),
        request_id: "revision-one".into(),
        expected_version: 1,
        changes: vec![RequirementChange::Add {
            requirement: requirement("addition"),
        }],
        status: None,
        source: source(),
        reason: "Supplement, retain prior requirements".into(),
    }
}
fn get(version: Option<i64>) -> GetTaskRequest {
    GetTaskRequest {
        namespace: "project/alpha".into(),
        task_id: "task-one".into(),
        version,
    }
}
fn checkpoint() -> RecordTaskCheckpointRequest {
    RecordTaskCheckpointRequest {
        namespace: "project/alpha".into(),
        task_id: "task-one".into(),
        request_id: "checkpoint-one".into(),
        checkpoint_id: "cp-one".into(),
        expected_version: 1,
        expected_checkpoint_sequence: 0,
        steps: vec![CheckpointStep {
            step_id: "planned".into(),
            text: "Plan to verify artifacts".into(),
            mode: Mode::Draft,
            verification: FeedbackVerificationResult::NotPerformed,
            evidence_refs: vec![],
        }],
        blockers: vec![],
        next_steps: vec!["Check current artifact digest".into()],
        environment: Some("isolated test".into()),
        repository_revision: Some("test-revision".into()),
        artifacts: vec![ArtifactReference {
            uri: "artifact:report".into(),
            digest: Some("reported-hash".into()),
        }],
        source: source(),
    }
}
fn context(max_bytes: usize) -> TaskContextRequest {
    TaskContextRequest {
        namespace: "project/alpha".into(),
        task_id: "task-one".into(),
        max_bytes,
    }
}
#[tokio::test]
async fn supplemental_revision_retains_original_and_immutable_history() {
    let f = Fixture::new().await;
    f.create().await;
    let r = task::revise_task(&f.store, revise()).await.unwrap();
    assert_eq!(r.record.version, 2);
    assert_eq!(r.record.requirements.len(), 2);
    let old = task::get_task(&f.store, get(Some(1)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.requirements.len(), 1);
    assert_eq!(old.requirements[0].requirement_id, "initial");
}
#[tokio::test]
async fn receipt_replay_is_atomic_and_payload_conflict_rejected() {
    let f = Fixture::new().await;
    let first = task::create_task(&f.store, create()).await.unwrap();
    let replay = task::create_task(&f.store, create()).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(first.operation_id, replay.operation_id);
    let mut changed = create();
    changed.title = "Different request".into();
    assert!(task::create_task(&f.store, changed).await.is_err());
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_revisions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn stale_revision_cas_rejected_and_retry_returns_original() {
    let f = Fixture::new().await;
    f.create().await;
    task::revise_task(&f.store, revise()).await.unwrap();
    assert!(
        task::revise_task(&f.store, revise())
            .await
            .unwrap()
            .replayed
    );
    let mut stale = revise();
    stale.request_id = "other".into();
    assert!(task::revise_task(&f.store, stale).await.is_err());
}
#[tokio::test]
async fn explicit_replacement_preserves_withdrawn_id() {
    let f = Fixture::new().await;
    f.create().await;
    let mut r = revise();
    let mut q = requirement("replacement");
    q.replaces = Some("initial".into());
    r.changes = vec![RequirementChange::Replace {
        requirement_id: "initial".into(),
        requirement: q,
    }];
    let result = task::revise_task(&f.store, r).await.unwrap();
    assert_eq!(
        result.record.requirements[0].state,
        RequirementState::Withdrawn
    );
    assert_eq!(
        result.record.requirements[1].replaces.as_deref(),
        Some("initial")
    );
}
#[tokio::test]
async fn candidate_changes_cannot_erase_reported_requirements() {
    let f = Fixture::new().await;
    f.create().await;
    let mut r = revise();
    r.source.kind = FeedbackSourceKind::ModelAsserted;
    r.changes = vec![RequirementChange::Withdraw {
        requirement_id: "initial".into(),
    }];
    assert!(task::revise_task(&f.store, r).await.is_err());
    let mut q = requirement("model");
    q.source.kind = FeedbackSourceKind::ModelAsserted;
    let mut r = revise();
    r.changes = vec![RequirementChange::Add { requirement: q }];
    assert!(task::revise_task(&f.store, r).await.is_err());
}
#[tokio::test]
async fn all_candidate_task_has_no_resumable_effective_requirements() {
    let f = Fixture::new().await;
    let mut r = create();
    r.source.kind = FeedbackSourceKind::ModelAsserted;
    r.requirements[0].source.kind = FeedbackSourceKind::ModelAsserted;
    r.requirements[0].state = RequirementState::Candidate;
    task::create_task(&f.store, r).await.unwrap();
    let c = task::build_task_resume_context(&f.store, context(16384))
        .await
        .unwrap();
    assert!(!c.can_resume);
    assert_eq!(c.mandatory_requirement_count, 0);
}
#[tokio::test]
async fn paused_and_cancelled_tasks_are_not_resumable() {
    let f = Fixture::new().await;
    f.create().await;
    let mut r = revise();
    r.changes.clear();
    r.status = Some(TaskStatus::Paused);
    task::revise_task(&f.store, r.clone()).await.unwrap();
    assert!(
        !task::build_task_resume_context(&f.store, context(16384))
            .await
            .unwrap()
            .can_resume
    );
    r.request_id = "cancel".into();
    r.expected_version = 2;
    r.status = Some(TaskStatus::Cancelled);
    task::revise_task(&f.store, r).await.unwrap();
    assert!(
        !task::build_task_resume_context(&f.store, context(16384))
            .await
            .unwrap()
            .can_resume
    );
}
#[tokio::test]
async fn context_budget_never_silently_drops_constraints() {
    let f = Fixture::new().await;
    let mut r = create();
    r.requirements[0].text = "Hard constraint. ".repeat(240);
    task::create_task(&f.store, r).await.unwrap();
    let c = task::build_task_resume_context(&f.store, context(2048))
        .await
        .unwrap();
    assert!(!c.complete);
    assert!(!c.can_resume);
    assert_eq!(c.mandatory_requirement_count, 1);
    assert!(c.mandatory_requirements.is_empty());
    assert_eq!(serde_json::to_vec(&c).unwrap().len(), c.serialized_bytes);
    assert!(c.serialized_bytes <= 2048);
    assert!(
        c.diagnostics
            .iter()
            .any(|d| d.contains("mandatory_requirements_do_not_fit"))
    );
}
#[tokio::test]
async fn checkpoint_sequence_cas_and_resume_version_are_honest() {
    let f = Fixture::new().await;
    f.create().await;
    task::record_task_checkpoint(&f.store, checkpoint())
        .await
        .unwrap();
    assert!(
        task::record_task_checkpoint(&f.store, checkpoint())
            .await
            .unwrap()
            .replayed
    );
    let mut duplicate = checkpoint();
    duplicate.request_id = "cp-two".into();
    duplicate.checkpoint_id = "cp-two".into();
    assert!(
        task::record_task_checkpoint(&f.store, duplicate)
            .await
            .is_err()
    );
    task::revise_task(&f.store, revise()).await.unwrap();
    let c = task::build_task_resume_context(&f.store, context(16384))
        .await
        .unwrap();
    assert_eq!(c.task_version, 2);
    assert_eq!(c.checkpoint.unwrap().task_version, 1);
    assert!(!c.can_resume);
    assert_eq!(
        c.artifact_verification,
        FeedbackVerificationResult::NotPerformed
    );
    assert!(
        c.diagnostics
            .iter()
            .any(|d| d == "stale_checkpoint_task_version")
    );
}
#[tokio::test]
async fn plans_cannot_be_labeled_verified_results() {
    let f = Fixture::new().await;
    f.create().await;
    let mut r = checkpoint();
    r.steps[0].verification = FeedbackVerificationResult::Passed;
    assert!(task::record_task_checkpoint(&f.store, r).await.is_err());
    let snapshot = task::get_task_snapshot(&f.store, "project/alpha", "task-one")
        .await
        .unwrap()
        .unwrap();
    assert!(snapshot.checkpoint.is_none());
}
#[tokio::test]
async fn source_reference_is_checked_in_exact_namespace() {
    let f = Fixture::new().await;
    let mut r = create();
    r.requirements[0].source.reference = Some("event:missing".into());
    assert!(task::create_task(&f.store, r).await.is_err());
    assert!(task::get_task(&f.store, get(None)).await.unwrap().is_none());
}
#[tokio::test]
async fn tasks_are_workspace_isolated_and_global_scope_is_rejected() {
    let f = Fixture::new().await;
    f.create().await;
    let mut r = get(None);
    r.namespace = "project/beta".into();
    assert!(task::get_task(&f.store, r).await.unwrap().is_none());
    let mut r = create();
    r.namespace = "world".into();
    assert!(task::create_task(&f.store, r).await.is_err());
    let mut r = create();
    r.namespace = "project/beta".into();
    task::create_task(&f.store, r).await.unwrap();
    assert_eq!(
        task::list_tasks(
            &f.store,
            ListTasksRequest {
                namespace: "project/beta".into(),
                limit: 20,
                after_id: None
            }
        )
        .await
        .unwrap()
        .len(),
        1
    );
}
#[tokio::test]
async fn context_snapshot_truth_and_size_are_explicit() {
    let f = Fixture::new().await;
    f.create().await;
    let c = task::build_task_resume_context(&f.store, context(16384))
        .await
        .unwrap();
    assert!(c.complete && c.can_resume && c.mandatory_snapshot_consistent);
    assert!(!c.snapshot_consistent);
    assert_eq!(c.task_version_after_selection, Some(1));
    assert_eq!(
        c.mandatory_requirement_count,
        c.mandatory_requirements.len()
    );
    assert_eq!(c.authority, "knowledge_only_no_execution_authority");
    assert_eq!(serde_json::to_vec(&c).unwrap().len(), c.serialized_bytes);
}
#[tokio::test]
async fn schema7_requires_explicit_migration_and_schema8_preserves_records() {
    let f = Fixture::new().await;
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    sqlx::raw_sql("DROP TABLE experience_use_events;DROP TABLE experience_uses;DROP TABLE task_checkpoint_sources;DROP TABLE task_checkpoints;DROP TABLE task_requirements;DROP TABLE task_revisions;DROP TABLE task_heads;DELETE FROM schema_migrations WHERE version>=8;PRAGMA user_version=7;").execute(&pool).await.unwrap();
    assert!(open_current_database(&f.url).await.is_err());
    let report = migrate_database(&f.url).await.unwrap();
    assert_eq!(report.schema_version, Some(CURRENT_DATABASE_SCHEMA_VERSION));
    assert!(report.backup_path.is_some());
    let store = open_current_database(&f.url).await.unwrap();
    task::create_task(&store, create()).await.unwrap();
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM task_requirements")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1);
}
#[tokio::test]
async fn task_cli_rejects_ambiguous_arguments_without_opening_database() {
    assert!(
        !agent_llm_mm::support::task_cli::try_run_cli(&["doctor".into()])
            .await
            .unwrap()
    );
    assert!(
        agent_llm_mm::support::task_cli::try_run_cli(&["task".into(), "create".into()])
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_expected_version_writers_leave_one_revision_and_one_receipt() {
    let f = Fixture::new().await;
    f.create().await;
    let a = revise();
    let mut b = revise();
    b.request_id = "other-writer".into();
    b.changes = vec![RequirementChange::Add {
        requirement: requirement("other-addition"),
    }];
    let (a, b) = tokio::join!(
        task::revise_task(&f.store, a),
        task::revise_task(&f.store, b)
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    let revisions: i64 = sqlx::query_scalar("SELECT count(*) FROM task_revisions")
        .fetch_one(&pool)
        .await
        .unwrap();
    let receipts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM operation_log WHERE entrypoint='revise_task'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(revisions, 2);
    assert_eq!(receipts, 1);
}
#[tokio::test]
async fn receipt_insert_failure_rolls_back_task_mutation() {
    let f = Fixture::new().await;
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_task_receipt BEFORE INSERT ON operation_log WHEN NEW.entrypoint='create_task' BEGIN SELECT RAISE(ABORT,'synthetic receipt failure'); END").execute(&pool).await.unwrap();
    assert!(task::create_task(&f.store, create()).await.is_err());
    for table in ["task_heads", "task_revisions", "task_requirements"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0, "{table}");
    }
    sqlx::query("DROP TRIGGER reject_task_receipt")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        !task::create_task(&f.store, create())
            .await
            .unwrap()
            .replayed
    );
}
#[tokio::test]
async fn existing_foreign_namespace_source_is_rejected_without_partial_write() {
    let f = Fixture::new().await;
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('foreign-evidence','2026-10-16T00:00:00Z','world','project/beta','observation','synthetic foreign evidence')").execute(&pool).await.unwrap();
    let mut r = create();
    r.requirements[0].source.reference = Some("event:foreign-evidence".into());
    assert!(task::create_task(&f.store, r).await.is_err());
    assert!(task::get_task(&f.store, get(None)).await.unwrap().is_none());
    f.create().await;
    let mut cp = checkpoint();
    cp.steps[0].mode = Mode::Observed;
    cp.steps[0].verification = FeedbackVerificationResult::Passed;
    cp.steps[0].evidence_refs = vec!["event:foreign-evidence".into()];
    assert!(task::record_task_checkpoint(&f.store, cp).await.is_err());
}
#[tokio::test]
async fn full_database_backup_restore_preserves_tasks_checkpoints_and_receipts() {
    let f = Fixture::new().await;
    f.create().await;
    task::record_task_checkpoint(&f.store, checkpoint())
        .await
        .unwrap();
    let original = f._dir.path().join("tasks.sqlite");
    let backup = f._dir.path().join("backup.sqlite");
    let restored = f._dir.path().join("restored.sqlite");
    agent_llm_mm::support::database_backup::transfer_database("backup", &original, &backup)
        .await
        .unwrap();
    agent_llm_mm::support::database_backup::transfer_database("restore", &backup, &restored)
        .await
        .unwrap();
    let store = open_current_database(&agent_llm_mm::support::sqlite_url::sqlite_file_url(
        &restored,
    ))
    .await
    .unwrap();
    let snapshot = task::get_task_snapshot(&store, "project/alpha", "task-one")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snapshot.task.requirements.len(), 1);
    assert_eq!(snapshot.checkpoint.unwrap().sequence, 1);
    assert!(task::create_task(&store, create()).await.unwrap().replayed);
    assert!(
        task::record_task_checkpoint(&store, checkpoint())
            .await
            .unwrap()
            .replayed
    );
}

#[tokio::test]
async fn corrupted_checkpoint_id_edges_or_source_scope_fail_closed_on_read() {
    for damage in ["id", "edges", "foreign_source", "plan_verified"] {
        let f = Fixture::new().await;
        f.create().await;
        let pool = SqlitePool::connect(&f.url).await.unwrap();
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('evidence','2026-10-16T00:00:00Z','world','project/alpha','observation','synthetic result')").execute(&pool).await.unwrap();
        let mut cp = checkpoint();
        cp.steps[0].mode = Mode::Observed;
        cp.steps[0].verification = FeedbackVerificationResult::Passed;
        cp.steps[0].evidence_refs = vec!["event:evidence".into()];
        task::record_task_checkpoint(&f.store, cp).await.unwrap();
        match damage {
            "id" => {
                sqlx::query("UPDATE task_checkpoints SET payload_json=json_set(payload_json,'$.checkpoint_id','forged')").execute(&pool).await.unwrap();
            }
            "edges" => {
                sqlx::query("DELETE FROM task_checkpoint_sources")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            "foreign_source" => {
                sqlx::query("UPDATE events SET namespace='project/beta' WHERE event_id='evidence'")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            "plan_verified" => {
                sqlx::query("UPDATE task_checkpoints SET payload_json=json_set(payload_json,'$.steps[0].mode','Draft')").execute(&pool).await.unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            task::get_task_snapshot(&f.store, "project/alpha", "task-one")
                .await
                .is_err(),
            "accepted {damage}"
        );
        assert!(
            task::build_task_resume_context(&f.store, context(16384))
                .await
                .is_err(),
            "context accepted {damage}"
        );
    }
}
#[tokio::test]
async fn corrupted_requirement_provenance_and_missing_current_revision_fail_closed() {
    for damage in ["illegal_source_state", "foreign_source", "head"] {
        let f = Fixture::new().await;
        let pool = SqlitePool::connect(&f.url).await.unwrap();
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('evidence','2026-10-16T00:00:00Z','world','project/alpha','conversation','synthetic requirement')").execute(&pool).await.unwrap();
        let mut request = create();
        request.requirements[0].source.reference = Some("event:evidence".into());
        task::create_task(&f.store, request).await.unwrap();
        match damage {
            "illegal_source_state" => {
                sqlx::query("UPDATE task_requirements SET payload_json=json_set(payload_json,'$.source.kind','model_asserted')").execute(&pool).await.unwrap();
            }
            "foreign_source" => {
                sqlx::query("UPDATE events SET namespace='project/beta' WHERE event_id='evidence'")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            "head" => {
                sqlx::query("UPDATE task_heads SET current_version=99")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            task::get_task(&f.store, get(None)).await.is_err(),
            "accepted {damage}"
        );
        assert!(
            task::list_tasks(
                &f.store,
                ListTasksRequest {
                    namespace: "project/alpha".into(),
                    limit: 20,
                    after_id: None
                }
            )
            .await
            .is_err(),
            "list accepted {damage}"
        );
    }
}

#[tokio::test]
async fn persisted_corruption_is_storage_error_but_invalid_request_is_param_error() {
    let f = Fixture::new().await;
    f.create().await;
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    sqlx::query("UPDATE task_heads SET current_version=99")
        .execute(&pool)
        .await
        .unwrap();
    let error = task::get_task(&f.store, get(None)).await.unwrap_err();
    assert!(matches!(error, agent_llm_mm::error::AppError::Message(_)));
    let mut request = get(None);
    request.namespace = "world".into();
    assert!(matches!(
        task::get_task(&f.store, request).await.unwrap_err(),
        agent_llm_mm::error::AppError::InvalidParams(_)
    ));
}
