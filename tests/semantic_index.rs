//! Entirely synthetic: no model, credentials, network or external inference.
#[cfg(windows)]
#[path = "support/windows_private.rs"]
mod windows_private;

use agent_llm_mm::{
    adapters::{
        local_embedding::LocalEmbeddingConfig,
        sqlite::{
            SqliteStore, inspect_database, open_current_database, reset_semantic_index,
            semantic_index::SemanticIndex,
        },
    },
    application::{export_memory::export_memory, recall_memory_enhanced::content_digest},
    domain::{
        ledger_export::ExportMemoryRequest,
        recall_applicability::DeclareRecallApplicabilityRequest,
        types::{MemoryScope, Namespace},
    },
    error::AppError,
    ports::{
        embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
        enhanced_memory_store::EnhancedMemoryStore,
    },
};
use async_trait::async_trait;
use sqlx::SqlitePool;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Fake {
    profile: EmbeddingProfile,
    calls: Arc<AtomicUsize>,
    fail: bool,
    mutate: Option<SqlitePool>,
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(AppError::Message("synthetic failure".into()));
        }
        if let Some(pool) = &self.mutate {
            sqlx::query(
                "UPDATE events SET summary='changed during inference' WHERE event_id='e000'",
            )
            .execute(pool)
            .await
            .unwrap();
        }
        Ok(texts
            .iter()
            .map(|text| {
                if purpose == EmbeddingPurpose::Query || text.contains("needle") {
                    vec![1.0, 0.0]
                } else {
                    vec![0.0, 1.0]
                }
            })
            .collect())
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    url: String,
    store: SqliteStore,
    pool: SqlitePool,
    config: LocalEmbeddingConfig,
    calls: Arc<AtomicUsize>,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(windows)]
        windows_private::private_directory(dir.path());
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&root.join("semantic.sqlite"));
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        let config = LocalEmbeddingConfig {
            enabled: true,
            local_only_declared: true,
            namespace_allowlist: vec!["project/a".into()],
            profile: EmbeddingProfile {
                model: "synthetic".into(),
                model_digest: "a".repeat(64),
                dimensions: 2,
                document_prefix: "d:".into(),
                query_prefix: "q:".into(),
            },
            ..Default::default()
        };
        Self {
            _dir: dir,
            root,
            url,
            store,
            pool,
            config,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn ns(&self) -> Namespace {
        Namespace::for_project("a")
    }
    fn index(&self) -> SemanticIndex {
        self.with_port(false, false)
    }
    fn with_port(&self, fail: bool, mutate: bool) -> SemanticIndex {
        SemanticIndex::new(
            self.store.clone(),
            self.config.clone(),
            Arc::new(Fake {
                profile: self.config.profile.clone(),
                calls: self.calls.clone(),
                fail,
                mutate: mutate.then(|| self.pool.clone()),
            }),
        )
        .unwrap()
    }
    async fn event(&self, id: &str, namespace: &str, text: &str) {
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES(?,'2026-01-01T00:00:00Z','world',?,'observation',?)").bind(id).bind(namespace).bind(text).execute(&self.pool).await.unwrap();
    }
    async fn built(&self) -> String {
        let index = self.index();
        let mut state = index.start(&self.ns()).await.unwrap();
        while !state.complete {
            state = index
                .resume(&self.ns(), &state.generation, 4)
                .await
                .unwrap();
        }
        state.generation
    }
}
#[tokio::test]
async fn rebuild_resumes_across_reopen_and_searches_beyond_original_64_window() {
    let f = Fixture::new().await;
    for n in 0..73 {
        f.event(
            &format!("e{n:03}"),
            "project/a",
            if n == 72 {
                "needle beyond old window"
            } else {
                "hay"
            },
        )
        .await;
    }
    f.event("foreign", "project/b", "needle forbidden").await;
    sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('unknown','2026-01-01T00:00:00Z','unknown','project/a','observation','needle unknown owner')").execute(&f.pool).await.unwrap();
    let index = f.index();
    assert!(index.status(&f.ns()).await.is_err());
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    let start = index.start(&f.ns()).await.unwrap();
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
    let first = index.resume(&f.ns(), &start.generation, 3).await.unwrap();
    assert_eq!(
        (first.examined, first.indexed, first.complete),
        (3, 3, false)
    );
    assert!(
        index
            .search_page(&f.ns(), &start.generation, "target", None, 10)
            .await
            .is_err()
    );
    assert!(inspect_database(&f.url).await.unwrap().is_current());
    let reopened = open_current_database(&f.url).await.unwrap();
    let resumed = SemanticIndex::new(
        reopened,
        f.config.clone(),
        Arc::new(Fake {
            profile: f.config.profile.clone(),
            calls: f.calls.clone(),
            fail: false,
            mutate: None,
        }),
    )
    .unwrap();
    let mut state = resumed.status(&f.ns()).await.unwrap();
    while !state.complete {
        state = resumed.resume(&f.ns(), &state.generation, 4).await.unwrap();
    }
    assert_eq!((state.examined, state.indexed, state.omitted), (73, 73, 0));
    let mut cursor = None;
    let mut hits = Vec::new();
    loop {
        let page = resumed
            .search_page(&f.ns(), &state.generation, "target", cursor, 9)
            .await
            .unwrap();
        assert!(page.scanned <= 9);
        hits.extend(page.hits);
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(hits.len(), 73);
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
    assert_eq!(hits[0].reference, "event:e072");
    assert!(hits.iter().all(|h| h.reference != "event:foreign"));
    assert!(resumed.start(&Namespace::for_project("b")).await.is_err());
}
#[tokio::test]
async fn failed_embedding_and_mutation_during_inference_do_not_advance_cursor() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    let start = f.index().start(&f.ns()).await.unwrap();
    assert!(
        f.with_port(true, false)
            .resume(&f.ns(), &start.generation, 4)
            .await
            .is_err()
    );
    let status = f.index().status(&f.ns()).await.unwrap();
    assert_eq!(status.examined, 0);
    assert!(!status.complete);
    assert!(
        f.with_port(false, true)
            .resume(&f.ns(), &start.generation, 4)
            .await
            .is_err()
    );
    let status = f.index().status(&f.ns()).await.unwrap();
    assert_eq!(status.indexed, 0);
    assert!(status.stale);
    let new = f.built().await;
    assert_ne!(new, start.generation);
    assert!(
        f.index()
            .resume(&f.ns(), &start.generation, 4)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn insert_update_delete_and_every_provenance_table_invalidate_generation() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    sqlx::raw_sql("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES('c','world','project/a','s','p','o','observed','active'); INSERT INTO evidence_links VALUES('c','e000'); INSERT INTO episode_events VALUES('episode:one','e000'); INSERT INTO reflections(reflection_id,recorded_at,summary) VALUES('r','2026-01-01T00:00:00Z','r'); INSERT INTO recall_applicability VALUES('project/other','event:x',1,'x','{}','{}');").execute(&f.pool).await.unwrap();
    for table in [
        "events",
        "claims",
        "evidence_links",
        "episode_events",
        "reflections",
        "recall_applicability",
    ] {
        let start = f.index().start(&f.ns()).await.unwrap();
        sqlx::query(&format!("UPDATE {table} SET rowid=rowid"))
            .execute(&f.pool)
            .await
            .unwrap();
        assert!(f.index().status(&f.ns()).await.unwrap().stale, "{table}");
        assert!(
            f.index()
                .resume(&f.ns(), &start.generation, 4)
                .await
                .is_err()
        );
    }
    // DELETE mutations in dependency order, and inserted rows, are covered too.
    for table in [
        "evidence_links",
        "episode_events",
        "claims",
        "reflections",
        "recall_applicability",
        "events",
    ] {
        f.index().start(&f.ns()).await.unwrap();
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&f.pool)
            .await
            .unwrap();
        assert!(
            f.index().status(&f.ns()).await.unwrap().stale,
            "delete {table}"
        );
    }
    let generation = f.built().await;
    f.event(
        "new",
        "project/b",
        "outside scope still conservatively invalidates",
    )
    .await;
    assert!(
        f.index()
            .search_page(&f.ns(), &generation, "target", None, 64)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn profile_model_digest_dimension_and_prefix_changes_invalidate_old_run() {
    let mut f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    f.built().await;
    let original = f.config.clone();
    for change in 0..5 {
        f.config = original.clone();
        match change {
            0 => f.config.profile.model = "other".into(),
            1 => f.config.profile.model_digest = "b".repeat(64),
            2 => f.config.profile.dimensions = 3,
            3 => f.config.profile.document_prefix = "new".into(),
            _ => f.config.profile.query_prefix = "new".into(),
        }
        assert!(f.index().status(&f.ns()).await.unwrap().stale);
    }
    f.config.enabled = false;
    assert!(
        SemanticIndex::new(
            f.store.clone(),
            f.config.clone(),
            Arc::new(Fake {
                profile: f.config.profile.clone(),
                calls: f.calls.clone(),
                fail: false,
                mutate: None
            })
        )
        .is_err()
    );
}
#[tokio::test]
async fn oversized_and_expired_sources_have_honest_coverage_and_are_not_embedded() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle expired").await;
    f.event("huge", "project/a", &"x".repeat(40000)).await;
    let record = f
        .store
        .hydrate_enhanced_record(MemoryScope::for_namespace(f.ns()), "event:e000")
        .await
        .unwrap()
        .unwrap();
    f.store
        .declare_recall_applicability(DeclareRecallApplicabilityRequest {
            request_id: "expired".into(),
            expected_revision: 0,
            declaration: serde_json::from_value(serde_json::json!({
                "namespace":"project/a","reference":"event:e000","content_digest":content_digest(&record).unwrap(),
                "valid_from":null,"valid_until":"2000-01-01T00:00:00Z","conditions":[],"environment":[]
            })).unwrap(),
        })
        .await
        .unwrap();
    f.built().await;
    let state = f.index().status(&f.ns()).await.unwrap();
    assert_eq!((state.examined, state.indexed, state.omitted), (2, 0, 2));
    assert_eq!(state.coverage, "completed_with_omissions");
    assert_eq!(f.calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn corrupt_vectors_missing_rows_and_cursor_mismatch_fail_closed() {
    let f = Fixture::new().await;
    for n in 0..3 {
        f.event(&format!("e{n:03}"), "project/a", "needle").await;
    }
    let generation = f.built().await;
    let page = f
        .index()
        .search_page(&f.ns(), &generation, "target", None, 1)
        .await
        .unwrap();
    assert!(
        f.index()
            .search_page(&f.ns(), &generation, "different", page.next_cursor, 1)
            .await
            .is_err()
    );
    sqlx::query("UPDATE derived_semantic_vectors SET vector=zeroblob(10000000) WHERE reference='event:e000'").execute(&f.pool).await.unwrap();
    assert!(
        f.index()
            .search_page(&f.ns(), &generation, "target", None, 2)
            .await
            .is_err()
    );
    let generation = f.built().await;
    sqlx::query("UPDATE derived_semantic_vectors SET checksum=? WHERE reference='event:e000'")
        .bind("a".repeat(64))
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.index()
            .search_page(&f.ns(), &generation, "target", None, 2)
            .await
            .is_err()
    );
    f.built().await;
    sqlx::query("DELETE FROM derived_semantic_vectors WHERE reference='event:e000'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.index().status(&f.ns()).await.is_err());
}
#[tokio::test]
async fn canonical_extension_inspection_teardown_and_partial_recovery_are_explicit() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    let generation = f.built().await;
    sqlx::query("DROP TRIGGER derived_semantic_events_UPDATE")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(!inspect_database(&f.url).await.unwrap().is_current());
    assert!(f.index().status(&f.ns()).await.is_err());
    assert!(f.index().start(&f.ns()).await.is_err());
    reset_semantic_index(&f.url).await.unwrap();
    assert!(inspect_database(&f.url).await.unwrap().is_current());
    let rebuilt = f.built().await;
    assert_ne!(generation, rebuilt);
    sqlx::raw_sql("DROP TRIGGER derived_semantic_events_UPDATE; CREATE TRIGGER derived_semantic_events_UPDATE AFTER UPDATE ON events BEGIN SELECT 1; END;").execute(&f.pool).await.unwrap();
    assert!(!inspect_database(&f.url).await.unwrap().is_current());
    assert!(f.index().status(&f.ns()).await.is_err());
    reset_semantic_index(&f.url).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM events")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn unknown_extension_objects_block_install_and_reset_does_not_delete_them() {
    let f = Fixture::new().await;
    sqlx::query("CREATE TABLE derived_semantic_foreign(secret TEXT)")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.index().start(&f.ns()).await.is_err());
    assert!(reset_semantic_index(&f.url).await.is_err());
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sqlite_master WHERE name='derived_semantic_foreign'",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}
#[tokio::test]
async fn full_snapshot_restore_preserves_resumption_and_scoped_export_omits_vectors() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    f.event("e001", "project/a", "hay").await;
    let start = f.index().start(&f.ns()).await.unwrap();
    f.index()
        .resume(&f.ns(), &start.generation, 1)
        .await
        .unwrap();
    let backup = f.root.join("backup.sqlite");
    agent_llm_mm::support::database_backup::transfer_database(
        "backup",
        &f.root.join("semantic.sqlite"),
        &backup,
    )
    .await
    .unwrap();
    let restored = f.root.join("restored.sqlite");
    agent_llm_mm::support::database_backup::transfer_database("restore", &backup, &restored)
        .await
        .unwrap();
    let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&restored);
    assert!(inspect_database(&url).await.unwrap().is_current());
    let store = open_current_database(&url).await.unwrap();
    let restored_index = SemanticIndex::new(
        store.clone(),
        f.config.clone(),
        Arc::new(Fake {
            profile: f.config.profile.clone(),
            calls: f.calls.clone(),
            fail: false,
            mutate: None,
        }),
    )
    .unwrap();
    let state = restored_index
        .resume(&f.ns(), &start.generation, 4)
        .await
        .unwrap();
    assert_eq!(state.indexed, 2);
    assert!(state.complete);
    let export = export_memory(&store, ExportMemoryRequest::new("project/a"))
        .await
        .unwrap();
    let json = serde_json::to_string(&export).unwrap();
    assert!(!json.contains("derived_semantic"));
    assert!(!json.contains("model_digest"));
    assert!(json.contains("needle"));
}

