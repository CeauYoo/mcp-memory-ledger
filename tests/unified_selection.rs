//! Synthetic, offline T12 tests. No model endpoint or credentials.
use agent_llm_mm::{
    adapters::{
        local_embedding::LocalEmbeddingConfig,
        sqlite::{
            SqliteStore, inspect_database, reset_semantic_index,
            selection_reads::hydrate_by_reference, semantic_index::SemanticIndex,
        },
    },
    application::{build_task_context::BuildTaskContextInput, task},
    domain::{
        recall_applicability::DeclareRecallApplicabilityRequest, task::TaskContextRequest,
        types::Namespace,
    },
    error::AppError,
    ports::{
        embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
        experience_store::ExperienceStore,
    },
    support::unified_selection,
};
use async_trait::async_trait;
use serde_json::json;
use sqlx::{Row, SqlitePool};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct Fake {
    profile: EmbeddingProfile,
    docs: Arc<AtomicUsize>,
    queries: Arc<AtomicUsize>,
}
#[async_trait]
impl EmbeddingPort for Fake {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    async fn embed(
        &self,
        _: &Namespace,
        purpose: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        match purpose {
            EmbeddingPurpose::Document => {
                self.docs.fetch_add(1, Ordering::SeqCst);
            }
            EmbeddingPurpose::Query => {
                self.queries.fetch_add(1, Ordering::SeqCst);
            }
        };
        Ok(texts
            .iter()
            .map(|s| {
                if purpose == EmbeddingPurpose::Query
                    || s.contains("target")
                    || s.contains("数据库")
                {
                    vec![1., 0.]
                } else {
                    vec![0., 1.]
                }
            })
            .collect())
    }
}
struct F {
    _dir: tempfile::TempDir,
    url: String,
    store: SqliteStore,
    pool: SqlitePool,
    index: SemanticIndex,
    docs: Arc<AtomicUsize>,
    queries: Arc<AtomicUsize>,
}
impl F {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(
            &dir.path().canonicalize().unwrap().join("test.sqlite"),
        );
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        let profile = EmbeddingProfile {
            model: "fake".into(),
            model_digest: "a".repeat(64),
            dimensions: 2,
            document_prefix: "d:".into(),
            query_prefix: "q:".into(),
        };
        let config = LocalEmbeddingConfig {
            enabled: true,
            local_only_declared: true,
            namespace_allowlist: vec!["project/a".into()],
            profile: profile.clone(),
            ..Default::default()
        };
        let docs = Arc::new(AtomicUsize::new(0));
        let queries = Arc::new(AtomicUsize::new(0));
        let index = SemanticIndex::new(
            store.clone(),
            config,
            Arc::new(Fake {
                profile,
                docs: docs.clone(),
                queries: queries.clone(),
            }),
        )
        .unwrap();
        Self {
            _dir: dir,
            url,
            store,
            pool,
            index,
            docs,
            queries,
        }
    }
    fn ns(&self) -> Namespace {
        Namespace::for_project("a")
    }
    async fn event(&self, id: &str, text: &str) {
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES(?,'2026-01-01T00:00:00Z','world','project/a','observation',?)").bind(id).bind(text).execute(&self.pool).await.unwrap();
    }
    async fn sources(&self) {
        self.event("source", "source").await;
        self.store.create_episode(serde_json::from_value(json!({"namespace":"project/a","request_id":"ep","episode_id":"ep","content":{"title":"target episode","objective":"数据库","actions":[],"observations":["observed"],"outcome":"target result","lesson":"target lesson","limitations":["caller reported"],"source_event_refs":["event:source"]}})).unwrap()).await.unwrap();
        self.store.create_candidate(serde_json::from_value(json!({"namespace":"project/a","request_id":"candidate","candidate_id":"c","content":{"kind":"procedural","title":"target experience","statement":"target objective","steps":["inert step"],"limitations":["review first"],"source_episode_ids":["ep"]}})).unwrap()).await.unwrap();
        self.store.update_candidate_status(serde_json::from_value(json!({"namespace":"project/a","request_id":"activate","candidate_id":"c","expected_version":1,"status":"active"})).unwrap()).await.unwrap();
    }
    async fn build(&self) {
        let mut s = self.index.start(&self.ns()).await.unwrap();
        while !s.complete {
            s = self
                .index
                .resume(&self.ns(), &s.generation, 4)
                .await
                .unwrap();
        }
    }
    fn input(&self, limit: usize, max_bytes: usize) -> BuildTaskContextInput {
        BuildTaskContextInput {
            namespace: self.ns(),
            query: "target".into(),
            limit,
            max_bytes,
        }
    }
}
#[tokio::test]
async fn unified_sources_reuse_one_query_and_keep_versions_conditions_and_limits() {
    let f = F::new().await;
    f.sources().await;
    for n in 0..70 {
        f.event(
            &format!("e{n:03}"),
            if n == 69 { "target fact" } else { "hay" },
        )
        .await;
    }
    let mut conn = f.pool.acquire().await.unwrap();
    let candidate = hydrate_by_reference(&mut conn, &f.ns(), "experience_candidate:c", 32768)
        .await
        .unwrap()
        .unwrap();
    drop(conn);
    let declaration:DeclareRecallApplicabilityRequest=serde_json::from_value(json!({"request_id":"conditions","expected_revision":0,"declaration":{"namespace":"project/a","reference":"experience_candidate:c","content_digest":candidate.content_digest().unwrap(),"valid_from":null,"valid_until":null,"conditions":["check rollback path"],"environment":[],"typed_environment":{"os":"linux"}}})).unwrap();
    f.store
        .declare_recall_applicability(declaration)
        .await
        .unwrap();
    f.build().await;
    let docs = f.docs.load(Ordering::SeqCst);
    let result = unified_selection::execute(&f.index, f.input(8, 32768), &Default::default())
        .await
        .unwrap();
    assert_eq!(f.queries.load(Ordering::SeqCst), 1);
    assert_eq!(f.docs.load(Ordering::SeqCst), docs);
    assert_eq!(result.episodes.len(), 1);
    assert_eq!(result.experiences.len(), 1);
    assert_eq!(result.experiences[0].version, 2);
    let report = result.report.unwrap();
    assert_eq!(report["ranking"], "bounded_partial_index_scan");
    assert!(report.to_string().contains("check rollback path"));
    assert!(report.to_string().contains("unknown"));
    let mismatch = [(
        "os".into(),
        agent_llm_mm::domain::recall_applicability::EnvironmentValue::Text("windows".into()),
    )]
    .into();
    let result = unified_selection::execute(&f.index, f.input(8, 32768), &mismatch)
        .await
        .unwrap();
    assert!(result.experiences.is_empty());
    assert_eq!(
        result.report.unwrap()["not_selected"]["environment_mismatch"],
        1
    );
    let small = unified_selection::execute(&f.index, f.input(8, 1400), &Default::default())
        .await
        .unwrap();
    assert!(serde_json::to_vec(&small).unwrap().len() <= 1400);
    assert!(
        small.report.unwrap()["not_selected"]["byte_budget"]
            .as_u64()
            .unwrap()
            > 0
    );
}
#[tokio::test]
async fn revised_experience_invalidates_index_and_pending_revision_is_not_selected() {
    let f = F::new().await;
    f.sources().await;
    f.build().await;
    f.store.rollback_candidate(serde_json::from_value(json!({"namespace":"project/a","request_id":"rollback","candidate_id":"c","expected_version":2,"target_version":1})).unwrap()).await.unwrap();
    assert!(f.index.status(&f.ns()).await.unwrap().stale);
    let unavailable = unified_selection::execute(&f.index, f.input(8, 32768), &Default::default())
        .await
        .unwrap();
    assert!(unavailable.experiences.is_empty());
    assert_eq!(f.queries.load(Ordering::SeqCst), 0);
    f.build().await;
    let result = unified_selection::execute(&f.index, f.input(8, 32768), &Default::default())
        .await
        .unwrap();
    assert!(result.experiences.is_empty());
    assert_eq!(result.episodes.len(), 1);
}
#[tokio::test]
async fn future_valid_boundary_invalidates_without_source_write() {
    let f = F::new().await;
    f.event("future", "target future").await;
    let mut conn = f.pool.acquire().await.unwrap();
    let source = hydrate_by_reference(&mut conn, &f.ns(), "event:future", 32768)
        .await
        .unwrap()
        .unwrap();
    drop(conn);
    let boundary = chrono::Utc::now() + chrono::Duration::milliseconds(500);
    f.store.declare_recall_applicability(serde_json::from_value(json!({"request_id":"future","expected_revision":0,"declaration":{"namespace":"project/a","reference":"event:future","content_digest":source.content_digest().unwrap(),"valid_from":boundary.to_rfc3339(),"valid_until":null,"conditions":[],"environment":[]}})).unwrap()).await.unwrap();
    f.build().await;
    let status = f.index.status(&f.ns()).await.unwrap();
    assert_eq!(status.indexed, 0);
    assert!(status.next_valid_from.is_some());
    tokio::time::sleep(std::time::Duration::from_millis(550)).await;
    assert!(f.index.status(&f.ns()).await.unwrap().stale);
    f.build().await;
    assert_eq!(f.index.status(&f.ns()).await.unwrap().indexed, 1);
}
#[tokio::test]
async fn task_keeps_mandatory_constraints_and_unified_optional_sources() {
    let f = F::new().await;
    f.sources().await;
    f.build().await;
    task::create_task(&f.store,serde_json::from_value(json!({"namespace":"project/a","request_id":"task","task_id":"t","title":"target","reason":"test","source":{"kind":"caller_reported"},"requirements":[{"requirement_id":"hard","kind":"prohibition","text":"Never deploy without approval","state":"reported","source":{"kind":"caller_reported"}}]})).unwrap()).await.unwrap();
    // Task writes do not invalidate the source-only index.
    let result = task::build_task_resume_context_with_selection(
        &f.store,
        TaskContextRequest {
            reported_environment: Default::default(),
            namespace: "project/a".into(),
            task_id: "t".into(),
            max_bytes: 32768,
        },
        |input| async { unified_selection::execute(&f.index, input, &Default::default()).await },
        5000,
        true,
    )
    .await
    .unwrap();
    assert!(result.can_resume);
    assert_eq!(
        result.mandatory_requirements[0].text,
        "Never deploy without approval"
    );
    assert_eq!(result.experiences.len(), 1);
    assert_eq!(result.episodes.len(), 1);
    assert!(result.optional_selection.is_some());
    assert_eq!(result.authority, "knowledge_only_no_execution_authority");
    assert!(result.serialized_bytes <= 32768);
    let budget = task::build_task_resume_context_with_selection(
        &f.store,
        TaskContextRequest {
            reported_environment: Default::default(),
            namespace: "project/a".into(),
            task_id: "t".into(),
            max_bytes: 2048,
        },
        |input| async { unified_selection::execute(&f.index, input, &Default::default()).await },
        5000,
        true,
    )
    .await
    .unwrap();
    assert_eq!(budget.mandatory_requirements.len(), 1);
    assert!(budget.serialized_bytes <= 2048);
}
#[tokio::test]
async fn canonical_v1_requires_explicit_reset_without_changing_authority() {
    let f = F::new().await;
    f.event("one", "target").await;
    f.build().await;
    let sql: String =
        sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE name='derived_semantic_runs'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    let rows=sqlx::query("SELECT name,type FROM sqlite_master WHERE name LIKE 'derived_semantic_experience_%' OR name IN ('derived_semantic_episodes_scope','derived_semantic_candidates_scope')").fetch_all(&f.pool).await.unwrap();
    for row in rows {
        let name: String = row.get("name");
        let kind: String = row.get("type");
        sqlx::query(&format!("DROP {kind} {name}"))
            .execute(&f.pool)
            .await
            .unwrap();
    }
    sqlx::query("DROP TABLE derived_semantic_runs")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query(&sql.replace(",next_valid_from TEXT", ""))
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(inspect_database(&f.url).await.unwrap().is_current());
    assert!(
        f.index
            .start(&f.ns())
            .await
            .unwrap_err()
            .to_string()
            .contains("explicit")
    );
    reset_semantic_index(&f.url).await.unwrap();
    f.build().await;
    assert_eq!(f.index.status(&f.ns()).await.unwrap().index_version, 2);
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(events, 1);
}
#[tokio::test]
async fn bounded_scan_is_not_mislabeled_global_and_all_experience_tables_invalidate() {
    let f = F::new().await;
    f.sources().await;
    for table in [
        "experience_episodes",
        "experience_episode_sources",
        "experience_candidates",
        "experience_candidate_versions",
        "experience_candidate_sources",
    ] {
        f.build().await;
        let column = if table == "experience_episodes" || table == "experience_episode_sources" {
            "episode_id"
        } else {
            "candidate_id"
        };
        sqlx::query(&format!("UPDATE {table} SET {column}={column}"))
            .execute(&f.pool)
            .await
            .unwrap();
        assert!(f.index.status(&f.ns()).await.unwrap().stale, "{table}");
    }
    for n in 0..260 {
        f.event(&format!("many{n:03}"), "target").await;
    }
    f.build().await;
    let result = unified_selection::execute(&f.index, f.input(8, 32768), &Default::default())
        .await
        .unwrap();
    let report = result.report.unwrap();
    assert_eq!(report["examined"], 66);
    assert_eq!(report["kind_coverage"]["event"]["examined"], 64);
    assert_eq!(report["unexamined_remaining"], true);
    assert_eq!(report["ranking"], "bounded_partial_index_scan");
    assert_eq!(f.queries.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn unified_task_final_source_recheck_and_timeout_fail_closed() {
    let f = F::new().await;
    f.sources().await;
    f.build().await;
    task::create_task(&f.store,serde_json::from_value(json!({"namespace":"project/a","request_id":"task","task_id":"t","title":"target","reason":"test","source":{"kind":"caller_reported"},"requirements":[{"requirement_id":"hard","kind":"prohibition","text":"Do not execute knowledge","state":"reported","source":{"kind":"caller_reported"}}]})).unwrap()).await.unwrap();
    let request = || TaskContextRequest {
        reported_environment: Default::default(),
        namespace: "project/a".into(),
        task_id: "t".into(),
        max_bytes: 32768,
    };
    let result = task::build_task_resume_context_with_selection(
        &f.store,
        request(),
        |input| async {
            let selected = unified_selection::execute(&f.index, input, &Default::default()).await?;
            sqlx::query("UPDATE events SET summary='changed' WHERE event_id='source'")
                .execute(&f.pool)
                .await
                .unwrap();
            Ok(selected)
        },
        5000,
        true,
    )
    .await
    .unwrap();
    assert_eq!(result.mandatory_requirements.len(), 1);
    assert!(result.experiences.is_empty());
    assert!(result.episodes.is_empty());
    assert!(result.related_memory.is_none());
    assert!(result.optional_selection.is_none());
    assert!(
        result
            .diagnostics
            .iter()
            .any(|s| s.contains("sources_changed"))
    );
    let timeout = task::build_task_resume_context_with_selection(
        &f.store,
        request(),
        |_| async {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            Err(AppError::Message("not reached".into()))
        },
        100,
        true,
    )
    .await
    .unwrap();
    assert_eq!(timeout.mandatory_requirements.len(), 1);
    assert!(
        timeout
            .diagnostics
            .iter()
            .any(|s| s.contains("deadline_exceeded"))
    );
    assert!(timeout.serialized_bytes <= 32768);
}
#[tokio::test]
async fn long_experience_reference_and_typed_task_environment_are_bounded() {
    let f = F::new().await;
    f.sources().await;
    let id = "x".repeat(256);
    let candidate=f.store.create_candidate(serde_json::from_value(json!({"namespace":"project/a","request_id":"long","candidate_id":id,"content":{"kind":"semantic","title":"target","statement":"target","steps":[],"limitations":[],"source_episode_ids":["ep"]}})).unwrap()).await.unwrap();
    let active=f.store.update_candidate_status(serde_json::from_value(json!({"namespace":"project/a","request_id":"long-active","candidate_id":candidate.record.candidate_id,"expected_version":1,"status":"active"})).unwrap()).await.unwrap();
    let projection =
        agent_llm_mm::application::selection_source::SelectionSource::Experience(active.record);
    let declaration=serde_json::from_value(json!({"request_id":"long-declaration","expected_revision":0,"declaration":{"namespace":"project/a","reference":projection.reference(),"content_digest":projection.content_digest().unwrap(),"valid_from":null,"valid_until":null,"conditions":["review"],"environment":[]}})).unwrap();
    f.store
        .declare_recall_applicability(declaration)
        .await
        .unwrap();
    f.build().await;
    let result = unified_selection::execute(&f.index, f.input(8, 32768), &Default::default())
        .await
        .unwrap();
    assert_eq!(result.experiences.len(), 2);
    assert!(
        result
            .report
            .unwrap()
            .to_string()
            .contains(&format!("experience_candidate:{id}"))
    );
    let parsed:TaskContextRequest=serde_json::from_value(json!({"namespace":"project/a","task_id":"t","reported_environment":{"os":"linux","ready":true}})).unwrap();
    assert_eq!(parsed.reported_environment.len(), 2);
    assert!(
        serde_json::from_value::<TaskContextRequest>(
            json!({"namespace":"project/a","task_id":"t","reported_environment":{"number":1.5}})
        )
        .is_err()
    );
}

#[tokio::test]
async fn validity_expiry_during_final_task_assembly_omits_optional_bundle() {
    let f = F::new().await;
    f.sources().await;
    let mut conn = f.pool.acquire().await.unwrap();
    let source = hydrate_by_reference(&mut conn, &f.ns(), "event:source", 32768)
        .await
        .unwrap()
        .unwrap();
    drop(conn);
    let boundary = chrono::Utc::now() + chrono::Duration::seconds(2);
    f.store.declare_recall_applicability(serde_json::from_value(json!({"request_id":"expires","expected_revision":0,"declaration":{"namespace":"project/a","reference":"event:source","content_digest":source.content_digest().unwrap(),"valid_from":null,"valid_until":boundary.to_rfc3339(),"conditions":[],"environment":[]}})).unwrap()).await.unwrap();
    f.build().await;
    task::create_task(&f.store,serde_json::from_value(json!({"namespace":"project/a","request_id":"task","task_id":"t","title":"target","reason":"test","source":{"kind":"caller_reported"},"requirements":[{"requirement_id":"hard","kind":"prohibition","text":"Do not execute knowledge","state":"reported","source":{"kind":"caller_reported"}}]})).unwrap()).await.unwrap();
    let result = task::build_task_resume_context_with_selection(
        &f.store,
        TaskContextRequest {
            reported_environment: Default::default(),
            namespace: "project/a".into(),
            task_id: "t".into(),
            max_bytes: 32768,
        },
        |input| async {
            let selection =
                unified_selection::execute(&f.index, input, &Default::default()).await?;
            assert!(selection.related_memory.is_some());
            tokio::time::sleep(
                (boundary - chrono::Utc::now()).to_std().unwrap_or_default()
                    + std::time::Duration::from_millis(25),
            )
            .await;
            Ok(selection)
        },
        5000,
        true,
    )
    .await
    .unwrap();
    assert_eq!(result.mandatory_requirements.len(), 1);
    assert!(result.related_memory.is_none());
    assert!(result.episodes.is_empty());
    assert!(result.experiences.is_empty());
    assert!(result.optional_selection.is_none());
    assert!(
        result
            .diagnostics
            .iter()
            .any(|s| s.contains("applicability_changed_during_final"))
    );
}

#[tokio::test]
async fn owner_provenance_survives_unified_selection_failure_and_timeout() {
    let f = F::new().await;
    f.sources().await;
    f.build().await;
    task::create_task(&f.store, serde_json::from_value(json!({
        "namespace":"project/a", "request_id":"owner-context", "task_id":"owner-task",
        "title":"target", "reason":"synthetic", "source":{"kind":"caller_reported"},
        "requirements":[{"requirement_id":"base", "kind":"requirement", "text":"retain initial goal", "state":"reported", "source":{"kind":"caller_reported"}}]
    })).unwrap()).await.unwrap();
    f.store.attest_task_owner(serde_json::from_value(json!({
        "namespace":"project/a", "task_id":"owner-task", "import_event_id":"owner-unified",
        "expected_version":1, "evidence_text":"Exact synthetic owner evidence",
        "change":{"operation":"append", "requirement_id":"protected", "kind":"prohibition", "exact_text":"禁止部署；保留原始要求。"}
    })).unwrap()).await.unwrap();
    let request = |budget| TaskContextRequest {
        namespace: "project/a".into(),
        task_id: "owner-task".into(),
        max_bytes: budget,
        reported_environment: Default::default(),
    };
    let successful = task::build_task_resume_context_with_selection(
        &f.store,
        request(32768),
        |input| async { unified_selection::execute(&f.index, input, &Default::default()).await },
        5000,
        true,
    )
    .await
    .unwrap();
    let unavailable = task::build_task_resume_context_with_selection(
        &f.store,
        request(4096),
        |_| async { Err(AppError::Message("synthetic optional failure".into())) },
        5000,
        true,
    )
    .await
    .unwrap();
    let timed_out = task::build_task_resume_context_with_selection(
        &f.store,
        request(4096),
        |_| std::future::pending::<Result<task::OptionalTaskSelection, AppError>>(),
        100,
        true,
    )
    .await
    .unwrap();
    for context in [&successful, &unavailable, &timed_out] {
        assert_eq!(context.owner_provenance.len(), 1);
        assert_eq!(
            context.owner_provenance[0].exact_text,
            "禁止部署；保留原始要求。"
        );
        assert!(
            context.owner_provenance[0]
                .attestation_basis
                .contains("not_authenticated_human")
        );
        assert_eq!(context.mandatory_requirements.len(), 2);
        assert!(
            context
                .mandatory_requirements
                .iter()
                .any(|r| r.requirement_id == "protected" && r.text == "禁止部署；保留原始要求。")
        );
        assert_eq!(context.authority, "knowledge_only_no_execution_authority");
    }
    assert!(unavailable.serialized_bytes <= 4096);
    assert!(timed_out.serialized_bytes <= 4096);
    assert!(timed_out.diagnostics.iter().any(|d| d.contains("deadline")));
}

#[tokio::test]
async fn stratified_windows_keep_episode_and_experience_visible_after_hundreds_of_facts() {
    let f = F::new().await;
    f.sources().await;
    for n in 0..305 {
        sqlx::query("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES(?,'world','project/a','irrelevant','has','hay','observed','active')").bind(format!("fact{n:03}")).execute(&f.pool).await.unwrap();
        f.event(&format!("noise{n:03}"), "unrelated observation")
            .await;
    }
    f.build().await;
    let document_calls = f.docs.load(Ordering::SeqCst);
    // Corrupt rows from another generation must remain outside the exact range seek,
    // not be scanned/hydrated and discarded after the LIMIT.
    sqlx::query("WITH RECURSIVE n(v) AS (VALUES(1) UNION ALL SELECT v+1 FROM n WHERE v<1000) INSERT INTO derived_semantic_vectors(namespace,generation,reference,content_digest,dimensions,vector,checksum) SELECT 'project/a','foreign-generation','claim:000foreign'||v,'bad',999,x'00','bad' FROM n").execute(&f.pool).await.unwrap();
    assert!(!f.index.status(&f.ns()).await.unwrap().stale);
    let result = unified_selection::execute(&f.index, f.input(8, 32768), &Default::default())
        .await
        .unwrap();
    assert_eq!(result.episodes.len(), 1);
    assert_eq!(result.experiences.len(), 1);
    let report = result.report.unwrap();
    assert_eq!(report["ranking"], "bounded_partial_index_scan");
    assert_eq!(report["examined"], 130);
    assert_eq!(report["unexamined_remaining"], true);
    for kind in ["claim", "event"] {
        assert_eq!(report["kind_coverage"][kind]["examined"], 64);
        assert_eq!(report["kind_coverage"][kind]["unexamined_remaining"], true);
    }
    for kind in ["experience_episode", "experience_candidate"] {
        assert_eq!(report["kind_coverage"][kind]["examined"], 1);
        assert_eq!(report["kind_coverage"][kind]["unexamined_remaining"], false);
    }
    assert_eq!(f.queries.load(Ordering::SeqCst), 1);
    assert_eq!(f.docs.load(Ordering::SeqCst), document_calls);
    // An internal kind window cannot emit a misleading public ID-pagination cursor.
    let prepared = f.index.prepare_query(&f.ns(), "target").await.unwrap();
    let page = f
        .index
        .search_prepared_kind(
            &f.ns(),
            &prepared,
            agent_llm_mm::adapters::sqlite::semantic_index::SemanticSourceKind::Claim,
            64,
        )
        .await
        .unwrap();
    assert!(page.has_more);
    assert!(page.next_cursor.is_none());
    assert_eq!(page.scanned, 64);
}
