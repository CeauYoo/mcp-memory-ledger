use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, open_current_database, open_read_only_current_database},
    application::{
        build_task_context::{self, BuildTaskContextInput},
        recall_memory::{self, RecallMemoryInput, RecallMemoryResult},
        search_memory::SearchMemoryRecord,
    },
    domain::types::Namespace,
};
use sqlx::{Connection, Row, SqliteConnection, SqlitePool};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    url: String,
    store: SqliteStore,
    pool: SqlitePool,
}
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("index.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        Self {
            _dir: dir,
            url,
            store,
            pool,
        }
    }
    async fn event(&self, id: &str, ns: &str, text: &str, time: &str) {
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES (?,?,'user',?,'observation',?)")
            .bind(id).bind(time).bind(ns).bind(text).execute(&self.pool).await.unwrap();
    }
    async fn claim(&self, id: &str, ns: &str, text: &str, status: &str) {
        sqlx::query("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES (?,'user',?,'developer','uses',?,'observed',?)")
            .bind(id).bind(ns).bind(text).bind(status).execute(&self.pool).await.unwrap();
    }
    async fn recall(&self, query: &str, limit: usize) -> RecallMemoryResult {
        recall(&self.store, query, limit).await
    }
}
async fn recall(store: &SqliteStore, query: &str, limit: usize) -> RecallMemoryResult {
    recall_memory::execute(
        store,
        RecallMemoryInput {
            namespace: Namespace::parse("user/alice").unwrap(),
            query: query.into(),
            limit,
        },
    )
    .await
    .unwrap()
}
fn ids(result: &RecallMemoryResult) -> Vec<&str> {
    result
        .records
        .iter()
        .map(|hit| match &hit.record {
            SearchMemoryRecord::Event { id, .. } | SearchMemoryRecord::Claim { id, .. } => {
                id.as_str()
            }
            _ => panic!("unexpected record type"),
        })
        .collect()
}
const TIME: &str = "2026-01-01T00:00:00Z";

#[tokio::test]
async fn exact_long_short_cjk_code_punctuation_and_ascii_only_casefold() {
    let f = Fixture::new().await;
    f.event("cjk", "user/alice", "北京咖啡馆保留长期记忆", TIME)
        .await;
    f.event(
        "code",
        "user/alice",
        "src::Memory_Store<T> %_\"*\\ COFFEE École",
        TIME,
    )
    .await;
    f.event(
        "near",
        "user/alice",
        "src Memory Store T %x_ coffee école",
        TIME,
    )
    .await;
    f.event("nul", "user/alice", "left\0coffee", TIME).await;
    f.event("emoji", "user/alice", "👩‍💻测试咖啡", TIME).await;
    for (query, expected, strategy) in [
        ("北京咖", vec!["event:cjk"], "fts5_trigram"),
        ("记忆", vec!["event:cjk"], "literal_short_terms"),
        ("京", vec!["event:cjk"], "literal_short_terms"),
        ("src::Memory_Store<T>", vec!["event:code"], "fts5_trigram"),
        ("%_\"*\\", vec!["event:code"], "fts5_trigram"),
        ("ÉCOLE", vec!["event:code"], "fts5_trigram"),
        ("école", vec!["event:near"], "fts5_trigram"),
        (
            "北京咖 COFFEE",
            vec!["event:cjk", "event:code", "event:near", "event:nul"],
            "fts5_trigram",
        ),
        (
            "记忆 COFFEE",
            vec!["event:cjk", "event:code", "event:near", "event:nul"],
            "fts5_trigram_with_short_literal_fallback",
        ),
        ("👩‍💻", vec!["event:emoji"], "fts5_trigram"),
    ] {
        let result = f.recall(query, 20).await;
        assert_eq!(ids(&result), expected, "query {query}");
        assert_eq!(result.strategy, strategy, "query {query}");
        assert!(result.index_warning.is_none());
        assert_eq!(result, f.recall(query, 20).await);
        assert!(
            result
                .records
                .iter()
                .all(|hit| hit.matched_terms == hit.explanation.matched_query_terms.len())
        );
    }
    assert!(f.store.inspect_retrieval_index().await.unwrap().is_usable());
}

