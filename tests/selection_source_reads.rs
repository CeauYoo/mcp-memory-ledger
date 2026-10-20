use agent_llm_mm::{
    adapters::sqlite::{
        SqliteStore,
        selection_reads::{hydrate_by_reference, hydrate_selection},
    },
    application::{recall_memory_enhanced::content_digest, selection_source::SelectionSource},
    domain::{experience::*, types::Namespace},
    ports::experience_store::ExperienceStore,
};
use sqlx::{Connection, SqlitePool};

const BUDGET: usize = 32768;

struct Fixture {
    _dir: tempfile::TempDir,
    pool: SqlitePool,
    store: SqliteStore,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("selection.sqlite");
        let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&path);
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        let fixture = Self {
            _dir: dir,
            pool,
            store,
        };
        fixture.event("e", "world", "project/a").await;
        fixture
    }
    async fn event(&self, id: &str, owner: &str, namespace: &str) {
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES(?,'2026-01-01T00:00:00Z',?,?,'observation','Observed cache test')")
            .bind(id).bind(owner).bind(namespace).execute(&self.pool).await.unwrap();
    }
    async fn episode(&self, id: &str) -> PersistedEpisode {
        self.store
            .create_episode(CreateEpisodeRequest {
                namespace: "project/a".into(),
                request_id: format!("create-{id}"),
                episode_id: id.into(),
                content: EpisodeContent {
                    title: "Cache test".into(),
                    objective: "Measure behavior".into(),
                    actions: vec!["Warm cache".into()],
                    observations: vec!["Test passed".into()],
                    outcome: "Staging passed".into(),
                    lesson: "Warm cache first".into(),
                    limitations: vec!["Staging only".into()],
                    source_event_refs: vec!["event:e".into()],
                },
            })
            .await
            .unwrap()
            .record
    }
    async fn candidate(&self) -> ExperienceCandidate {
        self.episode("ep").await;
        self.store
            .create_candidate(CreateCandidateRequest {
                namespace: "project/a".into(),
                request_id: "create-c".into(),
                candidate_id: "c".into(),
                content: ExperienceContent {
                    kind: ExperienceKind::Procedural,
                    title: "Cache procedure".into(),
                    statement: "Knowledge only".into(),
                    steps: vec!["Warm cache".into(), "Run checks".into()],
                    limitations: vec!["Staging only".into()],
                    source_episode_ids: vec!["ep".into()],
                },
            })
            .await
            .unwrap()
            .record
    }
    async fn activate(&self) -> ExperienceCandidate {
        self.candidate().await;
        self.store
            .update_candidate_status(UpdateCandidateStatusRequest {
                namespace: "project/a".into(),
                request_id: "activate-c".into(),
                candidate_id: "c".into(),
                expected_version: 1,
                status: ExperienceStatus::Active,
            })
            .await
            .unwrap()
            .record
    }
    async fn read(
        &self,
        reference: &str,
    ) -> Result<Option<SelectionSource>, agent_llm_mm::error::AppError> {
        let mut conn = self.pool.acquire().await.unwrap();
        hydrate_by_reference(&mut conn, &Namespace::for_project("a"), reference, BUDGET).await
    }
    async fn replace_episode(&self, record: &PersistedEpisode) {
        sqlx::query("UPDATE experience_episodes SET payload_json=? WHERE episode_id=?")
            .bind(serde_json::to_string(record).unwrap())
            .bind(&record.episode_id)
            .execute(&self.pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn typed_projection_keeps_memory_digest_and_disjoint_episode_reference() {
    let f = Fixture::new().await;
    let memory = f.read("event:e").await.unwrap().unwrap();
    let SelectionSource::Memory(ref original) = memory else {
        panic!("memory expected")
    };
    assert_eq!(memory.reference(), "event:e");
    assert_eq!(memory.kind(), "event");
    assert_eq!(
        memory.content_digest().unwrap(),
        content_digest(original).unwrap()
    );
    assert_eq!(memory.embedding_text(), "Observed cache test");
    f.episode("ep").await;
    let episode = f.read("experience_episode:ep").await.unwrap().unwrap();
    assert!(matches!(episode, SelectionSource::Episode(_)));
    assert_eq!(episode.reference(), "experience_episode:ep");
    assert_eq!(episode.kind(), "persisted_episode");
    assert!(episode.embedding_text().contains("Warm cache first"));
    assert!(episode.embedding_text().contains("Staging only"));
    assert_eq!(episode.content_digest().unwrap().len(), 64);
    assert!(f.read("episode:ep").await.is_err());
}

#[tokio::test]
async fn only_current_active_candidates_are_knowledge() {
    let f = Fixture::new().await;
    f.candidate().await;
    assert!(f.read("experience_candidate:c").await.unwrap().is_none());
    let active = f
        .store
        .update_candidate_status(UpdateCandidateStatusRequest {
            namespace: "project/a".into(),
            request_id: "activate".into(),
            candidate_id: "c".into(),
            expected_version: 1,
            status: ExperienceStatus::Active,
        })
        .await
        .unwrap()
        .record;
    let projected = f.read("experience_candidate:c").await.unwrap().unwrap();
    assert_eq!(projected.version(), Some(2));
    assert_eq!(projected.reference(), "experience_candidate:c");
    assert!(projected.embedding_text().contains("Run checks"));
    f.store
        .revise_candidate(ReviseCandidateRequest {
            namespace: "project/a".into(),
            request_id: "revise".into(),
            candidate_id: "c".into(),
            expected_version: active.version,
            content: active.content,
        })
        .await
        .unwrap();
    assert!(f.read("experience_candidate:c").await.unwrap().is_none());
    let historical_active: i64 = sqlx::query_scalar("SELECT count(*) FROM experience_candidate_versions WHERE candidate_id='c' AND status='active'")
        .fetch_one(&f.pool).await.unwrap();
    assert_eq!(historical_active, 1);
}

#[tokio::test]
async fn all_root_kinds_are_exactly_scoped_and_nested_reads_do_not_rollback_callers() {
    let f = Fixture::new().await;
    f.activate().await;
    let mut conn = f.pool.acquire().await.unwrap();
    for reference in ["event:e", "experience_episode:ep", "experience_candidate:c"] {
        assert!(
            hydrate_by_reference(&mut conn, &Namespace::for_project("b"), reference, BUDGET)
                .await
                .unwrap()
                .is_none()
        );
    }
    let rowid: i64 =
        sqlx::query_scalar("SELECT rowid FROM experience_candidates WHERE candidate_id='c'")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert!(matches!(
        hydrate_selection(&mut conn, &Namespace::for_project("a"), 3, rowid, BUDGET)
            .await
            .unwrap(),
        Some(SelectionSource::Experience(_))
    ));
    let mut tx = conn.begin().await.unwrap();
    sqlx::query("UPDATE events SET summary='outer transaction' WHERE event_id='e'")
        .execute(&mut *tx)
        .await
        .unwrap();
    let record = hydrate_by_reference(&mut tx, &Namespace::for_project("a"), "event:e", BUDGET)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.embedding_text(), "outer transaction");
    let summary: String = sqlx::query_scalar("SELECT summary FROM events WHERE event_id='e'")
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert_eq!(summary, "outer transaction");
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn episode_payload_and_normalized_edges_must_agree() {
    let f = Fixture::new().await;
    f.episode("ep").await;
    sqlx::query("DELETE FROM experience_episode_sources WHERE episode_id='ep'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_episode:ep").await.is_err());
    sqlx::query("INSERT INTO experience_episode_sources(episode_id,event_id) VALUES('ep','e')")
        .execute(&f.pool)
        .await
        .unwrap();
    f.event("extra", "world", "project/a").await;
    sqlx::query("INSERT INTO experience_episode_sources(episode_id,event_id) VALUES('ep','extra')")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_episode:ep").await.is_err());
}

#[tokio::test]
async fn candidate_payload_and_normalized_episode_edges_must_agree() {
    let f = Fixture::new().await;
    f.activate().await;
    sqlx::query("DELETE FROM experience_candidate_sources WHERE candidate_id='c' AND version=2")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_candidate:c").await.is_err());
}

#[tokio::test]
async fn source_events_must_have_canonical_owner_and_exact_namespace() {
    let f = Fixture::new().await;
    f.activate().await;
    sqlx::query("UPDATE events SET owner='unknown' WHERE event_id='e'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_episode:ep").await.is_err());
    assert!(f.read("experience_candidate:c").await.is_err());
    sqlx::query("UPDATE events SET owner='world',namespace='project/b' WHERE event_id='e'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_candidate:c").await.is_err());
}