#[tokio::test]
async fn source_and_vector_pagination_are_index_seeks_not_partition_sorts() {
    let f = Fixture::new().await;
    f.built().await;
    for sql in [
        "EXPLAIN QUERY PLAN SELECT rowid FROM events INDEXED BY derived_semantic_events_scope WHERE owner='world' AND namespace='project/a' AND rowid>0 ORDER BY rowid LIMIT 5",
        "EXPLAIN QUERY PLAN SELECT rowid FROM claims INDEXED BY derived_semantic_claims_scope WHERE owner='world' AND namespace='project/a' AND status='active' AND rowid>0 ORDER BY rowid LIMIT 5",
        "EXPLAIN QUERY PLAN SELECT id FROM derived_semantic_vectors INDEXED BY derived_semantic_vector_page WHERE namespace='project/a' AND generation='g' AND id>0 ORDER BY id LIMIT 65",
    ] {
        use sqlx::Row;
        let rows = sqlx::query(sql).fetch_all(&f.pool).await.unwrap();
        let details = rows
            .iter()
            .map(|r| r.get::<String, _>("detail"))
            .collect::<Vec<_>>()
            .join(";");
        assert!(!details.contains("TEMP B-TREE"), "{details}");
        assert!(details.contains("SEARCH"), "{details}");
        assert!(
            details.contains("rowid>?") || details.contains("id>?"),
            "{details}"
        );
    }
}
#[tokio::test]
async fn false_completed_run_and_moved_vector_are_rejected() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    f.index().start(&f.ns()).await.unwrap();
    sqlx::query("UPDATE derived_semantic_runs SET complete=1")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.index().status(&f.ns()).await.is_err());
    f.built().await;
    sqlx::query("UPDATE derived_semantic_vectors SET namespace='project/b'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.index().status(&f.ns()).await.is_err());
}
#[tokio::test]
async fn optional_index_keeps_workspace_adoption_supported_and_handles_wrong_object_type() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    f.built().await;
    let root = f.root.join("project");
    std::fs::create_dir(&root).unwrap();
    let report = agent_llm_mm::support::workspace_registry::preview_adoption(
        &f.root.join("registry.json"),
        &root,
        "project/a",
        &f.root.join("semantic.sqlite"),
    )
    .await
    .unwrap();
    assert!(
        report.inventory.supported,
        "{:?}",
        report.inventory.blocked_reasons
    );
    reset_semantic_index(&f.url).await.unwrap();
    sqlx::query("CREATE VIEW derived_semantic_vectors AS SELECT 1 AS value")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(!inspect_database(&f.url).await.unwrap().is_current());
    reset_semantic_index(&f.url).await.unwrap();
    assert!(inspect_database(&f.url).await.unwrap().is_current());
}
#[tokio::test]
async fn cli_start_status_and_reset_are_real_commands_without_inference() {
    let f = Fixture::new().await;
    let config = f.root.join("config.toml");
    std::fs::write(
        &config,
        format!(
            "database_url = {:?}\n[local_embedding]\n{}",
            f.url,
            toml::to_string(&f.config)
                .unwrap()
                .replace("[profile]", "[local_embedding.profile]")
        ),
    )
    .unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_agent_llm_mm"))
            .args(args)
            .env("AGENT_LLM_MM_CONFIG", &config)
            .env("AGENT_LLM_MM_DATABASE_URL", &f.url)
            .output()
            .unwrap()
    };
    for command in ["index-start", "index-status"] {
        let out = run(&[
            "retrieval",
            command,
            "--json",
            r#"{"namespace":"project/a"}"#,
        ]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["indexed"], 0);
    }
    // Tool arguments cannot inject a new permission or remote provider.
    let denied = run(&[
        "retrieval",
        "index-start",
        "--json",
        r#"{"namespace":"project/b"}"#,
    ]);
    assert!(!denied.status.success());
    let unknown = run(&[
        "retrieval",
        "index-start",
        "--json",
        r#"{"namespace":"project/a","enabled":true}"#,
    ]);
    assert!(!unknown.status.success());
    let out = run(&["retrieval", "index-reset"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(f.index().status(&f.ns()).await.is_err());
}

#[tokio::test]
async fn keyset_crosses_claim_event_boundary_and_includes_negative_rowids() {
    let f = Fixture::new().await;
    for (id, rid) in [("low", i64::MIN), ("zero", 0), ("one", 1)] {
        sqlx::query("INSERT INTO claims(rowid,claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES(?,?,'world','project/a','s','p','o','observed','active')")
            .bind(rid).bind(id).execute(&f.pool).await.unwrap();
        sqlx::query("INSERT INTO events(rowid,event_id,recorded_at,owner,namespace,kind,summary) VALUES(?,?,'2026-01-01T00:00:00Z','world','project/a','observation','needle')")
            .bind(rid).bind(id).execute(&f.pool).await.unwrap();
    }
    let generation = f.built().await;
    let state = f.index().status(&f.ns()).await.unwrap();
    assert_eq!((state.examined, state.indexed, state.omitted), (6, 6, 0));
    let page = f
        .index()
        .search_page(&f.ns(), &generation, "target", None, 64)
        .await
        .unwrap();
    assert_eq!(page.hits.len(), 6);
}
struct Hanging {
    profile: EmbeddingProfile,
}
#[async_trait]
impl EmbeddingPort for Hanging {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    async fn embed(
        &self,
        _: &Namespace,
        _: EmbeddingPurpose,
        _: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        std::future::pending().await
    }
}
#[tokio::test]
async fn cancellation_leaves_batch_cursor_and_vectors_uncommitted() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    let generation = f.index().start(&f.ns()).await.unwrap().generation;
    let index = SemanticIndex::new(
        f.store.clone(),
        f.config.clone(),
        Arc::new(Hanging {
            profile: f.config.profile.clone(),
        }),
    )
    .unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(30),
            index.resume(&f.ns(), &generation, 4)
        )
        .await
        .is_err()
    );
    let state = f.index().status(&f.ns()).await.unwrap();
    assert_eq!((state.examined, state.indexed), (0, 0));
    assert!(!state.complete);
    assert!(
        f.index()
            .resume(&f.ns(), &generation, 4)
            .await
            .unwrap()
            .complete
    );
}

