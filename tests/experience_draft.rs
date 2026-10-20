use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
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
use agent_llm_mm::{
    application::experience_draft::generate_experience_draft,
    domain::{experience_draft::*, self_revision::*},
    error::AppError,
    ports::{
        ModelDecision, ModelDecisionRequest, ModelPort,
        experience_draft_store::ExperienceDraftStore,
    },
};
use std::sync::atomic::{AtomicUsize, Ordering};
struct CountingModel {
    calls: AtomicUsize,
    mode: u8,
}
#[async_trait::async_trait]
impl ModelPort for CountingModel {
    async fn decide(&self, _: ModelDecisionRequest) -> Result<ModelDecision, AppError> {
        unreachable!()
    }
    async fn propose_self_revision(
        &self,
        _: SelfRevisionRequest,
    ) -> Result<SelfRevisionProposal, AppError> {
        unreachable!()
    }
    async fn propose_experience_draft(
        &self,
        input: ExperienceDraftInput,
    ) -> Result<ExperienceDraftProposal, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        input.validate()?;
        assert_eq!(input.evidence.len(), 1);
        assert_eq!(input.evidence[0].event_id, "e");
        if self.mode == 1 {
            return Err(AppError::Message(
                "synthetic provider secret: must not persist".into(),
            ));
        }
        let mut proposal = ExperienceDraftProposal {
            episode_content: episode(),
            candidate_content: content(&input.request.episode_id),
        };
        if self.mode == 2 {
            proposal.episode_content.source_event_refs = vec!["other".into()];
        }
        Ok(proposal)
    }
}
fn model(mode: u8) -> CountingModel {
    CountingModel {
        calls: AtomicUsize::new(0),
        mode,
    }
}
fn draft() -> GenerateExperienceDraftRequest {
    GenerateExperienceDraftRequest {
        namespace: "project/p".into(),
        request_id: "draft".into(),
        use_id: "u".into(),
        episode_id: "draft-ep".into(),
        candidate_id: "draft-c".into(),
        allow_model_generation: true,
    }
}
#[tokio::test]
async fn explicit_opt_in_exact_scope_and_pending_only_with_replay() {
    let f = Fixture::new().await;
    f.record().await;
    let m = model(0);
    let mut r = draft();
    r.allow_model_generation = false;
    assert!(generate_experience_draft(&f.store, &m, r).await.is_err());
    let mut r = draft();
    r.namespace = "project/q".into();
    assert!(generate_experience_draft(&f.store, &m, r).await.is_err());
    assert_eq!(m.calls.load(Ordering::SeqCst), 0);
    let first = generate_experience_draft(&f.store, &m, draft())
        .await
        .unwrap();
    assert_eq!(first.status, ExperienceDraftStatus::PendingCandidateCreated);
    assert_eq!(
        first.derivation.unwrap().candidate.unwrap().record.status,
        ExperienceStatus::Pending
    );
    let retry = generate_experience_draft(&f.store, &m, draft())
        .await
        .unwrap();
    assert!(retry.replayed);
    assert_eq!(m.calls.load(Ordering::SeqCst), 1);
    let mut conflict = draft();
    conflict.candidate_id = "different".into();
    assert!(
        generate_experience_draft(&f.store, &m, conflict)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn durable_reservation_without_result_never_calls_model() {
    let f = Fixture::new().await;
    f.record().await;
    let m = model(0);
    f.store.reserve_experience_draft(draft()).await.unwrap();
    let out = generate_experience_draft(&f.store, &m, draft())
        .await
        .unwrap();
    assert_eq!(out.status, ExperienceDraftStatus::IncompleteOrInFlight);
    assert_eq!(m.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn known_failures_and_rejected_evidence_are_terminal_and_redacted() {
    for (mode, status) in [
        (1, ExperienceDraftStatus::KnownFailed),
        (2, ExperienceDraftStatus::Rejected),
    ] {
        let f = Fixture::new().await;
        f.record().await;
        let m = model(mode);
        assert_eq!(
            generate_experience_draft(&f.store, &m, draft())
                .await
                .unwrap()
                .status,
            status
        );
        assert_eq!(
            generate_experience_draft(&f.store, &m, draft())
                .await
                .unwrap()
                .status,
            status
        );
        assert_eq!(m.calls.load(Ordering::SeqCst), 1);
        let responses: Vec<String> =
            sqlx::query_scalar("SELECT response_summary_json FROM operation_log")
                .fetch_all(&f.pool)
                .await
                .unwrap();
        assert!(!responses.join("").contains("synthetic provider secret"));
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM experience_candidates WHERE candidate_id='draft-c'",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
    }
}
#[tokio::test]
async fn saved_proposal_resumes_after_restart_without_model() {
    let f = Fixture::new().await;
    f.record().await;
    let m = model(0);
    let reservation = f.store.reserve_experience_draft(draft()).await.unwrap();
    let proposal = ExperienceDraftProposal {
        episode_content: episode(),
        candidate_content: content("draft-ep"),
    };
    proposal.validate(&reservation.record).unwrap();
    f.store
        .complete_experience_draft(
            &draft(),
            DraftCompletion::SavedProposal {
                proposal: Box::new(proposal),
            },
        )
        .await
        .unwrap();
    let reopened = SqliteStore::bootstrap(&f.url).await.unwrap();
    let out = generate_experience_draft(&reopened, &m, draft())
        .await
        .unwrap();
    assert_eq!(out.status, ExperienceDraftStatus::PendingCandidateCreated);
    assert_eq!(m.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn concurrent_identical_requests_invoke_at_most_once() {
    let f = Fixture::new().await;
    f.record().await;
    let m = model(0);
    let (a, b) = tokio::join!(
        generate_experience_draft(&f.store, &m, draft()),
        generate_experience_draft(&f.store, &m, draft())
    );
    assert!(a.is_ok());
    assert!(b.is_ok());
    assert_eq!(m.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn oversized_evidence_and_tampered_edges_fail_before_model() {
    let f = Fixture::new().await;
    f.record().await;
    let m = model(0);
    sqlx::query("UPDATE events SET summary=? WHERE event_id='e'")
        .bind("x".repeat(MAX_DRAFT_INPUT_BYTES + 1))
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        generate_experience_draft(&f.store, &m, draft())
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM experience_use_events WHERE use_id='u'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        generate_experience_draft(&f.store, &m, draft())
            .await
            .is_err()
    );
    assert_eq!(m.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn saved_proposal_retries_only_unfinished_derivation_steps() {
    let f = Fixture::new().await;
    f.record().await;
    let m = model(0);
    sqlx::query("CREATE TRIGGER fail_draft_candidate BEFORE INSERT ON experience_candidates WHEN NEW.candidate_id='draft-c' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END").execute(&f.pool).await.unwrap();
    let out = generate_experience_draft(&f.store, &m, draft())
        .await
        .unwrap();
    assert_eq!(out.status, ExperienceDraftStatus::SavedProposal);
    assert!(out.derivation.unwrap().episode.is_some());
    sqlx::query("DROP TRIGGER fail_draft_candidate")
        .execute(&f.pool)
        .await
        .unwrap();
    let out = generate_experience_draft(&f.store, &m, draft())
        .await
        .unwrap();
    assert_eq!(out.status, ExperienceDraftStatus::PendingCandidateCreated);
    assert!(out.derivation.unwrap().episode.unwrap().replayed);
    assert_eq!(m.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn oversized_persisted_receipt_and_event_metadata_are_rejected() {
    let f = Fixture::new().await;
    f.record().await;
    let m = model(0);
    sqlx::query("UPDATE events SET observed_at=? WHERE event_id='e'")
        .bind("x".repeat(MAX_DRAFT_INPUT_BYTES + 1))
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        generate_experience_draft(&f.store, &m, draft())
            .await
            .is_err()
    );
    sqlx::query("UPDATE events SET observed_at=NULL WHERE event_id='e'")
        .execute(&f.pool)
        .await
        .unwrap();
    let reserved = f.store.reserve_experience_draft(draft()).await.unwrap();
    sqlx::query("UPDATE operation_log SET response_summary_json=? WHERE operation_id=?")
        .bind("x".repeat(140000))
        .bind(&reserved.operation_id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        generate_experience_draft(&f.store, &m, draft())
            .await
            .is_err()
    );
    assert_eq!(m.calls.load(Ordering::SeqCst), 0);
}