#[tokio::test]
async fn persisted_payload_identity_and_status_are_not_trusted() {
    let f = Fixture::new().await;
    let mut candidate = f.activate().await;
    for version in [1, 3] {
        candidate.version = version;
        sqlx::query("UPDATE experience_candidate_versions SET payload_json=? WHERE candidate_id='c' AND version=2")
            .bind(serde_json::to_string(&candidate).unwrap()).execute(&f.pool).await.unwrap();
        assert!(f.read("experience_candidate:c").await.is_err());
    }
    candidate.version = 2;
    candidate.status = ExperienceStatus::Pending;
    sqlx::query("UPDATE experience_candidate_versions SET payload_json=? WHERE candidate_id='c' AND version=2")
        .bind(serde_json::to_string(&candidate).unwrap()).execute(&f.pool).await.unwrap();
    assert!(f.read("experience_candidate:c").await.is_err());
    sqlx::query("UPDATE experience_episodes SET payload_json=json_set(payload_json,'$.episode_id','other') WHERE episode_id='ep'")
        .execute(&f.pool).await.unwrap();
    assert!(f.read("experience_episode:ep").await.is_err());
}

#[tokio::test]
async fn oversized_root_and_transitive_payloads_are_skipped_before_json_decode() {
    let f = Fixture::new().await;
    f.activate().await;
    // Deliberately invalid JSON: an error would prove decoding happened before
    // the mandatory length preflight.
    sqlx::query("UPDATE experience_episodes SET payload_json=? WHERE episode_id='ep'")
        .bind("{".repeat(BUDGET + 1))
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_episode:ep").await.unwrap().is_none());
    assert!(f.read("experience_candidate:c").await.unwrap().is_none());
    sqlx::query("UPDATE experience_candidate_versions SET payload_json=? WHERE candidate_id='c' AND version=2")
        .bind("{".repeat(BUDGET + 1)).execute(&f.pool).await.unwrap();
    assert!(f.read("experience_candidate:c").await.unwrap().is_none());
}

