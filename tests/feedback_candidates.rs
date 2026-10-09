use agent_llm_mm::{
    adapters::sqlite::SqliteStore,
    application::{
        feedback_candidate::{self, *},
        recall_memory,
        search_memory::SearchMemoryRecord,
    },
    domain::{
        claim::{ClaimDraft, ClaimReference},
        event::{Event, EventReference},
        feedback::{FeedbackMetadata, FeedbackSourceKind, FeedbackVerificationResult},
        feedback_candidate::{FeedbackCandidate, FeedbackCandidateState, claim_version},
        types::{EventKind, MemoryScope, Mode, Namespace, Owner},
    },
    error::AppError,
    ports::feedback_candidate_store::FeedbackCandidateStore,
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

#[async_trait]
impl FeedbackCandidateStore for Deps {
    async fn get_feedback_candidate(
        &self,
        namespace: &Namespace,
        id: &str,
    ) -> Result<Option<FeedbackCandidate>, AppError> {
        self.store.get_feedback_candidate(namespace, id).await
    }
}
fn namespace() -> Namespace {
    Namespace::parse("project/alpha").unwrap()
}
fn action(candidate: &FeedbackCandidate, key: &str) -> FeedbackCandidateActionInput {
    FeedbackCandidateActionInput {
        namespace: namespace(),
        candidate_id: candidate.candidate_id.clone(),
        request_id: key.into(),
    }
}
impl Deps {
    async fn feedback(&self, id: &str, mutate: impl FnOnce(&mut FeedbackMetadata)) {
        let target = StoredClaim::new("original".into(), claim("old"), ClaimStatus::Active);
        let mut feedback = FeedbackMetadata {
            source_kind: FeedbackSourceKind::ToolReported,
            producer: "local-test-tool".into(),
            observed_target: "claim:original".into(),
            observed_version: Some(claim_version(&target).unwrap()),
            expected: "old".into(),
            actual: "new".into(),
            verification_method: "fixture-observation".into(),
            verification_result: FeedbackVerificationResult::Failed,
            // The synthetic observation asserts no applicability restrictions.
            // Producer authentication remains explicitly outside this contract.
            limitations: vec![],
            evidence_refs: vec![],
        };
        mutate(&mut feedback);
        self.store
            .append_event(StoredEvent::new(
                id.into(),
                Utc::now(),
                Event::new_with_namespace(
                    Owner::World,
                    namespace(),
                    EventKind::Observation,
                    "new observation",
                )
                .unwrap()
                .with_feedback(feedback)
                .unwrap(),
            ))
            .await
            .unwrap();
    }
    fn proposal(&self, key: &str, evidence: &[&str]) -> ProposeFeedbackCandidateInput {
        ProposeFeedbackCandidateInput {
            namespace: namespace(),
            target_claim_reference: ClaimReference::parse("claim:original").unwrap(),
            expected_target_version: claim_version(&StoredClaim::new(
                "original".into(),
                claim("old"),
                ClaimStatus::Active,
            ))
            .unwrap(),
            replacement_object: "new".into(),
            evidence_event_ids: evidence
                .iter()
                .map(|id| EventReference::parse(*id).unwrap())
                .collect(),
            summary: "Correct setting from reported observation".into(),
            request_id: key.into(),
        }
    }
    async fn prepared(&self, event: &str, key: &str) -> FeedbackCandidate {
        let candidate = feedback_candidate::propose(self, self.proposal(key, &[event]))
            .await
            .unwrap();
        let candidate =
            feedback_candidate::validate(self, action(&candidate, &format!("validate-{key}")))
                .await
                .unwrap();
        assert_eq!(candidate.state, FeedbackCandidateState::Validated);
        candidate
    }
}

#[tokio::test]
async fn external_feedback_candidate_commits_atomically_and_recall_uses_new_claim() {
    let deps = Deps::new().await;
    let version = feedback_candidate::get_target_version(
        &deps.store,
        FeedbackTargetInput {
            namespace: namespace(),
            target_claim_reference: ClaimReference::parse("original").unwrap(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        version.target_version,
        deps.proposal("probe", &[]).expected_target_version
    );
    deps.feedback("verified", |_| {}).await;
    let proposed = feedback_candidate::propose(&deps, deps.proposal("propose", &["verified"]))
        .await
        .unwrap();
    assert_eq!(proposed.state, FeedbackCandidateState::Proposed);
    assert!(!proposed.validation.passed);
    assert!(
        feedback_candidate::commit(&deps, action(&proposed, "premature"))
            .await
            .is_err()
    );
    deps.assert_no_correction().await;
    let validated = feedback_candidate::validate(&deps, action(&proposed, "validate"))
        .await
        .unwrap();
    let committed = feedback_candidate::commit(&deps, action(&validated, "commit"))
        .await
        .unwrap();
    assert_eq!(committed.state, FeedbackCandidateState::Committed);
    assert!(committed.validation.limitation.contains("semantic truth"));
    assert_eq!(
        feedback_candidate::commit(&deps, action(&validated, "commit"))
            .await
            .unwrap(),
        committed
    );
    assert_eq!(
        feedback_candidate::commit(&deps, action(&validated, "commit-again"))
            .await
            .unwrap(),
        committed
    );
    let original = deps
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: MemoryScope::for_namespace(namespace()),
            claim_reference: Some(ClaimReference::parse("original").unwrap()),
            status: None,
            mode: None,
            limit: 1,
        })
        .await
        .unwrap()
        .remove(0);
    assert_eq!(original.claim.status, ClaimStatus::Superseded);
    assert_eq!(original.claim.claim.object(), "old");
    assert_eq!(
        original
            .revision
            .replacement_claim_reference
            .unwrap()
            .claim_id(),
        committed.replacement_claim_id.as_deref().unwrap()
    );
    let recall = recall_memory::execute(
        &deps.store,
        recall_memory::RecallMemoryInput {
            namespace: namespace(),
            query: "setting".into(),
            limit: 10,
        },
    )
    .await
    .unwrap();
    let claims = recall
        .records
        .iter()
        .filter_map(|hit| {
            if let SearchMemoryRecord::Claim {
                id, object, status, ..
            } = &hit.record
            {
                Some((id, object, status))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].1, "new");
    assert_eq!(*claims[0].2, ClaimStatus::Active);
    let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM reflections), (SELECT count(*) FROM claims), (SELECT count(*) FROM evidence_links)").fetch_one(&deps.pool).await.unwrap();
    assert_eq!(counts, (1, 2, 1));
}

#[tokio::test]
async fn deterministic_validation_blocks_insufficient_unbound_and_model_reports() {
    for case in [
        "model",
        "inconclusive",
        "not_performed",
        "target",
        "version",
        "actual",
        "expected",
        "unbound_reference",
        "scope",
        "no_feedback",
        "missing",
        "empty",
    ] {
        let deps = Deps::new().await;
        deps.feedback("verified", |feedback| match case {
            "model" => feedback.source_kind = FeedbackSourceKind::ModelAsserted,
            "inconclusive" => {
                feedback.verification_result = FeedbackVerificationResult::Inconclusive
            }
            "not_performed" => {
                feedback.verification_result = FeedbackVerificationResult::NotPerformed
            }
            "target" => feedback.observed_target = "claim:other".into(),
            "version" => feedback.observed_version = None,
            "actual" => feedback.actual = "unrelated".into(),
            "expected" => feedback.expected = "unrelated".into(),
            "unbound_reference" => {
                feedback.evidence_refs = vec![EventReference::parse("evidence").unwrap()]
            }
            _ => {}
        })
        .await;
        if case == "scope" {
            sqlx::query("UPDATE events SET namespace = 'project/beta' WHERE event_id = 'verified'")
                .execute(&deps.pool)
                .await
                .unwrap();
        }
        let ids = match case {
            "empty" => vec![],
            "missing" => vec!["missing"],
            "no_feedback" => vec!["evidence"],
            _ => vec!["verified"],
        };
        let candidate = feedback_candidate::propose(&deps, deps.proposal("propose", &ids))
            .await
            .unwrap();
        let candidate = feedback_candidate::validate(&deps, action(&candidate, "validate"))
            .await
            .unwrap();
        assert_eq!(candidate.state, FeedbackCandidateState::Blocked, "{case}");
        assert!(!candidate.validation.reasons.is_empty(), "{case}");
        assert!(
            feedback_candidate::commit(&deps, action(&candidate, "commit"))
                .await
                .is_err(),
            "{case}"
        );
        deps.assert_no_correction().await;
    }
}

#[tokio::test]
async fn limited_reports_remain_inspectable_but_cannot_correct_claims() {
    for limitation in [
        "This result does not support changing the target claim",
        "Only observed in one local fixture",
    ] {
        let deps = Deps::new().await;
        deps.feedback("limited", |feedback| {
            feedback.limitations = vec![limitation.into()];
        })
        .await;
        let proposed = feedback_candidate::propose(&deps, deps.proposal("propose", &["limited"]))
            .await
            .unwrap();
        let blocked = feedback_candidate::validate(&deps, action(&proposed, "validate"))
            .await
            .unwrap();
        assert_eq!(blocked.state, FeedbackCandidateState::Blocked);
        assert!(!blocked.validation.passed);
        assert_eq!(blocked.validation.reasons.len(), 1);
        assert_eq!(
            blocked.validation.reasons[0].code,
            "limitations_require_review"
        );
        assert_eq!(
            feedback_candidate::validate(&deps, action(&proposed, "validate"))
                .await
                .unwrap(),
            blocked
        );
        let receipts_before: i64 = sqlx::query_scalar("SELECT count(*) FROM operation_log")
            .fetch_one(&deps.pool)
            .await
            .unwrap();
        assert!(
            feedback_candidate::commit(&deps, action(&blocked, "commit"))
                .await
                .is_err()
        );
        deps.assert_no_correction().await;
        let receipts_after: i64 = sqlx::query_scalar("SELECT count(*) FROM operation_log")
            .fetch_one(&deps.pool)
            .await
            .unwrap();
        assert_eq!(receipts_before, receipts_after);
        assert_eq!(
            deps.store
                .get_feedback_candidate(&namespace(), &blocked.candidate_id)
                .await
                .unwrap()
                .unwrap(),
            blocked
        );
        let retained: String = sqlx::query_scalar("SELECT json_extract(feedback_json, '$.limitations[0]') FROM events WHERE event_id='limited'")
            .fetch_one(&deps.pool).await.unwrap();
        assert_eq!(retained, limitation);
        let target_status: String =
            sqlx::query_scalar("SELECT status FROM claims WHERE claim_id='original'")
                .fetch_one(&deps.pool)
                .await
                .unwrap();
        assert_eq!(target_status, "active");
    }
}

#[tokio::test]
async fn proposal_retries_rejections_and_no_new_evidence_are_durable() {
    let deps = Deps::new().await;
    deps.feedback("verified", |_| {}).await;
    let candidate = feedback_candidate::propose(&deps, deps.proposal("propose", &["verified"]))
        .await
        .unwrap();
    assert_eq!(
        feedback_candidate::propose(&deps, deps.proposal("propose", &["verified"]))
            .await
            .unwrap(),
        candidate
    );
    assert_eq!(
        feedback_candidate::propose(&deps, deps.proposal("other-key", &["verified"]))
            .await
            .unwrap(),
        candidate
    );
    let rejected_input = RejectFeedbackCandidateInput {
        namespace: namespace(),
        candidate_id: candidate.candidate_id.clone(),
        reason: "Human rejects the report".into(),
        request_id: "reject".into(),
    };
    let rejected = feedback_candidate::reject(&deps, rejected_input.clone())
        .await
        .unwrap();
    assert_eq!(
        feedback_candidate::reject(&deps, rejected_input)
            .await
            .unwrap(),
        rejected
    );
    assert_eq!(
        feedback_candidate::propose(&deps, deps.proposal("after-rejection", &["verified"]))
            .await
            .unwrap(),
        rejected
    );
    let mut changed = deps.proposal("changed", &["verified"]);
    changed.summary = "try again".into();
    assert!(
        feedback_candidate::propose(&deps, changed)
            .await
            .unwrap_err()
            .to_string()
            .contains("no_new_evidence")
    );
    assert!(
        feedback_candidate::validate(&deps, action(&rejected, "validate-rejected"))
            .await
            .is_err()
    );
    assert!(
        feedback_candidate::commit(&deps, action(&rejected, "commit-rejected"))
            .await
            .is_err()
    );
    deps.feedback("new-evidence", |_| {}).await;
    let new_candidate =
        feedback_candidate::propose(&deps, deps.proposal("fresh", &["new-evidence", "verified"]))
            .await
            .unwrap();
    assert_ne!(new_candidate.candidate_id, candidate.candidate_id);
    let changed_key = deps.proposal("propose", &["new-evidence"]);
    assert!(
        feedback_candidate::propose(&deps, changed_key)
            .await
            .unwrap_err()
            .to_string()
            .contains("different payload")
    );
    deps.assert_no_correction().await;
}

#[tokio::test]
async fn commit_rechecks_version_content_status_scope_and_evidence_inside_transaction() {
    for mutation in [
        "UPDATE claims SET object = 'changed' WHERE claim_id = 'original'",
        "UPDATE claims SET status = 'disputed' WHERE claim_id = 'original'",
        "UPDATE claims SET namespace = 'project/beta' WHERE claim_id = 'original'",
        "UPDATE events SET namespace = 'project/beta' WHERE event_id = 'verified'",
        "UPDATE events SET feedback_json = json_set(feedback_json, '$.limitations', json_array('This result does not support changing the target claim')) WHERE event_id = 'verified'",
        "DELETE FROM events WHERE event_id = 'verified'",
    ] {
        let deps = Deps::new().await;
        deps.feedback("verified", |_| {}).await;
        let candidate = deps.prepared("verified", "propose").await;
        *deps.mutation_before_transaction.lock().unwrap() = Some(mutation);
        assert!(
            feedback_candidate::commit(&deps, action(&candidate, "commit"))
                .await
                .is_err(),
            "{mutation}"
        );
        let actual = deps
            .store
            .get_feedback_candidate(&namespace(), &candidate.candidate_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(actual.state, FeedbackCandidateState::Validated);
        deps.assert_no_correction().await;
        let blocked = feedback_candidate::validate(&deps, action(&candidate, "revalidate"))
            .await
            .unwrap();
        assert_eq!(blocked.state, FeedbackCandidateState::Blocked);
    }
}

#[tokio::test]
async fn competing_candidates_create_one_successor_and_retry_one_result() {
    let deps = Deps::new().await;
    deps.feedback("first-evidence", |_| {}).await;
    deps.feedback("second-evidence", |_| {}).await;
    let first = deps.prepared("first-evidence", "first").await;
    let second = deps.prepared("second-evidence", "second").await;
    let (a, b) = tokio::join!(
        feedback_candidate::commit(&deps, action(&first, "commit-first")),
        feedback_candidate::commit(&deps, action(&second, "commit-second"))
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let (winner, key) = if a.is_ok() {
        (&first, "commit-first")
    } else {
        (&second, "commit-second")
    };
    assert_eq!(
        feedback_candidate::commit(&deps, action(winner, key))
            .await
            .unwrap()
            .state,
        FeedbackCandidateState::Committed
    );
    let counts: (i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM reflections), (SELECT count(*) FROM claims), (SELECT count(*) FROM feedback_candidates WHERE state = 'committed')").fetch_one(&deps.pool).await.unwrap();
    assert_eq!(counts, (1, 2, 1));
}

#[tokio::test]
async fn candidate_and_receipt_write_failure_rolls_back_whole_correction_then_retry_succeeds() {
    for trigger in [
        "CREATE TRIGGER inject_feedback_failure BEFORE UPDATE ON feedback_candidates WHEN NEW.state = 'committed' BEGIN SELECT RAISE(ABORT, 'injected candidate failure'); END",
        "CREATE TRIGGER inject_feedback_failure BEFORE INSERT ON operation_log WHEN NEW.entrypoint = 'commit_feedback_candidate' BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END",
    ] {
        let deps = Deps::new().await;
        deps.feedback("verified", |_| {}).await;
        let candidate = deps.prepared("verified", "propose").await;
        sqlx::query(trigger).execute(&deps.pool).await.unwrap();
        assert!(
            feedback_candidate::commit(&deps, action(&candidate, "commit"))
                .await
                .is_err()
        );
        deps.assert_no_correction().await;
        assert_eq!(
            deps.store
                .get_feedback_candidate(&namespace(), &candidate.candidate_id)
                .await
                .unwrap()
                .unwrap(),
            candidate
        );
        sqlx::query("DROP TRIGGER inject_feedback_failure")
            .execute(&deps.pool)
            .await
            .unwrap();
        assert_eq!(
            feedback_candidate::commit(&deps, action(&candidate, "commit"))
                .await
                .unwrap()
                .state,
            FeedbackCandidateState::Committed
        );
    }
}

#[tokio::test]
async fn scoped_candidate_reads_and_actions_do_not_leak_other_namespaces() {
    let deps = Deps::new().await;
    deps.feedback("verified", |_| {}).await;
    let candidate = deps.prepared("verified", "propose").await;
    let wrong = Namespace::parse("project/beta").unwrap();
    assert!(
        feedback_candidate::get(
            &deps,
            GetFeedbackCandidateInput {
                namespace: wrong.clone(),
                candidate_id: candidate.candidate_id.clone()
            }
        )
        .await
        .is_err()
    );
    let mut input = action(&candidate, "validate-wrong");
    input.namespace = wrong.clone();
    assert!(feedback_candidate::validate(&deps, input).await.is_err());
    assert!(
        feedback_candidate::get_target_version(
            &deps.store,
            FeedbackTargetInput {
                namespace: wrong,
                target_claim_reference: ClaimReference::parse("original").unwrap()
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn caller_reported_definitive_verification_is_accepted_but_proposal_is_immutable() {
    let deps = Deps::new().await;
    deps.feedback("caller-observation", |feedback| {
        feedback.source_kind = FeedbackSourceKind::CallerReported;
        feedback.verification_result = FeedbackVerificationResult::Passed;
    })
    .await;
    let candidate = deps.prepared("caller-observation", "propose").await;
    let mut forged = candidate.clone();
    forged.proposal.summary = "silently replaced proposal".into();
    forged.revision += 1;
    let mut tx = deps.store.begin_reflection_transaction().await.unwrap();
    assert!(
        tx.update_feedback_candidate(&forged, candidate.revision)
            .await
            .is_err()
    );
    assert!(tx.commit().await.is_err());
    assert_eq!(
        deps.store
            .get_feedback_candidate(&namespace(), &candidate.candidate_id)
            .await
            .unwrap()
            .unwrap(),
        candidate
    );
    assert_eq!(
        feedback_candidate::commit(&deps, action(&candidate, "commit"))
            .await
            .unwrap()
            .state,
        FeedbackCandidateState::Committed
    );
}

#[tokio::test]
async fn competing_commit_and_reject_leave_one_terminal_outcome_without_partial_claims() {
    let deps = Deps::new().await;
    deps.feedback("verified", |_| {}).await;
    let candidate = deps.prepared("verified", "propose").await;
    let (commit, reject) = tokio::join!(
        feedback_candidate::commit(&deps, action(&candidate, "commit")),
        feedback_candidate::reject(
            &deps,
            RejectFeedbackCandidateInput {
                namespace: namespace(),
                candidate_id: candidate.candidate_id.clone(),
                reason: "Reject concurrent proposal".into(),
                request_id: "reject".into()
            }
        )
    );
    assert_eq!(usize::from(commit.is_ok()) + usize::from(reject.is_ok()), 1);
    let stored = deps
        .store
        .get_feedback_candidate(&namespace(), &candidate.candidate_id)
        .await
        .unwrap()
        .unwrap();
    let reflections: i64 = sqlx::query_scalar("SELECT count(*) FROM reflections")
        .fetch_one(&deps.pool)
        .await
        .unwrap();
    if stored.state == FeedbackCandidateState::Rejected {
        assert_eq!(reflections, 0);
        deps.assert_no_correction().await;
    } else {
        assert_eq!(stored.state, FeedbackCandidateState::Committed);
        assert_eq!(reflections, 1);
    }
}
