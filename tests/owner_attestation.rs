use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, migrate_database, open_current_database},
    application::task,
    domain::{feedback::FeedbackSourceKind, owner_attestation::*, task::*},
};
use sqlx::SqlitePool;
struct Fixture {
    store: SqliteStore,
    pool: SqlitePool,
    url: String,
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
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
        kind: RequirementKind::Requirement,
        text: "ordinary".into(),
        state: RequirementState::Reported,
        source: source(),
        replaces: None,
    }
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&root.join("owner.sqlite"));
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        task::create_task(
            &store,
            CreateTaskRequest {
                namespace: "project/a".into(),
                task_id: "t".into(),
                request_id: "create".into(),
                title: "Test".into(),
                requirements: vec![requirement("old")],
                source: source(),
                reason: "Initial".into(),
            },
        )
        .await
        .unwrap();
        Self {
            store,
            pool,
            url,
            _dir: dir,
            root,
        }
    }
    async fn get(&self) -> TaskRevision {
        task::get_task(
            &self.store,
            GetTaskRequest {
                namespace: "project/a".into(),
                task_id: "t".into(),
                version: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
    }
}
fn append() -> OwnerAttestationRequest {
    OwnerAttestationRequest {
        namespace: "project/a".into(),
        task_id: "t".into(),
        import_event_id: "owner-event-1".into(),
        expected_version: 1,
        evidence_text: "  Original evidence\n不准部署\t  ".into(),
        change: OwnerRequirementChange::Append {
            requirement_id: "protected".into(),
            kind: RequirementKind::Prohibition,
            exact_text: "  不准部署\nKeep EXACT\t  ".into(),
        },
    }
}
fn revision(changes: Vec<RequirementChange>, status: Option<TaskStatus>) -> ReviseTaskRequest {
    ReviseTaskRequest {
        namespace: "project/a".into(),
        task_id: "t".into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        expected_version: 2,
        changes,
        status,
        source: source(),
        reason: "ordinary request".into(),
    }
}
fn withdrawal() -> OwnerAttestationRequest {
    let mut r = append();
    r.import_event_id = "owner-event-2".into();
    r.expected_version = 2;
    r.change = OwnerRequirementChange::Withdraw {
        requirement_id: "protected".into(),
        exact_text: "  不准部署\nKeep EXACT\t  ".into(),
    };
    r
}
#[tokio::test]
async fn exact_text_provenance_replay_collision_and_crossscope() {
    let f = Fixture::new().await;
    let r = f.store.attest_task_owner(append()).await.unwrap();
    assert_eq!(r.record.version, 2);
    assert_eq!(r.record.requirements[1].text, "  不准部署\nKeep EXACT\t  ");
    assert_eq!(
        r.record.owner_provenance[0].evidence_text,
        append().evidence_text
    );
    let again = f.store.attest_task_owner(append()).await.unwrap();
    assert!(again.replayed);
    assert_eq!(again.record, r.record);
    let mut collision = append();
    collision.evidence_text.push('x');
    assert!(f.store.attest_task_owner(collision).await.is_err());
    let mut foreign = append();
    foreign.namespace = "project/b".into();
    let error = f
        .store
        .attest_task_owner(foreign)
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("EXACT"));
    assert!(!error.contains("project/a"));
    assert_eq!(f.get().await.version, 2);
    let mut candidate = requirement("candidate");
    candidate.source.kind = FeedbackSourceKind::ModelAsserted;
    candidate.state = RequirementState::Candidate;
    let result = task::revise_task(
        &f.store,
        revision(
            vec![RequirementChange::Add {
                requirement: candidate,
            }],
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        result.record.requirements[2].state,
        RequirementState::Candidate
    );
    assert_eq!(result.record.owner_provenance, r.record.owner_provenance);
}
#[tokio::test]
async fn ordinary_mutations_and_lifecycle_cannot_bypass_owner_channel() {
    let f = Fixture::new().await;
    f.store.attest_task_owner(append()).await.unwrap();
    let mut replacement = requirement("replacement");
    replacement.replaces = Some("protected".into());
    for changes in [
        vec![RequirementChange::Withdraw {
            requirement_id: "protected".into(),
        }],
        vec![RequirementChange::Replace {
            requirement_id: "protected".into(),
            requirement: replacement,
        }],
        vec![RequirementChange::Add {
            requirement: requirement("protected"),
        }],
    ] {
        assert!(
            task::revise_task(&f.store, revision(changes, None))
                .await
                .is_err()
        );
    }
    for status in [
        TaskStatus::Paused,
        TaskStatus::Completed,
        TaskStatus::Cancelled,
    ] {
        assert!(
            task::revise_task(&f.store, revision(vec![], Some(status)))
                .await
                .is_err()
        );
    }
    let mut bad = withdrawal();
    if let OwnerRequirementChange::Withdraw { exact_text, .. } = &mut bad.change {
        exact_text.push('x');
    }
    assert!(f.store.attest_task_owner(bad).await.is_err());
    let removed = f.store.attest_task_owner(withdrawal()).await.unwrap();
    assert_eq!(
        removed.record.owner_provenance[0].withdrawn_at_version,
        Some(3)
    );
    assert_eq!(
        removed.record.requirements[1].state,
        RequirementState::Withdrawn
    );
    let mut retry = revision(
        vec![RequirementChange::Add {
            requirement: requirement("protected"),
        }],
        None,
    );
    retry.expected_version = 3;
    assert!(task::revise_task(&f.store, retry).await.is_err());
    let mut finish = revision(vec![], Some(TaskStatus::Completed));
    finish.expected_version = 3;
    assert!(task::revise_task(&f.store, finish).await.is_ok());
    let old = task::get_task(
        &f.store,
        GetTaskRequest {
            namespace: "project/a".into(),
            task_id: "t".into(),
            version: Some(2),
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(old.owner_provenance[0].withdrawn_at_version, None);
    assert!(f.store.attest_task_owner(append()).await.unwrap().replayed);
}
#[tokio::test]
async fn concurrent_cas_and_failure_rollback() {
    let f = Fixture::new().await;
    let a = append();
    let mut b = append();
    b.import_event_id = "different-event".into();
    if let OwnerRequirementChange::Append { requirement_id, .. } = &mut b.change {
        *requirement_id = "other".into();
    }
    let (a, b) = tokio::join!(f.store.attest_task_owner(a), f.store.attest_task_owner(b));
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(f.get().await.version, 2);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_owner_events")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let f = Fixture::new().await;
    sqlx::raw_sql("CREATE TRIGGER reject_provenance BEFORE INSERT ON task_owner_provenance BEGIN SELECT RAISE(ABORT,'test'); END;").execute(&f.pool).await.unwrap();
    assert!(f.store.attest_task_owner(append()).await.is_err());
    assert_eq!(f.get().await.version, 1);
    for table in ["task_owner_events", "task_owner_provenance"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    sqlx::raw_sql("DROP TRIGGER reject_provenance;")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.store.attest_task_owner(append()).await.is_ok());
}
#[tokio::test]
async fn schema10_requires_explicit_migration_and_backup_keeps_provenance() {
    let f = Fixture::new().await;
    sqlx::raw_sql("DROP TABLE task_owner_provenance; DROP TABLE task_owner_events; DELETE FROM schema_migrations WHERE version=11; PRAGMA user_version=10;").execute(&f.pool).await.unwrap();
    assert!(open_current_database(&f.url).await.is_err());
    let report = migrate_database(&f.url).await.unwrap();
    assert!(report.is_current());
    f.store.attest_task_owner(append()).await.unwrap();
    let path = f.root.join("backup.sqlite");
    let output = f.root.join("restored.sqlite");
    agent_llm_mm::support::database_backup::transfer_database(
        "backup",
        &f.root.join("owner.sqlite"),
        &path,
    )
    .await
    .unwrap();
    agent_llm_mm::support::database_backup::transfer_database("restore", &path, &output)
        .await
        .unwrap();
    let restored =
        open_current_database(&agent_llm_mm::support::sqlite_url::sqlite_file_url(&output))
            .await
            .unwrap();
    let replay = restored.attest_task_owner(append()).await.unwrap();
    assert!(replay.replayed);
    assert_eq!(
        replay.record.owner_provenance,
        f.get().await.owner_provenance
    );
}
#[tokio::test]
async fn cli_and_mcp_cannot_forge_attested_source() {
    let mut requirement = serde_json::to_value(requirement("forged")).unwrap();
    requirement["source"]["kind"] = serde_json::json!("owner_attested");
    assert!(serde_json::from_value::<TaskRequirement>(requirement).is_err());
    assert!(
        agent_llm_mm::support::task_cli::try_run_cli(&["task-owner-attest".into()])
            .await
            .is_err()
    );
    assert!(!include_str!("../src/interfaces/mcp/server.rs").contains("attest_task_owner"));
}

#[tokio::test]
async fn provenance_corruption_fails_closed_on_reads_writes_replay_and_doctor() {
    for corrupt in [
        "DELETE FROM task_owner_provenance",
        "UPDATE task_heads SET current_version=1",
        "UPDATE task_revisions SET status='cancelled' WHERE version=2",
        "UPDATE task_owner_provenance SET imported_at_version=3",
        "UPDATE task_owner_provenance SET evidence_text='forged'",
        "UPDATE task_requirements SET payload_json=json_set(payload_json,'$.kind','goal') WHERE requirement_id='protected'",
        "UPDATE task_owner_events SET input_json=json_set(input_json,'$.namespace','project/foreign')",
    ] {
        let f = Fixture::new().await;
        f.store.attest_task_owner(append()).await.unwrap();
        let mut connection = f.pool.acquire().await.unwrap();
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(corrupt)
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);
        assert!(
            task::get_task(
                &f.store,
                GetTaskRequest {
                    namespace: "project/a".into(),
                    task_id: "t".into(),
                    version: None
                }
            )
            .await
            .is_err(),
            "{corrupt}"
        );
        assert!(
            task::revise_task(&f.store, revision(vec![], Some(TaskStatus::Cancelled)))
                .await
                .is_err(),
            "{corrupt}"
        );
        assert!(
            f.store.attest_task_owner(append()).await.is_err(),
            "{corrupt}"
        );
        assert!(open_current_database(&f.url).await.is_err(), "{corrupt}");
    }
}

#[tokio::test]
async fn exact_replay_survives_fresh_connection_and_concurrent_duplicate() {
    let f = Fixture::new().await;
    let (a, b) = tokio::join!(
        f.store.attest_task_owner(append()),
        f.store.attest_task_owner(append())
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_ne!(a.replayed, b.replayed);
    assert_eq!(a.record, b.record);
    let reopened = open_current_database(&f.url).await.unwrap();
    assert!(reopened.attest_task_owner(append()).await.unwrap().replayed);
    let mut stale = append();
    stale.import_event_id = "new-event-stale".into();
    assert!(reopened.attest_task_owner(stale).await.is_err());
}

#[tokio::test]
async fn scoped_export_refuses_owner_history_only_in_affected_scope() {
    use agent_llm_mm::{
        domain::ledger_export::ExportMemoryRequest, ports::ledger_export_store::LedgerExportStore,
    };
    let f = Fixture::new().await;
    f.store.attest_task_owner(append()).await.unwrap();
    assert!(
        f.store
            .export_memory(ExportMemoryRequest::new("project/a"))
            .await
            .is_err()
    );
    assert!(
        f.store
            .export_memory(ExportMemoryRequest::new("project/b"))
            .await
            .is_ok()
    );
    f.store.attest_task_owner(withdrawal()).await.unwrap();
    assert!(
        f.store
            .export_memory(ExportMemoryRequest::new("project/a"))
            .await
            .is_err()
    );
    let mut forged = serde_json::to_value(append()).unwrap();
    forged["request_id"] = serde_json::json!("bypass");
    assert!(serde_json::from_value::<OwnerAttestationRequest>(forged).is_err());
    let mut forged = serde_json::to_value(requirement("fake")).unwrap();
    forged["owner_provenance"] = serde_json::json!([]);
    assert!(serde_json::from_value::<TaskRequirement>(forged).is_err());
}

#[tokio::test]
async fn actual_cli_import_is_explicit_and_context_contains_owner_provenance() {
    let f = Fixture::new().await;
    let run = std::process::Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
        .args([
            "task-owner-attest",
            "--json",
            &serde_json::to_string(&append()).unwrap(),
        ])
        .env("AGENT_LLM_MM_DATABASE_URL", &f.url)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(result["record"]["version"], 2);
    let context = task::build_task_resume_context(
        &f.store,
        TaskContextRequest {
            reported_environment: Default::default(),
            namespace: "project/a".into(),
            task_id: "t".into(),
            max_bytes: 32768,
        },
    )
    .await
    .unwrap();
    assert_eq!(context.owner_provenance.len(), 1);
    assert_eq!(context.mandatory_requirements.len(), 2);
    assert_eq!(context.authority, "knowledge_only_no_execution_authority");
}