#[tokio::test]
async fn non_text_and_malformed_bounded_payloads_fail_closed() {
    let f = Fixture::new().await;
    f.episode("ep").await;
    sqlx::query("UPDATE experience_episodes SET payload_json=x'7b' WHERE episode_id='ep'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_episode:ep").await.is_err());
    sqlx::query("UPDATE experience_episodes SET payload_json='{' WHERE episode_id='ep'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_episode:ep").await.is_err());
}

#[tokio::test]
async fn cumulative_source_payload_bytes_are_bounded_without_truncation() {
    let f = Fixture::new().await;
    let mut candidate = f.activate().await;
    let mut extra = f.episode("extra").await;
    let mut first = f
        .store
        .get_episode(GetEpisodeRequest {
            namespace: "project/a".into(),
            episode_id: "ep".into(),
        })
        .await
        .unwrap()
        .unwrap();
    for record in [&mut first, &mut extra] {
        record.content.actions = vec!["x".repeat(3500); 5];
        f.replace_episode(record).await;
    }
    // Each source is safe alone, but their combined payloads are not.
    assert!(f.read("experience_episode:ep").await.unwrap().is_some());
    assert!(f.read("experience_episode:extra").await.unwrap().is_some());
    candidate.content.source_episode_ids.push("extra".into());
    sqlx::query("UPDATE experience_candidate_versions SET payload_json=? WHERE candidate_id='c' AND version=2")
        .bind(serde_json::to_string(&candidate).unwrap()).execute(&f.pool).await.unwrap();
    sqlx::query("INSERT INTO experience_candidate_sources(candidate_id,version,episode_id) VALUES('c',2,'extra')")
        .execute(&f.pool).await.unwrap();
    assert!(f.read("experience_candidate:c").await.unwrap().is_none());
}