struct BarrierPort {
    profile: EmbeddingProfile,
    barrier: Arc<tokio::sync::Barrier>,
}
#[async_trait]
impl EmbeddingPort for BarrierPort {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    async fn embed(
        &self,
        _: &Namespace,
        _: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        self.barrier.wait().await;
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
}
#[tokio::test]
async fn competing_resumes_commit_a_source_page_only_once() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    let generation = f.index().start(&f.ns()).await.unwrap().generation;
    let index = SemanticIndex::new(
        f.store.clone(),
        f.config.clone(),
        Arc::new(BarrierPort {
            profile: f.config.profile.clone(),
            barrier: Arc::new(tokio::sync::Barrier::new(2)),
        }),
    )
    .unwrap();
    let ns = f.ns();
    let (left, right) = tokio::join!(
        index.resume(&ns, &generation, 4),
        index.resume(&ns, &generation, 4)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let state = f.index().status(&ns).await.unwrap();
    assert_eq!((state.examined, state.indexed, state.omitted), (1, 1, 0));
    assert!(state.complete);
}

#[tokio::test]
async fn persisted_run_control_corruption_cannot_skip_sources_or_forge_coverage() {
    let f = Fixture::new().await;
    for n in 0..5 {
        f.event(&format!("e{n:03}"), "project/a", "needle").await;
    }
    for mutation in [
        "source_rowid=9223372036854775807",
        "kind=1,source_rowid=9223372036854775807,complete=1",
        "examined=examined+1,omitted=omitted+1",
        "control_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
    ] {
        let generation = f.index().start(&f.ns()).await.unwrap().generation;
        f.index().resume(&f.ns(), &generation, 2).await.unwrap();
        sqlx::query(&format!("UPDATE derived_semantic_runs SET {mutation}"))
            .execute(&f.pool)
            .await
            .unwrap();
        assert!(f.index().status(&f.ns()).await.is_err(), "{mutation}");
        assert!(
            f.index().resume(&f.ns(), &generation, 4).await.is_err(),
            "{mutation}"
        );
    }
}
#[tokio::test]
async fn teardown_refuses_unknown_objects_that_drop_table_would_implicitly_remove() {
    let f = Fixture::new().await;
    f.built().await;
    for ddl in [
        "CREATE TRIGGER custom_semantic_hook AFTER INSERT ON derived_semantic_vectors BEGIN SELECT 1; END",
        "CREATE INDEX custom_semantic_lookup ON derived_semantic_vectors(reference)",
    ] {
        sqlx::query(ddl).execute(&f.pool).await.unwrap();
        assert!(!inspect_database(&f.url).await.unwrap().is_current());
        assert!(reset_semantic_index(&f.url).await.is_err());
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE name='derived_semantic_vectors'",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(count, 1, "teardown rolled back");
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name IN ('custom_semantic_hook','custom_semantic_lookup')").fetch_one(&f.pool).await.unwrap();
    assert_eq!(count, 2, "unknown dependent objects preserved");
}

#[tokio::test]
async fn index_hits_expose_declarations_without_claiming_environment_satisfaction() {
    let f = Fixture::new().await;
    f.event("e000", "project/a", "needle").await;
    f.event("e001", "project/a", "no declaration").await;
    f.event("e002", "project/a", "stale declaration").await;
    let record = f
        .store
        .hydrate_enhanced_record(MemoryScope::for_namespace(f.ns()), "event:e000")
        .await
        .unwrap()
        .unwrap();
    f.store.declare_recall_applicability(DeclareRecallApplicabilityRequest {
        request_id:"conditions".into(),expected_revision:0,
        declaration:serde_json::from_value(serde_json::json!({
            "namespace":"project/a","reference":"event:e000","content_digest":content_digest(&record).unwrap(),
            "valid_from":null,"valid_until":null,"conditions":["operator must verify"],"environment":["unverified condition"]
        })).unwrap(),
    }).await.unwrap();
    f.store.declare_recall_applicability(DeclareRecallApplicabilityRequest {
        request_id:"stale".into(),expected_revision:0,
        declaration:serde_json::from_value(serde_json::json!({
            "namespace":"project/a","reference":"event:e002","content_digest":"a".repeat(64),
            "valid_from":null,"valid_until":"2000-01-01T00:00:00Z","conditions":["outdated"],"environment":[]
        })).unwrap(),
    }).await.unwrap();
    let generation = f.built().await;
    let page = f
        .index()
        .search_page(&f.ns(), &generation, "target", None, 64)
        .await
        .unwrap();
    assert_eq!(
        page.applicability_policy,
        "current_digest_time_declarations_only_conditions_and_environment_not_evaluated"
    );
    let declared = page
        .hits
        .iter()
        .find(|hit| hit.reference == "event:e000")
        .unwrap();
    assert_eq!(
        declared.applicability,
        "declared_conditions_and_environment_not_evaluated"
    );
    assert_eq!(
        declared.declared_applicability.as_ref().unwrap().conditions,
        ["operator must verify"]
    );
    let unknown = page
        .hits
        .iter()
        .find(|hit| hit.reference == "event:e001")
        .unwrap();
    assert_eq!(
        unknown.applicability,
        "unknown_no_current_applicability_declaration"
    );
    assert!(unknown.declared_applicability.is_none());
    let stale = page
        .hits
        .iter()
        .find(|hit| hit.reference == "event:e002")
        .unwrap();
    assert_eq!(
        stale.applicability,
        "declaration_for_other_content_digest_unknown"
    );
    assert_ne!(
        stale.content_digest,
        stale
            .declared_applicability
            .as_ref()
            .unwrap()
            .content_digest
    );
}