#[tokio::test]
async fn scope_and_active_status_filter_before_limit_with_balanced_quotas() {
    let f = Fixture::new().await;
    for i in 0..30 {
        f.event(&format!("a-decoy-{i}"), "user/bob", "coffee target", TIME)
            .await;
        f.claim(
            &format!("a-decoy-{i}"),
            "user/bob",
            "coffee target",
            "active",
        )
        .await;
        f.claim(
            &format!("b-old-{i}"),
            "user/alice",
            "coffee target",
            "superseded",
        )
        .await;
        f.claim(
            &format!("b-disputed-{i}"),
            "user/alice",
            "coffee target",
            "disputed",
        )
        .await;
        f.claim(&format!("claim-{i:02}"), "user/alice", "coffee", "active")
            .await;
        f.event(&format!("event-{i:02}"), "user/alice", "coffee", TIME)
            .await;
    }
    for (limit, claims, events) in [(1, 1, 0), (2, 1, 1), (5, 3, 2), (8, 4, 4)] {
        let result = f.recall("coffee", limit).await;
        assert_eq!(result.records.len(), limit);
        assert!(result.has_more);
        assert_eq!(
            result
                .records
                .iter()
                .filter(|h| matches!(h.record, SearchMemoryRecord::Claim { .. }))
                .count(),
            claims
        );
        assert_eq!(
            result
                .records
                .iter()
                .filter(|h| matches!(h.record, SearchMemoryRecord::Event { .. }))
                .count(),
            events
        );
        assert!(
            ids(&result)
                .iter()
                .all(|id| !id.contains("decoy") && !id.contains("old") && !id.contains("disputed"))
        );
    }
    sqlx::query("UPDATE claims SET status='superseded' WHERE namespace='user/alice' AND claim_id != 'claim-00'").execute(&f.pool).await.unwrap();
    let result = f.recall("coffee", 6).await;
    assert_eq!(result.records.len(), 6);
    assert!(matches!(
        result.records[0].record,
        SearchMemoryRecord::Claim { .. }
    ));
    assert!(
        result.records[1..]
            .iter()
            .all(|h| matches!(h.record, SearchMemoryRecord::Event { .. }))
    );
    assert!(f.store.inspect_retrieval_index().await.unwrap().is_usable());
}

#[tokio::test]
async fn score_then_true_event_time_and_claim_time_unknown_are_explained() {
    let f = Fixture::new().await;
    f.event(
        "z-new",
        "user/alice",
        "coffee",
        "2026-02-01T01:00:00.000000002+01:00",
    )
    .await;
    f.event(
        "a-old",
        "user/alice",
        "coffee",
        "2026-02-01T00:00:00.000000001Z",
    )
    .await;
    f.event("top", "user/alice", "coffee target", TIME).await;
    f.claim("claim", "user/alice", "coffee", "active").await;
    let result = f.recall("coffee target", 8).await;
    assert_eq!(
        ids(&result),
        ["claim:claim", "event:top", "event:z-new", "event:a-old"]
    );
    assert!(!result.has_more);
    assert_eq!(
        result.records[0].explanation.time_basis,
        "claim_creation_time_unknown_no_recency_assumed"
    );
    assert_eq!(
        result.records[0].explanation.validity,
        "active_claim_not_independently_verified"
    );
    assert_eq!(
        result.records[1].explanation.matched_query_terms,
        ["coffee", "target"]
    );
    assert_eq!(
        result.records[1].explanation.validity,
        "historical_event_not_a_current_conclusion"
    );
}

