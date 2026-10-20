use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, inspect_database},
    application::{
        recall_memory::{self, RecallMemoryInput},
        recall_memory_enhanced::{self, EnhancedRecallConfig, EnhancedRecallInput},
    },
    domain::{
        recall_applicability::{DeclareRecallApplicabilityRequest, RecallApplicability},
        types::Namespace,
    },
    error::AppError,
    ports::embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
};
use async_trait::async_trait;
use sqlx::SqlitePool;
struct Fixture {
    _dir: tempfile::TempDir,
    url: String,
    store: SqliteStore,
    pool: SqlitePool,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("enhanced.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        Self {
            _dir: dir,
            url,
            store,
            pool,
        }
    }
    async fn event(&self, id: &str, ns: &str, text: &str) {
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES(?,'2026-01-01T00:00:00Z','world',?,'observation',?)").bind(id).bind(ns).bind(text).execute(&self.pool).await.unwrap();
    }
    fn input(&self, query: &str) -> EnhancedRecallInput {
        EnhancedRecallInput {
            namespace: Namespace::for_project("a"),
            query: query.into(),
            limit: 20,
            max_bytes: 16384,
            reported_environment: Default::default(),
        }
    }
}
#[tokio::test]
async fn chinese_alias_recall_keeps_literal_unchanged_and_namespace_isolated() {
    let f = Fixture::new().await;
    f.event("a", "project/a", "我喜欢喝咖啡").await;
    f.event("b", "project/b", "我喜欢喝咖啡").await;
    let legacy = recall_memory::execute(
        &f.store,
        RecallMemoryInput {
            namespace: Namespace::for_project("a"),
            query: "咖啡推荐".into(),
            limit: 20,
        },
    )
    .await
    .unwrap();
    assert!(legacy.records.is_empty());
    let result = recall_memory_enhanced::execute(
        &f.store,
        f.input("咖啡推荐"),
        &EnhancedRecallConfig::default(),
    )
    .await
    .unwrap();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].reference, "event:a");
    assert!(
        result.records[0]
            .routes
            .keys()
            .any(|r| r.starts_with("tokens"))
    );
    assert_eq!(
        serde_json::to_vec(&result).unwrap().len(),
        result.serialized_bytes
    );
}
struct Fake {
    profile: EmbeddingProfile,
}
#[async_trait]
impl EmbeddingPort for Fake {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    async fn embed(
        &self,
        namespace: &Namespace,
        purpose: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        assert_eq!(namespace.as_str(), "project/a");
        assert!(!texts.iter().any(|s| s.contains("foreign")));
        Ok(texts
            .iter()
            .map(|s| match purpose {
                EmbeddingPurpose::Query => vec![1.0, 0.0],
                EmbeddingPurpose::Document => {
                    if s.contains("espresso") {
                        vec![1.0, 0.0]
                    } else {
                        vec![0.0, 1.0]
                    }
                }
            })
            .collect())
    }
}
#[tokio::test]
async fn semantic_only_scope_first_window_and_invalid_profile_fallback() {
    let f = Fixture::new().await;
    f.event("a", "project/a", "espresso").await;
    f.event("b", "project/b", "foreign espresso").await;
    let fake = Fake {
        profile: EmbeddingProfile {
            dimensions: 2,
            ..Default::default()
        },
    };
    let r = recall_memory_enhanced::execute_with_embedding(
        &f.store,
        f.input("coffee"),
        &Default::default(),
        Some(&fake),
    )
    .await
    .unwrap();
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.records[0].reference, "event:a");
    assert!(r.records[0].routes.contains_key("local_embedding"));
    let bad = Fake {
        profile: EmbeddingProfile {
            dimensions: 3,
            ..Default::default()
        },
    };
    let r = recall_memory_enhanced::execute_with_embedding(
        &f.store,
        f.input("espresso"),
        &Default::default(),
        Some(&bad),
    )
    .await
    .unwrap();
    assert_eq!(r.records.len(), 1);
    assert!(!r.warnings.is_empty());
    assert!(!r.records[0].routes.contains_key("local_embedding"));
}
#[tokio::test]
async fn authority_expiry_replay_corruption_and_missing_table_are_explicit() {
    let f = Fixture::new().await;
    f.event("a", "project/a", "coffee").await;
    let r = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    let request = DeclareRecallApplicabilityRequest {
        request_id: "declare-1".into(),
        expected_revision: 0,
        declaration: RecallApplicability {
            namespace: "project/a".into(),
            reference: "event:a".into(),
            content_digest: r.records[0].content_digest.clone(),
            valid_from: None,
            valid_until: Some("2020-01-01T00:00:00Z".into()),
            conditions: vec![],
            environment: vec![],
            typed_environment: Default::default(),
        },
    };
    let a = f
        .store
        .declare_recall_applicability(request.clone())
        .await
        .unwrap();
    let b = f
        .store
        .declare_recall_applicability(request.clone())
        .await
        .unwrap();
    assert_eq!(a.revision, b.revision);
    let r = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    assert!(r.records.is_empty());
    assert_eq!(r.applicability_omissions, 1);
    sqlx::query("UPDATE recall_applicability SET payload_json=zeroblob(1000000)")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.store
            .declare_recall_applicability(request)
            .await
            .unwrap_err()
            .to_string()
            .contains("corruption")
    );
    assert!(
        recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
            .await
            .unwrap_err()
            .to_string()
            .contains("corruption")
    );
    sqlx::query("DROP TABLE recall_applicability")
        .execute(&f.pool)
        .await
        .unwrap();
    let report = inspect_database(&f.url).await.unwrap();
    assert!(!report.is_current());
    assert!(
        recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn derived_fts_corruption_does_not_hide_exact_route() {
    let f = Fixture::new().await;
    f.event("a", "project/a", "coffee").await;
    sqlx::query("DROP TABLE text_recall_fts")
        .execute(&f.pool)
        .await
        .unwrap();
    let r = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    assert_eq!(r.records.len(), 1);
}
#[tokio::test]
async fn oversized_source_is_counted_and_later_record_backfills() {
    let f = Fixture::new().await;
    f.event("a", "project/a", &format!("coffee {}", "x".repeat(40000)))
        .await;
    f.event("b", "project/a", "coffee small").await;
    let r = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.records[0].reference, "event:b");
    assert!(r.unavailable_after_retrieval > 0);
}
#[tokio::test]
async fn whole_record_output_packing_skips_large_record_and_backfills() {
    let f = Fixture::new().await;
    f.event("a", "project/a", &format!("coffee {}", "x".repeat(7000)))
        .await;
    f.event("b", "project/a", "coffee small").await;
    let mut input = f.input("coffee");
    input.limit = 1;
    input.max_bytes = 2200;
    let r = recall_memory_enhanced::execute(&f.store, input, &Default::default())
        .await
        .unwrap();
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.records[0].reference, "event:b");
    assert_eq!(r.byte_budget_omissions, 1);
    assert_eq!(serde_json::to_vec(&r).unwrap().len(), r.serialized_bytes);
    assert!(r.serialized_bytes <= 2200);
}
#[tokio::test]
async fn safe_hydration_preserves_legacy_provenance_digest() {
    use agent_llm_mm::{
        application::search_memory::SearchMemoryRecord,
        domain::{claim::ClaimReference, event::EventReference, types::MemoryScope},
        ports::{
            ClaimRecordQuery, ClaimStatus, EventRecordQuery, MemoryReadStore,
            enhanced_memory_store::EnhancedMemoryStore,
        },
    };
    let f = Fixture::new().await;
    f.event("a", "project/a", "coffee").await;
    sqlx::raw_sql("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES('c','world','project/a','user','likes','coffee','observed','active'); INSERT INTO evidence_links(claim_id,event_id) VALUES('c','a'); INSERT INTO episode_events(episode_reference,event_id) VALUES('episode:one','a');").execute(&f.pool).await.unwrap();
    let scope = MemoryScope::for_namespace(Namespace::for_project("a"));
    let old: SearchMemoryRecord = f
        .store
        .query_claim_records(ClaimRecordQuery {
            scope: scope.clone(),
            claim_reference: Some(ClaimReference::parse("claim:c").unwrap()),
            status: Some(ClaimStatus::Active),
            mode: None,
            limit: 1,
        })
        .await
        .unwrap()
        .remove(0)
        .into();
    let new = f
        .store
        .hydrate_enhanced_record(scope.clone(), "claim:c")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new, old);
    let old: SearchMemoryRecord = f
        .store
        .query_event_records(EventRecordQuery {
            scope: scope.clone(),
            event_reference: Some(EventReference::parse("event:a").unwrap()),
            kind: None,
            recorded_after: None,
            recorded_before: None,
            limit: 1,
        })
        .await
        .unwrap()
        .remove(0)
        .into();
    let new = f
        .store
        .hydrate_enhanced_record(scope, "event:a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new, old);
}
#[tokio::test]
async fn changed_after_selection_cannot_inherit_stale_lexical_evidence() {
    use agent_llm_mm::{
        application::search_memory::SearchMemoryRecord,
        domain::types::MemoryScope,
        ports::{
            enhanced_memory_store::{EnhancedMemoryStore, EnhancedRecordPage},
            text_memory_store::{TextMemoryPage, TextMemoryQuery, TextMemoryStore},
        },
    };
    struct Changing {
        store: SqliteStore,
        pool: SqlitePool,
    }
    #[async_trait]
    impl TextMemoryStore for Changing {
        async fn recall_text(&self, q: TextMemoryQuery) -> Result<TextMemoryPage, AppError> {
            self.recall_text_exact(q).await
        }
        async fn recall_text_exact(&self, q: TextMemoryQuery) -> Result<TextMemoryPage, AppError> {
            let page = self.store.recall_text_exact(q).await?;
            sqlx::query("UPDATE events SET summary='unrelated' WHERE event_id='a'")
                .execute(&self.pool)
                .await
                .unwrap();
            Ok(page)
        }
    }
    #[async_trait]
    impl EnhancedMemoryStore for Changing {
        async fn scan_enhanced_records(
            &self,
            s: MemoryScope,
            l: usize,
        ) -> Result<EnhancedRecordPage, AppError> {
            self.store.scan_enhanced_records(s, l).await
        }
        async fn hydrate_enhanced_record(
            &self,
            s: MemoryScope,
            r: &str,
        ) -> Result<Option<SearchMemoryRecord>, AppError> {
            self.store.hydrate_enhanced_record(s, r).await
        }
    }
    let f = Fixture::new().await;
    f.event("a", "project/a", "coffee").await;
    let input = f.input("coffee");
    let changing = Changing {
        store: f.store.clone(),
        pool: f.pool.clone(),
    };
    let r = recall_memory_enhanced::execute(&changing, input, &Default::default())
        .await
        .unwrap();
    assert!(r.records.is_empty());
    assert!(r.unavailable_after_retrieval > 0);
}
#[tokio::test]
async fn schema9_migration_adds_no_invented_validity_and_preserves_sources() {
    use agent_llm_mm::adapters::sqlite::{CURRENT_DATABASE_SCHEMA_VERSION, migrate_database};
    let f = Fixture::new().await;
    f.event("a", "project/a", "原始来源 unchanged").await;
    sqlx::raw_sql("DROP TABLE task_owner_provenance; DROP TABLE task_owner_events; DROP TABLE recall_applicability; DELETE FROM schema_migrations WHERE version>=10; PRAGMA user_version=9;").execute(&f.pool).await.unwrap();
    let before: String = sqlx::query_scalar("SELECT summary FROM events WHERE event_id='a'")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    f.pool.close().await;
    let result = migrate_database(&f.url).await.unwrap();
    assert_eq!(result.schema_version, Some(CURRENT_DATABASE_SCHEMA_VERSION));
    let pool = SqlitePool::connect(&f.url).await.unwrap();
    let after: String = sqlx::query_scalar("SELECT summary FROM events WHERE event_id='a'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recall_applicability")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn scoped_export_blocks_only_relevant_new_authority() {
    use agent_llm_mm::{
        domain::ledger_export::ExportMemoryRequest, ports::ledger_export_store::LedgerExportStore,
    };
    let f = Fixture::new().await;
    f.event("a", "project/a", "coffee").await;
    f.event("b", "project/b", "tea").await;
    let r = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    f.store
        .declare_recall_applicability(DeclareRecallApplicabilityRequest {
            request_id: "metadata".into(),
            expected_revision: 0,
            declaration: RecallApplicability {
                namespace: "project/a".into(),
                reference: "event:a".into(),
                content_digest: r.records[0].content_digest.clone(),
                valid_from: None,
                valid_until: None,
                conditions: vec!["only during tests".into()],
                environment: vec![],
                typed_environment: Default::default(),
            },
        })
        .await
        .unwrap();
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
}

#[tokio::test]
async fn typed_environment_filters_only_known_mismatch_and_preserves_unknown() {
    use agent_llm_mm::domain::recall_applicability::{EnvironmentMatch, EnvironmentValue};
    let f = Fixture::new().await;
    f.event("typed", "project/a", "coffee").await;
    let before = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    let request = DeclareRecallApplicabilityRequest {
        request_id: "typed-declaration".into(),
        expected_revision: 0,
        declaration: RecallApplicability {
            namespace: "project/a".into(),
            reference: "event:typed".into(),
            content_digest: before.records[0].content_digest.clone(),
            valid_from: None,
            valid_until: None,
            conditions: vec!["human review required".into()],
            environment: vec![],
            typed_environment: [
                ("os".into(), EnvironmentValue::Text("linux".into())),
                ("offline".into(), EnvironmentValue::Boolean(true)),
            ]
            .into(),
        },
    };
    f.store
        .declare_recall_applicability(request.clone())
        .await
        .unwrap();
    // Exact replay remains idempotent with the new optional declaration.
    assert_eq!(
        f.store
            .declare_recall_applicability(request)
            .await
            .unwrap()
            .revision,
        1
    );
    let unknown = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    assert_eq!(
        unknown.records[0].environment_match,
        EnvironmentMatch::Unknown
    );
    let mut input = f.input("coffee");
    input.reported_environment = [
        ("os".into(), EnvironmentValue::Text("linux".into())),
        ("offline".into(), EnvironmentValue::Boolean(true)),
    ]
    .into();
    let matched = recall_memory_enhanced::execute(&f.store, input.clone(), &Default::default())
        .await
        .unwrap();
    assert_eq!(
        matched.records[0].environment_match,
        EnvironmentMatch::MatchesReportedValues
    );
    // A typed equality is not proof of natural-language applicability or authorization.
    assert_eq!(
        matched.records[0].applicability,
        "declared_conditions_not_evaluated"
    );
    input
        .reported_environment
        .insert("os".into(), EnvironmentValue::Text("windows".into()));
    let mismatch = recall_memory_enhanced::execute(&f.store, input, &Default::default())
        .await
        .unwrap();
    assert!(mismatch.records.is_empty());
    assert_eq!(mismatch.applicability_omissions, 1);
}

struct ChangeDeclarationDuringEmbedding<'a> {
    store: &'a SqliteStore,
    request: DeclareRecallApplicabilityRequest,
    profile: EmbeddingProfile,
    changed: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl EmbeddingPort for ChangeDeclarationDuringEmbedding<'_> {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    async fn embed(
        &self,
        _: &Namespace,
        _: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        if !self.changed.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.store
                .declare_recall_applicability(self.request.clone())
                .await?;
        }
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
}
#[tokio::test]
async fn typed_environment_rechecks_updated_declarations_after_embedding() {
    use agent_llm_mm::domain::recall_applicability::EnvironmentValue;
    for stale_digest in [false, true] {
        let f = Fixture::new().await;
        f.event("race", "project/a", "coffee").await;
        let before =
            recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
                .await
                .unwrap();
        let mut request = DeclareRecallApplicabilityRequest {
            request_id: "race-v1".into(),
            expected_revision: 0,
            declaration: RecallApplicability {
                namespace: "project/a".into(),
                reference: "event:race".into(),
                content_digest: before.records[0].content_digest.clone(),
                valid_from: None,
                valid_until: None,
                conditions: vec![],
                environment: vec![],
                typed_environment: [("os".into(), EnvironmentValue::Text("linux".into()))].into(),
            },
        };
        f.store
            .declare_recall_applicability(request.clone())
            .await
            .unwrap();
        request.request_id = "race-v2".into();
        request.expected_revision = 1;
        request
            .declaration
            .typed_environment
            .insert("os".into(), EnvironmentValue::Text("windows".into()));
        if stale_digest {
            request.declaration.content_digest = "0".repeat(64);
        }
        let port = ChangeDeclarationDuringEmbedding {
            store: &f.store,
            request,
            changed: false.into(),
            profile: EmbeddingProfile {
                model: "synthetic".into(),
                model_digest: "a".repeat(64),
                dimensions: 2,
                document_prefix: String::new(),
                query_prefix: String::new(),
            },
        };
        let mut input = f.input("coffee");
        input.reported_environment = [("os".into(), EnvironmentValue::Text("linux".into()))].into();
        let result = recall_memory_enhanced::execute_with_embedding(
            &f.store,
            input,
            &Default::default(),
            Some(&port),
        )
        .await
        .unwrap();
        assert!(port.changed.load(std::sync::atomic::Ordering::SeqCst));
        if stale_digest {
            assert_eq!(result.records.len(), 1);
            assert_eq!(
                result.records[0].environment_match,
                agent_llm_mm::domain::recall_applicability::EnvironmentMatch::NotDeclared
            );
            assert_eq!(
                result.records[0].applicability,
                "declaration_for_other_content_digest_unknown"
            );
            assert_eq!(result.applicability_omissions, 0);
        } else {
            assert!(result.records.is_empty());
            assert_eq!(result.applicability_omissions, 1);
        }
        assert_eq!(result.unavailable_after_retrieval, 0);
    }
}