#[tokio::test]
async fn provenance_has_a_cumulative_64_edge_ceiling() {
    let f = Fixture::new().await;
    f.activate().await;
    let mut episode = f
        .store
        .get_episode(GetEpisodeRequest {
            namespace: "project/a".into(),
            episode_id: "ep".into(),
        })
        .await
        .unwrap()
        .unwrap();
    for index in 1..64 {
        let id = format!("e{index}");
        f.event(&id, "world", "project/a").await;
        episode
            .content
            .source_event_refs
            .push(format!("event:{id}"));
        sqlx::query("INSERT INTO experience_episode_sources(episode_id,event_id) VALUES('ep',?)")
            .bind(&id)
            .execute(&f.pool)
            .await
            .unwrap();
    }
    f.replace_episode(&episode).await;
    assert!(f.read("experience_episode:ep").await.unwrap().is_some());
    // 1 candidate->episode plus 64 episode->event edges is not truncated.
    assert!(f.read("experience_candidate:c").await.unwrap().is_none());
    f.event("e65", "world", "project/a").await;
    sqlx::query("INSERT INTO experience_episode_sources(episode_id,event_id) VALUES('ep','e65')")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_episode:ep").await.unwrap().is_none());
}

#[tokio::test]
async fn budgets_count_utf8_bytes_and_never_emit_partial_records() {
    let f = Fixture::new().await;
    let mut episode = f.episode("ep").await;
    episode.content.lesson = "界".repeat(1000);
    f.replace_episode(&episode).await;
    let mut conn = f.pool.acquire().await.unwrap();
    assert!(
        hydrate_by_reference(
            &mut conn,
            &Namespace::for_project("a"),
            "experience_episode:ep",
            3000
        )
        .await
        .unwrap()
        .is_none()
    );
    let record = hydrate_by_reference(
        &mut conn,
        &Namespace::for_project("a"),
        "experience_episode:ep",
        BUDGET,
    )
    .await
    .unwrap()
    .unwrap();
    let SelectionSource::Episode(record) = record else {
        panic!("episode expected")
    };
    assert_eq!(record.content.lesson, episode.content.lesson);
}

#[tokio::test]
async fn active_claims_keep_the_existing_authoritative_projection() {
    let f = Fixture::new().await;
    sqlx::raw_sql("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES('claim-a','world','project/a','cache','needs','warming','observed','active'); INSERT INTO evidence_links(claim_id,event_id) VALUES('claim-a','e');")
        .execute(&f.pool).await.unwrap();
    let source = f.read("claim:claim-a").await.unwrap().unwrap();
    assert_eq!(source.reference(), "claim:claim-a");
    assert_eq!(source.embedding_text(), "cache\nneeds\nwarming");
    let SelectionSource::Memory(record) = &source else {
        panic!("memory expected")
    };
    assert_eq!(
        source.content_digest().unwrap(),
        content_digest(record).unwrap()
    );
    sqlx::query("UPDATE claims SET status='superseded' WHERE claim_id='claim-a'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("claim:claim-a").await.unwrap().is_none());
}

#[tokio::test]
async fn source_episode_namespace_and_oversized_edge_metadata_are_preflighted() {
    let f = Fixture::new().await;
    f.activate().await;
    sqlx::query("UPDATE experience_episodes SET namespace='project/b' WHERE episode_id='ep'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.read("experience_candidate:c").await.is_err());
    sqlx::query("UPDATE experience_episodes SET namespace='project/a' WHERE episode_id='ep'")
        .execute(&f.pool)
        .await
        .unwrap();
    let huge = "x".repeat(BUDGET + 1);
    f.event(&huge, "world", "project/a").await;
    sqlx::query("INSERT INTO experience_episode_sources(episode_id,event_id) VALUES('ep',?)")
        .bind(huge)
        .execute(&f.pool)
        .await
        .unwrap();
    // No partial graph and no identifier-validation error after materialization.
    assert!(f.read("experience_episode:ep").await.unwrap().is_none());
    assert!(f.read("experience_candidate:c").await.unwrap().is_none());
}

#[tokio::test]
async fn memory_provenance_also_obeys_the_unified_edge_ceiling() {
    let f = Fixture::new().await;
    for index in 0..64 {
        sqlx::query("INSERT INTO episode_events(episode_reference,event_id) VALUES(?,'e')")
            .bind(format!("episode:{index}"))
            .execute(&f.pool)
            .await
            .unwrap();
    }
    assert!(f.read("event:e").await.unwrap().is_some());
    sqlx::query(
        "INSERT INTO episode_events(episode_reference,event_id) VALUES('episode:overflow','e')",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(f.read("event:e").await.unwrap().is_none());
}