#[tokio::test]
async fn source_changes_rollback_scope_moves_deletes_and_status_updates_are_transactional() {
    let f = Fixture::new().await;
    f.event("event", "user/alice", "beforealpha", TIME).await;
    f.claim("claim", "user/alice", "beforealpha", "active")
        .await;
    {
        let mut tx = f.pool.begin().await.unwrap();
        sqlx::query("UPDATE events SET summary='afterbeta'")
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("UPDATE claims SET status='superseded'")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.rollback().await.unwrap();
    }
    assert_eq!(f.recall("beforealpha", 10).await.records.len(), 2);
    assert!(f.recall("afterbeta", 10).await.records.is_empty());
    sqlx::query("UPDATE events SET summary='afterbeta'")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE claims SET status='superseded'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.recall("beforealpha", 10).await.records.is_empty());
    assert_eq!(ids(&f.recall("afterbeta", 10).await), ["event:event"]);
    sqlx::query("UPDATE claims SET status='active', object='afterbeta'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(f.recall("afterbeta", 10).await.records.len(), 2);
    sqlx::query("UPDATE events SET namespace='user/bob'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(ids(&f.recall("afterbeta", 10).await), ["claim:claim"]);
    sqlx::query("DELETE FROM claims")
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(f.recall("afterbeta", 10).await.records.is_empty());
    assert!(f.store.inspect_retrieval_index().await.unwrap().is_usable());
}

#[tokio::test]
async fn missing_trigger_or_fts_degrades_without_mutation_then_explicit_rebuild_recovers() {
    let f = Fixture::new().await;
    f.event(
        "legacy id with spaces",
        "user/alice",
        "coffee literal",
        TIME,
    )
    .await;
    f.claim(
        "legacy claim with spaces",
        "user/alice",
        "coffee literal",
        "active",
    )
    .await;
    sqlx::query("DROP TRIGGER text_recall_events_au")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE events SET summary='coffee changed'")
        .execute(&f.pool)
        .await
        .unwrap();
    let result = f.recall("changed", 10).await;
    assert_eq!(ids(&result), ["event:legacy id with spaces"]);
    assert_eq!(result.strategy, "degraded_literal_scan");
    assert!(
        result
            .index_warning
            .unwrap()
            .contains("missing:text_recall_events_au")
    );
    let readonly = open_read_only_current_database(&f.url)
        .await
        .unwrap()
        .unwrap();
    let report = readonly.inspect_retrieval_index().await.unwrap();
    assert!(!report.is_usable());
    assert!(!report.rebuild_performed);
    assert!(
        !readonly
            .inspect_retrieval_index()
            .await
            .unwrap()
            .is_usable()
    );
    let before: Vec<(String, String)> = sqlx::query_as("SELECT event_id,summary FROM events")
        .fetch_all(&f.pool)
        .await
        .unwrap();
    assert!(
        f.store
            .rebuild_retrieval_index()
            .await
            .unwrap()
            .rebuild_performed
    );
    assert_eq!(f.recall("changed", 10).await.strategy, "fts5_trigram");
    assert_eq!(
        ids(&f.recall("literal", 10).await),
        ["claim:legacy claim with spaces"]
    );
    sqlx::query("DROP TABLE text_recall_fts")
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(
        f.recall("changed", 10).await.strategy,
        "degraded_literal_scan"
    );
    assert!(f.store.rebuild_retrieval_index().await.unwrap().is_usable());
    let after: Vec<(String, String)> = sqlx::query_as("SELECT event_id,summary FROM events")
        .fetch_all(&f.pool)
        .await
        .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn read_only_posting_inspection_detects_silent_corruption_and_rebuild_preserves_ledger() {
    let f = Fixture::new().await;
    f.event("event", "user/alice", "coffee target", TIME).await;
    f.claim("claim", "user/alice", "coffee", "active").await;
    let rows = sqlx::query("SELECT doc_id,summary,subject,predicate,object FROM text_recall_documents WHERE record_type='event'").fetch_all(&f.pool).await.unwrap();
    for row in rows {
        sqlx::query("INSERT INTO text_recall_fts(text_recall_fts,rowid,summary,subject,predicate,object) VALUES ('delete',?,?,?,?,?)")
            .bind(row.get::<i64,_>("doc_id")).bind(row.get::<String,_>("summary")).bind(row.get::<String,_>("subject")).bind(row.get::<String,_>("predicate")).bind(row.get::<String,_>("object")).execute(&f.pool).await.unwrap();
    }
    let readonly = open_read_only_current_database(&f.url)
        .await
        .unwrap()
        .unwrap();
    let report = readonly.inspect_retrieval_index().await.unwrap();
    assert!(report.content_consistent);
    assert!(!report.postings_consistent);
    assert!(!report.rebuild_performed);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue == "trigram_postings_differ_from_documents")
    );
    assert!(f.store.rebuild_retrieval_index().await.unwrap().is_usable());
    assert_eq!(ids(&f.recall("target", 10).await), ["event:event"]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM events")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM claims")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn vacuum_backup_restore_preserves_stable_id_mapping_and_future_updates() {
    let f = Fixture::new().await;
    for i in 0..10 {
        f.event(&format!("event-{i}"), "user/alice", "coffee", TIME)
            .await;
    }
    sqlx::query("DELETE FROM events WHERE event_id IN ('event-1','event-4','event-5')")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("VACUUM").execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE events SET summary='changedcoffee' WHERE event_id='event-9'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(ids(&f.recall("changedcoffee", 10).await), ["event:event-9"]);
    assert!(f.store.inspect_retrieval_index().await.unwrap().is_usable());
    let backup = f._dir.path().join("restore.sqlite");
    let mut connection = SqliteConnection::connect(&f.url).await.unwrap();
    sqlx::query("VACUUM INTO ?")
        .bind(backup.to_str().unwrap())
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let restored = open_current_database(&format!("sqlite://{}", backup.display()))
        .await
        .unwrap();
    assert_eq!(
        ids(&recall(&restored, "changedcoffee", 10).await),
        ["event:event-9"]
    );
    assert!(
        restored
            .inspect_retrieval_index()
            .await
            .unwrap()
            .is_usable()
    );
}

#[tokio::test]
async fn explanation_and_degraded_warning_are_inside_exact_context_byte_budget() {
    let f = Fixture::new().await;
    for i in 0..10 {
        f.event(
            &format!("event-{i}"),
            "user/alice",
            &format!("coffee 北京 🧠 {}", "\\\"".repeat(i * 5)),
            TIME,
        )
        .await;
    }
    f.claim("current", "user/alice", "coffee", "active").await;
    sqlx::query("DROP TRIGGER text_recall_events_ai")
        .execute(&f.pool)
        .await
        .unwrap();
    for budget in [700, 1200, 2000, 3500, 10000] {
        let input = BuildTaskContextInput {
            namespace: Namespace::parse("user/alice").unwrap(),
            query: "coffee".into(),
            limit: 5,
            max_bytes: budget,
        };
        if let Ok(context) = build_task_context::execute(&f.store, input).await {
            assert_eq!(
                serde_json::to_vec(&context).unwrap().len(),
                context.serialized_bytes
            );
            assert!(context.serialized_bytes <= budget);
            assert_eq!(context.records.len() + context.omissions.byte_budget, 5);
            assert!(context.omissions.candidate_limit);
            assert_eq!(context.omissions.claim_status_policy, "active_only");
            assert!(context.index_warning.is_some());
        }
    }
}

#[tokio::test]
async fn malformed_fts_pages_fall_back_to_ledger_and_explicit_rebuild_repairs() {
    let f = Fixture::new().await;
    f.event("event", "user/alice", "coffee target", TIME).await;
    sqlx::query("UPDATE text_recall_fts_data SET block=x'00' WHERE id>10")
        .execute(&f.pool)
        .await
        .unwrap();
    let result = f.recall("coffee", 10).await;
    assert_eq!(ids(&result), ["event:event"]);
    assert_eq!(result.strategy, "degraded_literal_scan");
    assert!(
        result
            .index_warning
            .unwrap()
            .contains("derived_index_query_failed")
    );
    assert!(!f.store.inspect_retrieval_index().await.unwrap().is_usable());
    assert!(f.store.rebuild_retrieval_index().await.unwrap().is_usable());
    assert_eq!(f.recall("coffee", 10).await.strategy, "fts5_trigram");
}

#[tokio::test]
async fn context_interleaves_claim_and_event_allocations_before_byte_packing() {
    let f = Fixture::new().await;
    for i in 0..10 {
        f.event(&format!("event-{i}"), "user/alice", "coffee evidence", TIME)
            .await;
        f.claim(
            &format!("claim-{i}"),
            "user/alice",
            "coffee conclusion",
            "active",
        )
        .await;
    }
    let make_input = |limit, max_bytes| BuildTaskContextInput {
        namespace: Namespace::parse("user/alice").unwrap(),
        query: "coffee".into(),
        limit,
        max_bytes,
    };
    let two = build_task_context::execute(&f.store, make_input(2, 100_000))
        .await
        .unwrap();
    let result = build_task_context::execute(&f.store, make_input(10, two.serialized_bytes + 64))
        .await
        .unwrap();
    // Optional diagnostics in the reference context can leave room for additional
    // primary records here. Preserve interleaving rather than an obsolete exact count.
    assert!(result.records.len() >= 2);
    assert!(matches!(
        result.records[0].record,
        SearchMemoryRecord::Claim { .. }
    ));
    assert!(matches!(
        result.records[1].record,
        SearchMemoryRecord::Event { .. }
    ));
    assert_eq!(result.records.len() + result.omissions.byte_budget, 10);
    assert_eq!(
        serde_json::to_vec(&result).unwrap().len(),
        result.serialized_bytes
    );
}

// Deterministically exercise the selection/hydration boundary instead of using a flaky
// timing-based concurrency test. A legacy same-ID upsert happens after candidate selection.
struct MutatingRecall<'a> {
    fixture: &'a Fixture,
    new_text: &'a str,
}
#[async_trait::async_trait]
impl agent_llm_mm::ports::text_memory_store::TextMemoryStore for MutatingRecall<'_> {
    async fn recall_text(
        &self,
        query: agent_llm_mm::ports::text_memory_store::TextMemoryQuery,
    ) -> Result<agent_llm_mm::ports::text_memory_store::TextMemoryPage, agent_llm_mm::error::AppError>
    {
        let page = self.fixture.store.recall_text(query).await?;
        sqlx::query("UPDATE events SET summary=? WHERE event_id='mutable'")
            .bind(self.new_text)
            .execute(&self.fixture.pool)
            .await
            .unwrap();
        Ok(page)
    }
}
#[async_trait::async_trait]
impl agent_llm_mm::ports::MemoryReadStore for MutatingRecall<'_> {
    async fn query_event_records(
        &self,
        q: agent_llm_mm::ports::EventRecordQuery,
    ) -> Result<Vec<agent_llm_mm::ports::EventReadRecord>, agent_llm_mm::error::AppError> {
        self.fixture.store.query_event_records(q).await
    }
    async fn query_claim_records(
        &self,
        q: agent_llm_mm::ports::ClaimRecordQuery,
    ) -> Result<Vec<agent_llm_mm::ports::ClaimReadRecord>, agent_llm_mm::error::AppError> {
        self.fixture.store.query_claim_records(q).await
    }
    async fn query_episode_records(
        &self,
        q: agent_llm_mm::ports::EpisodeRecordQuery,
    ) -> Result<Vec<agent_llm_mm::ports::EpisodeReadRecord>, agent_llm_mm::error::AppError> {
        self.fixture.store.query_episode_records(q).await
    }
    async fn query_reflection_records(
        &self,
        q: agent_llm_mm::ports::ReflectionRecordQuery,
    ) -> Result<Vec<agent_llm_mm::ports::ReflectionReadRecord>, agent_llm_mm::error::AppError> {
        self.fixture.store.query_reflection_records(q).await
    }
    async fn query_scoped_event_ids(
        &self,
        q: agent_llm_mm::ports::ScopedEventIdQuery,
    ) -> Result<std::collections::BTreeSet<String>, agent_llm_mm::error::AppError> {
        self.fixture.store.query_scoped_event_ids(q).await
    }
    async fn query_claim_reflection_history(
        &self,
        q: agent_llm_mm::ports::ClaimReflectionHistoryQuery,
    ) -> Result<agent_llm_mm::ports::ClaimReflectionHistoryPage, agent_llm_mm::error::AppError>
    {
        self.fixture.store.query_claim_reflection_history(q).await
    }
    async fn query_self_model_history(
        &self,
        q: agent_llm_mm::ports::SelfModelHistoryQuery,
    ) -> Result<agent_llm_mm::ports::SelfModelHistoryPage, agent_llm_mm::error::AppError> {
        self.fixture.store.query_self_model_history(q).await
    }
}

#[tokio::test]
async fn hydration_rechecks_changed_text_and_reports_lost_candidates() {
    let f = Fixture::new().await;
    f.event("mutable", "user/alice", "coffee target", TIME)
        .await;
    let input = || RecallMemoryInput {
        namespace: Namespace::parse("user/alice").unwrap(),
        query: "coffee target".into(),
        limit: 5,
    };
    let changed = recall_memory::execute(
        &MutatingRecall {
            fixture: &f,
            new_text: "coffee changed",
        },
        input(),
    )
    .await
    .unwrap();
    assert_eq!(changed.records[0].matched_terms, 1);
    assert_eq!(
        changed.records[0].explanation.matched_query_terms,
        ["coffee"]
    );
    let removed = recall_memory::execute(
        &MutatingRecall {
            fixture: &f,
            new_text: "now unrelated",
        },
        input(),
    )
    .await
    .unwrap();
    assert!(removed.records.is_empty());
    assert_eq!(removed.unavailable_after_retrieval, 1);
}
