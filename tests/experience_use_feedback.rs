use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, inspect_database, migrate_database, open_current_database},
    application::{experience, experience_use, task},
    domain::{
        event::Event,
        experience::*,
        experience_use::*,
        feedback::{FeedbackSourceKind, FeedbackVerificationResult},
        task::*,
        types::{EventKind, Namespace, Owner},
    },
    ports::{EventStore, StoredEvent},
};
use chrono::Utc;
use sqlx::SqlitePool;
struct Fixture {
    store: SqliteStore,
    pool: SqlitePool,
    url: String,
    _dir: tempfile::TempDir,
}
fn episode() -> EpisodeContent {
    EpisodeContent {
        title: "Cache check".into(),
        objective: "Observe cache".into(),
        actions: vec!["Run test".into()],
        observations: vec!["Test passed".into()],
        outcome: "Observed pass".into(),
        lesson: "Warm cache".into(),
        limitations: vec!["One local run; no causal proof".into()],
        source_event_refs: vec!["event:e".into()],
    }
}
fn content(id: &str) -> ExperienceContent {
    ExperienceContent {
        kind: ExperienceKind::Semantic,
        title: "Cache".into(),
        statement: "Warm cache".into(),
        steps: vec![],
        limitations: vec!["Local only".into()],
        source_episode_ids: vec![id.into()],
    }
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("use.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        for (id, ns) in [("e", "project/p"), ("other", "project/q")] {
            store
                .append_event(StoredEvent::new(
                    id.into(),
                    Utc::now(),
                    Event::new_with_namespace(
                        Owner::World,
                        Namespace::parse(ns).unwrap(),
                        EventKind::Observation,
                        "test result",
                    )
                    .unwrap(),
                ))
                .await
                .unwrap();
        }
        task::create_task(
            &store,
            CreateTaskRequest {
                namespace: "project/p".into(),
                task_id: "t".into(),
                request_id: "create-t".into(),
                title: "Cache".into(),
                requirements: vec![TaskRequirement {
                    requirement_id: "r".into(),
                    kind: RequirementKind::Goal,
                    text: "Check cache".into(),
                    state: RequirementState::Reported,
                    source: TaskSource {
                        kind: FeedbackSourceKind::CallerReported,
                        reference: None,
                    },
                    replaces: None,
                }],
                source: TaskSource {
                    kind: FeedbackSourceKind::CallerReported,
                    reference: None,
                },
                reason: "request".into(),
            },
        )
        .await
        .unwrap();
        experience::create_episode(
            &store,
            CreateEpisodeRequest {
                namespace: "project/p".into(),
                request_id: "ep".into(),
                episode_id: "ep".into(),
                content: episode(),
            },
        )
        .await
        .unwrap();
        experience::create_candidate(
            &store,
            CreateCandidateRequest {
                namespace: "project/p".into(),
                request_id: "c".into(),
                candidate_id: "c".into(),
                content: content("ep"),
            },
        )
        .await
        .unwrap();
        experience::update_candidate_status(
            &store,
            UpdateCandidateStatusRequest {
                namespace: "project/p".into(),
                request_id: "activate".into(),
                candidate_id: "c".into(),
                expected_version: 1,
                status: ExperienceStatus::Active,
            },
        )
        .await
        .unwrap();
        Self {
            store,
            pool,
            url,
            _dir: dir,
        }
    }
    async fn record(&self) {
        experience_use::record_experience_use(&self.store, request())
            .await
            .unwrap();
    }
    async fn withdraw(&self) {
        experience::update_candidate_status(
            &self.store,
            UpdateCandidateStatusRequest {
                namespace: "project/p".into(),
                request_id: "withdraw".into(),
                candidate_id: "c".into(),
                expected_version: 2,
                status: ExperienceStatus::Rejected,
            },
        )
        .await
        .unwrap();
    }
}
fn request() -> RecordExperienceUseRequest {
    RecordExperienceUseRequest {
        namespace: "project/p".into(),
        request_id: "use-write".into(),
        use_id: "u".into(),
        task_id: "t".into(),
        task_version: 1,
        expected_task_version: 1,
        candidate_id: "c".into(),
        candidate_version: 2,
        expected_candidate_version: 2,
        decision: ExperienceUseDecision::Adopted,
        reason: "Caller chose this suggestion".into(),
        conditions: vec![ApplicabilityCondition {
            condition: "Local cache test".into(),
            assessment: ApplicabilityAssessment::CallerReportedSatisfied,
        }],
        action_event_refs: vec!["e".into()],
        outcome_event_refs: vec!["event:e".into()],
        verification: FeedbackVerificationResult::Passed,
        evaluation_method: "Caller-reported local test".into(),
        limitations: vec!["No independent verification or causal attribution".into()],
        related_use_id: None,
    }
}
fn list() -> ListExperienceUsesRequest {
    ListExperienceUsesRequest {
        namespace: "project/p".into(),
        task_id: "t".into(),
        limit: 20,
        after_id: None,
    }
}
fn derive() -> DeriveExperienceRequest {
    DeriveExperienceRequest {
        namespace: "project/p".into(),
        request_id: "derive".into(),
        use_id: "u".into(),
        episode_id: "derived-ep".into(),
        candidate_id: "derived-c".into(),
        episode_content: episode(),
        candidate_content: content("derived-ep"),
    }
}
#[tokio::test]
async fn use_receipts_cas_scope_and_applicability() {
    let f = Fixture::new().await;
    let first = experience_use::record_experience_use(&f.store, request())
        .await
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(first.record.observation.action_event_refs, vec!["event:e"]);
    assert!(
        experience_use::record_experience_use(&f.store, request())
            .await
            .unwrap()
            .replayed
    );
    let mut r = request();
    r.reason = "changed".into();
    assert!(
        experience_use::record_experience_use(&f.store, r)
            .await
            .is_err()
    );
    let mut r = request();
    r.request_id = "new".into();
    r.use_id = "new".into();
    r.expected_task_version = 2;
    assert!(
        experience_use::record_experience_use(&f.store, r)
            .await
            .is_err()
    );
    let mut r = request();
    r.request_id = "cross".into();
    r.use_id = "cross".into();
    r.outcome_event_refs = vec!["event:other".into()];
    assert!(
        experience_use::record_experience_use(&f.store, r)
            .await
            .is_err()
    );
    let mut r = request();
    r.conditions[0].assessment = ApplicabilityAssessment::Unknown;
    assert!(r.validate().is_err());
    let mut l = list();
    l.namespace = "project/q".into();
    assert!(
        experience_use::list_experience_uses(&f.store, l)
            .await
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn withdrawal_invalidates_reuse_without_rewriting_results() {
    let f = Fixture::new().await;
    f.record().await;
    assert!(
        experience_use::list_experience_uses(&f.store, list())
            .await
            .unwrap()[0]
            .currently_reusable
    );
    f.withdraw().await;
    let views = experience_use::list_experience_uses(&f.store, list())
        .await
        .unwrap();
    assert!(!views[0].currently_reusable);
    assert_eq!(
        views[0].record.observation.verification,
        FeedbackVerificationResult::Passed
    );
    assert!(
        experience_use::record_experience_use(&f.store, request())
            .await
            .unwrap()
            .replayed
    );
    let mut late = request();
    late.use_id = "late".into();
    late.request_id = "late".into();
    late.related_use_id = Some("u".into());
    late.expected_candidate_version = 3;
    late.verification = FeedbackVerificationResult::Failed;
    experience_use::record_experience_use(&f.store, late)
        .await
        .unwrap();
    let ctx = task::build_task_resume_context(
        &f.store,
        TaskContextRequest {
            namespace: "project/p".into(),
            task_id: "t".into(),
            max_bytes: 16384,
        },
    )
    .await
    .unwrap();
    assert!(ctx.experiences.is_empty());
}
#[tokio::test]
async fn derivation_is_source_bounded_idempotent_and_pending() {
    let f = Fixture::new().await;
    f.record().await;
    let d = experience_use::derive_experience_from_use(&f.store, derive())
        .await
        .unwrap();
    assert_eq!(d.stage, "pending_candidate_created");
    assert_eq!(
        d.candidate.unwrap().record.status,
        ExperienceStatus::Pending
    );
    let replay = experience_use::derive_experience_from_use(&f.store, derive())
        .await
        .unwrap();
    assert!(replay.episode.unwrap().replayed && replay.candidate.unwrap().replayed);
    let mut changed = derive();
    changed.candidate_content.statement = "changed".into();
    assert!(
        experience_use::derive_experience_from_use(&f.store, changed)
            .await
            .is_err()
    );
    let mut cross = derive();
    cross.request_id = "cross".into();
    cross.episode_content.source_event_refs = vec!["event:other".into()];
    assert!(
        experience_use::derive_experience_from_use(&f.store, cross)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn derivation_resumes_after_candidate_failure_without_duplicate_episode() {
    let f = Fixture::new().await;
    f.record().await;
    sqlx::raw_sql("CREATE TRIGGER fail_derived_candidate BEFORE INSERT ON experience_candidates WHEN NEW.candidate_id='derived-c' BEGIN SELECT RAISE(ABORT,'test failure'); END;").execute(&f.pool).await.unwrap();
    let d = experience_use::derive_experience_from_use(&f.store, derive())
        .await
        .unwrap();
    assert_eq!(d.stage, "episode_created");
    assert!(d.candidate.is_none() && d.retryable_error.is_some());
    sqlx::query("DROP TRIGGER fail_derived_candidate")
        .execute(&f.pool)
        .await
        .unwrap();
    let d = experience_use::derive_experience_from_use(&f.store, derive())
        .await
        .unwrap();
    assert!(d.episode.unwrap().replayed);
    assert_eq!(d.stage, "pending_candidate_created");
}
#[tokio::test]
async fn schema8_migration_preserves_old_payload_and_receipt_bytes() {
    let f = Fixture::new().await;
    let before:Vec<(String,String)>=sqlx::query_as("SELECT request_summary_json,response_summary_json FROM operation_log ORDER BY operation_id").fetch_all(&f.pool).await.unwrap();
    let payload: Vec<String> = sqlx::query_scalar(
        "SELECT payload_json FROM experience_candidate_versions ORDER BY version",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    sqlx::raw_sql("DROP TABLE experience_use_events;DROP TABLE experience_uses;DELETE FROM schema_migrations WHERE version=9;PRAGMA user_version=8;").execute(&f.pool).await.unwrap();
    assert!(open_current_database(&f.url).await.is_err());
    let migrated = migrate_database(&f.url).await.unwrap();
    assert_eq!(migrated.schema_version, Some(9));
    assert!(migrated.backup_path.is_some());
    let after:Vec<(String,String)>=sqlx::query_as("SELECT request_summary_json,response_summary_json FROM operation_log ORDER BY operation_id").fetch_all(&f.pool).await.unwrap();
    assert_eq!(before, after);
    let after: Vec<String> = sqlx::query_scalar(
        "SELECT payload_json FROM experience_candidate_versions ORDER BY version",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert_eq!(payload, after);
    assert!(inspect_database(&f.url).await.unwrap().is_current());
    f.record().await;
}
#[tokio::test]
async fn receipt_failure_rolls_back_use_and_relational_edges() {
    let f = Fixture::new().await;
    sqlx::raw_sql("CREATE TRIGGER fail_use_receipt BEFORE INSERT ON operation_log WHEN NEW.entrypoint='record_experience_use' BEGIN SELECT RAISE(ABORT,'test failure'); END;").execute(&f.pool).await.unwrap();
    assert!(
        experience_use::record_experience_use(&f.store, request())
            .await
            .is_err()
    );
    for table in ["experience_uses", "experience_use_events"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(n, 0);
    }
}

#[tokio::test]
async fn full_backup_restores_observations_edges_and_derivation_receipts() {
    let f = Fixture::new().await;
    f.record().await;
    experience_use::derive_experience_from_use(&f.store, derive())
        .await
        .unwrap();
    let original = f._dir.path().join("use.sqlite");
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
    assert!(
        experience_use::record_experience_use(&store, request())
            .await
            .unwrap()
            .replayed
    );
    assert_eq!(
        experience_use::list_experience_uses(&store, list())
            .await
            .unwrap()
            .len(),
        1
    );
    let d = experience_use::derive_experience_from_use(&store, derive())
        .await
        .unwrap();
    assert!(d.episode.unwrap().replayed && d.candidate.unwrap().replayed);
}
#[tokio::test]
async fn edge_corruption_fails_closed_and_retry_keys_are_not_persisted() {
    let f = Fixture::new().await;
    let mut r = request();
    r.request_id = "private-use-retry-key".into();
    experience_use::record_experience_use(&f.store, r)
        .await
        .unwrap();
    let mut d = derive();
    d.request_id = "private-derive-retry-key".into();
    experience_use::derive_experience_from_use(&f.store, d)
        .await
        .unwrap();
    let rows: Vec<String> = sqlx::query_scalar("SELECT response_summary_json FROM operation_log")
        .fetch_all(&f.pool)
        .await
        .unwrap();
    assert!(
        rows.iter().all(
            |s| !s.contains("private-use-retry-key") && !s.contains("private-derive-retry-key")
        )
    );
    sqlx::query("DELETE FROM experience_use_events WHERE role='action'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        experience_use::list_experience_uses(&f.store, list())
            .await
            .is_err()
    );
}