#[tokio::test]
async fn typed_mismatch_never_reaches_document_embedding() {
    use agent_llm_mm::domain::recall_applicability::EnvironmentValue;
    let f = Fixture::new().await;
    f.event("blocked", "project/a", "foreign coffee").await;
    f.event("allowed", "project/a", "coffee").await;
    let before = recall_memory_enhanced::execute(&f.store, f.input("coffee"), &Default::default())
        .await
        .unwrap();
    let digest = before
        .records
        .iter()
        .find(|h| h.reference == "event:blocked")
        .unwrap()
        .content_digest
        .clone();
    f.store
        .declare_recall_applicability(DeclareRecallApplicabilityRequest {
            request_id: "blocked-env".into(),
            expected_revision: 0,
            declaration: RecallApplicability {
                namespace: "project/a".into(),
                reference: "event:blocked".into(),
                content_digest: digest,
                valid_from: None,
                valid_until: None,
                conditions: vec![],
                environment: vec![],
                typed_environment: [("os".into(), EnvironmentValue::Text("windows".into()))].into(),
            },
        })
        .await
        .unwrap();
    let mut input = f.input("coffee");
    input.reported_environment = [("os".into(), EnvironmentValue::Text("linux".into()))].into();
    // Fake rejects any input text containing "foreign", proving exclusion before transmission.
    let port = Fake {
        profile: EmbeddingProfile {
            dimensions: 2,
            ..Default::default()
        },
    };
    let result = recall_memory_enhanced::execute_with_embedding(
        &f.store,
        input,
        &Default::default(),
        Some(&port),
    )
    .await
    .unwrap();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].reference, "event:allowed");
    assert_eq!(result.applicability_omissions, 1);
}

#[tokio::test]
async fn literal_pre_typed_environment_persisted_request_replays() {
    let f = Fixture::new().await;
    let payload = format!(
        r#"{{"namespace":"project/a","reference":"event:old","content_digest":"{}","valid_from":null,"valid_until":null,"conditions":[],"environment":[]}}"#,
        "a".repeat(64)
    );
    let request =
        format!(r#"{{"request_id":"old","expected_revision":0,"declaration":{payload}}}"#);
    sqlx::query("INSERT INTO recall_applicability(namespace,reference,revision,request_id,request_json,payload_json) VALUES('project/a','event:old',1,'old',?,?)")
        .bind(&request).bind(&payload).execute(&f.pool).await.unwrap();
    let replay = f
        .store
        .declare_recall_applicability(serde_json::from_str(&request).unwrap())
        .await
        .unwrap();
    assert_eq!(replay.revision, 1);
    assert!(replay.declaration.typed_environment.is_empty());
}
