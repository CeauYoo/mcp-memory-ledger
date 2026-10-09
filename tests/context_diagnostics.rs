use agent_llm_mm::{
    adapters::sqlite::{SqliteStore, open_read_only_current_database},
    application::build_task_context::{self, BuildTaskContextInput, BuildTaskContextResult},
    domain::{
        event::EventReference,
        experience::{CreateEpisodeRequest, EpisodeContent},
        types::{MemoryScope, Namespace},
    },
    ports::{
        experience_store::ExperienceStore,
        text_memory_store::{
            CLAIM_STATUS_SAMPLE_LIMIT, LINKED_EPISODE_LIMIT, LinkedEpisodeQuery, TextMemoryQuery,
            TextMemoryStore,
        },
    },
};
use sqlx::SqlitePool;

struct Fixture {
    _dir: tempfile::TempDir,
    url: String,
    store: SqliteStore,
    pool: SqlitePool,
}
const NS: &str = "user/alice";
impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", dir.path().join("context.sqlite").display());
        let store = SqliteStore::bootstrap(&url).await.unwrap();
        let pool = SqlitePool::connect(&url).await.unwrap();
        Self {
            _dir: dir,
            url,
            store,
            pool,
        }
    }
    async fn claim(&self, id: &str, ns: &str, object: &str, status: &str) {
        sqlx::query("INSERT INTO claims(claim_id,owner,namespace,subject,predicate,object,mode,status) VALUES (?,'user',?,'person','likes',?,'observed',?)")
            .bind(id).bind(ns).bind(object).bind(status).execute(&self.pool).await.unwrap();
    }
    async fn event(&self, id: &str, ns: &str, summary: &str) {
        sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES (?,'2000-01-01T00:00:00Z','user',?,'observation',?)")
            .bind(id).bind(ns).bind(summary).execute(&self.pool).await.unwrap();
    }
    async fn episode(&self, id: &str, ns: &str, sources: &[&str]) {
        self.store
            .create_episode(CreateEpisodeRequest {
                namespace: ns.into(),
                request_id: id.into(),
                episode_id: id.into(),
                content: EpisodeContent {
                    title: "Whole episode 北京".into(),
                    objective: "Find useful coffee".into(),
                    actions: vec!["Observe".into()],
                    observations: vec!["Recorded source".into()],
                    outcome: "Found coffee".into(),
                    lesson: "Check the source 🧠".into(),
                    limitations: vec!["One observation; not a universal conclusion".into()],
                    source_event_refs: sources.iter().map(|s| format!("event:{s}")).collect(),
                },
            })
            .await
            .unwrap();
    }
    async fn context(&self, query: &str, limit: usize, max_bytes: usize) -> BuildTaskContextResult {
        build_task_context::execute(&self.store, input(query, limit, max_bytes))
            .await
            .unwrap()
    }
}
fn input(query: &str, limit: usize, max_bytes: usize) -> BuildTaskContextInput {
    BuildTaskContextInput {
        namespace: Namespace::parse(NS).unwrap(),
        query: query.into(),
        limit,
        max_bytes,
    }
}
fn scope() -> MemoryScope {
    MemoryScope::for_namespace(Namespace::parse(NS).unwrap())
}
fn assert_budget(context: &BuildTaskContextResult) {
    assert_eq!(
        serde_json::to_vec(context).unwrap().len(),
        context.serialized_bytes
    );
    assert!(context.serialized_bytes <= context.max_bytes);
}

#[tokio::test]
async fn observable_statuses_are_scoped_factual_and_read_only() {
    let f = Fixture::new().await;
    for (id, value, status) in [
        ("active-a", "coffee black", "active"),
        ("active-b", "coffee white", "active"),
        ("active-duplicate", "coffee black", "active"),
        ("disputed", "coffee disputed", "disputed"),
        ("superseded", "coffee old", "superseded"),
        ("unrelated", "tea", "disputed"),
    ] {
        f.claim(id, NS, value, status).await;
    }
    f.claim("foreign-secret", "user/bob", "coffee", "disputed")
        .await;
    // An old recording date does not establish expiry, nor verify the Claim.
    sqlx::query("UPDATE claims SET recorded_at='1990-01-01T00:00:00Z', observed_at='1980-01-01T00:00:00Z' WHERE claim_id='active-a'").execute(&f.pool).await.unwrap();
    let before: Vec<(String, String)> =
        sqlx::query_as("SELECT claim_id,status FROM claims ORDER BY claim_id")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    let readonly = open_read_only_current_database(&f.url)
        .await
        .unwrap()
        .unwrap();
    let context = build_task_context::execute(&readonly, input("coffee missing", 20, 16000))
        .await
        .unwrap();
    assert_budget(&context);
    let d = context.diagnostics.as_ref().unwrap();
    assert_eq!(d.expiry, "not_evaluated_no_expiry_contract");
    let status = d.matching_claim_status.as_ref().unwrap();
    assert_eq!(status.disputed.sampled_match_count, 1);
    assert_eq!(status.disputed.sampled_claim_count, 2);
    assert!(status.disputed.scope_scan_complete);
    assert_eq!(status.disputed.claim_references, ["claim:disputed"]);
    assert_eq!(status.superseded.sampled_match_count, 1);
    assert_eq!(d.returned_records.active_unverified_claim_count, 3);
    assert_eq!(d.returned_records.unknown_recorded_at.count, 2);
    assert_eq!(d.returned_records.claims_without_evidence_refs.count, 3);
    assert_eq!(
        d.returned_records.query_terms_without_returned_match,
        ["missing"]
    );
    assert_eq!(d.returned_records.possible_differing_values.group_count, 1);
    assert_eq!(
        d.returned_records.possible_differing_values.groups[0].count,
        3
    );
    assert!(
        d.returned_records
            .possible_differing_values
            .interpretation
            .contains("not_inferred_contradiction")
    );
    assert!(
        !serde_json::to_string(&context)
            .unwrap()
            .contains("foreign-secret")
    );
    let after: Vec<(String, String)> =
        sqlx::query_as("SELECT claim_id,status FROM claims ORDER BY claim_id")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    assert_eq!(before, after);
    let one = f.context("coffee", 1, 16000).await;
    assert_eq!(
        one.diagnostics
            .unwrap()
            .returned_records
            .possible_differing_values
            .group_count,
        0
    );
    let absent = f.context("notfound", 10, 16000).await;
    assert!(
        absent
            .diagnostics
            .as_ref()
            .unwrap()
            .returned_records
            .no_records
    );
    assert_eq!(
        absent
            .diagnostics
            .unwrap()
            .returned_records
            .query_terms_without_returned_match,
        ["notfound"]
    );
}

#[tokio::test]
async fn status_samples_have_honest_lower_bounds_and_bounded_references() {
    let f = Fixture::new().await;
    for i in 0..CLAIM_STATUS_SAMPLE_LIMIT {
        f.claim(&format!("local-{i:04}"), NS, "unrelated", "disputed")
            .await;
        f.claim(
            &format!("aaa-foreign-{i:04}"),
            "user/bob",
            "coffee",
            "disputed",
        )
        .await;
    }
    f.claim("z-beyond-window", NS, "coffee", "disputed").await;
    for i in 0..20 {
        f.claim(&format!("old-{i:02}"), NS, "coffee", "superseded")
            .await;
    }
    let scan = f
        .store
        .inspect_text_claim_status(TextMemoryQuery {
            scope: scope(),
            terms: vec!["coffee".into()],
            limit: 1,
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(scan.disputed.sampled_claim_count, CLAIM_STATUS_SAMPLE_LIMIT);
    assert_eq!(scan.disputed.sampled_match_count, 0);
    assert!(!scan.disputed.scope_scan_complete);
    assert!(scan.disputed.claim_references.is_empty());
    assert_eq!(scan.superseded.sampled_match_count, 20);
    assert_eq!(scan.superseded.claim_references.len(), 8);
    assert!(scan.superseded.references_truncated);
    assert!(scan.superseded.scope_scan_complete);
    assert_eq!(scan.superseded.claim_references[0], "claim:old-00");
    assert!(
        f.store
            .inspect_text_claim_status(TextMemoryQuery {
                scope: MemoryScope::legacy_unscoped(),
                terms: vec!["coffee".into()],
                limit: 1
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn whole_rich_episodes_follow_primary_records_and_share_the_exact_budget() {
    let f = Fixture::new().await;
    f.event("source", NS, "coffee 北京").await;
    f.claim("active", NS, "coffee", "active").await;
    sqlx::query("INSERT INTO evidence_links(claim_id,event_id) VALUES ('active','source')")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO episode_events(episode_reference,event_id) VALUES ('legacy:coffee','source')",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    f.episode("rich:coffee", NS, &["source"]).await;
    let full = f.context("coffee", 10, 16000).await;
    assert_eq!(full.records.len(), 2);
    assert!(
        serde_json::to_string(&full.records)
            .unwrap()
            .contains("legacy:coffee")
    );
    assert_eq!(full.rich_episodes.len(), 1);
    assert_eq!(full.rich_episodes[0].content.lesson, "Check the source 🧠");
    assert_eq!(
        full.rich_episodes[0].content.source_event_refs,
        ["event:source"]
    );
    assert_budget(&full);
    let mut saw_omitted = false;
    for budget in (550..5500).step_by(71) {
        if let Ok(context) =
            build_task_context::execute(&f.store, input("coffee", 10, budget)).await
        {
            assert_budget(&context);
            assert_eq!(context.records.len() + context.omissions.byte_budget, 2);
            if context.omissions.diagnostics == Some("byte_budget") {
                saw_omitted = true;
            }
            if context.records.is_empty() {
                assert!(context.rich_episodes.is_empty());
            }
            for record in &context.rich_episodes {
                assert_eq!(record.content, full.rich_episodes[0].content);
            }
        }
    }
    assert!(saw_omitted);
    let exact = f.context("coffee", 10, full.serialized_bytes).await;
    assert_budget(&exact);
    assert_eq!(exact.records, full.records);
    assert_eq!(exact.rich_episodes, full.rich_episodes);
}

#[tokio::test]
async fn rich_episode_scope_payload_and_all_original_sources_are_revalidated() {
    let f = Fixture::new().await;
    f.event("anchor", NS, "coffee").await;
    f.event("moved", NS, "other").await;
    f.event("quarantined", NS, "other").await;
    f.event("foreign-source", "user/bob", "coffee secret").await;
    f.episode("a-moved", NS, &["anchor", "moved"]).await;
    f.episode("b-edge-mismatch", NS, &["anchor"]).await;
    f.episode("c-payload-mismatch", NS, &["anchor"]).await;
    f.episode("d-unknown-owner", NS, &["anchor", "quarantined"])
        .await;
    f.episode("e-identity-mismatch", NS, &["anchor"]).await;
    f.episode("f-missing-edge", NS, &["anchor", "quarantined"])
        .await;
    f.episode("z-good", NS, &["anchor"]).await;
    f.episode("aaa-foreign-episode", "user/bob", &["foreign-source"])
        .await;
    sqlx::query("UPDATE events SET namespace='user/bob' WHERE event_id='moved'")
        .execute(&f.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO experience_episode_sources(episode_id,event_id) VALUES ('b-edge-mismatch','foreign-source'),('aaa-foreign-episode','anchor')").execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE experience_episodes SET payload_json=json_set(payload_json,'$.namespace','user/bob') WHERE episode_id='c-payload-mismatch'").execute(&f.pool).await.unwrap();
    sqlx::query(
        "UPDATE events SET owner='unknown', namespace='world' WHERE event_id='quarantined'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE experience_episodes SET payload_json=json_set(payload_json,'$.episode_id','foreign-secret-id') WHERE episode_id='e-identity-mismatch'").execute(&f.pool).await.unwrap();
    sqlx::query("DELETE FROM experience_episode_sources WHERE episode_id='f-missing-edge' AND event_id='quarantined'").execute(&f.pool).await.unwrap();
    let result = f
        .store
        .linked_episodes(LinkedEpisodeQuery {
            scope: scope(),
            event_references: vec![
                EventReference::parse("event:anchor").unwrap(),
                EventReference::parse("event:foreign-source").unwrap(),
            ],
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].episode_id, "z-good");
    assert_eq!(result.unavailable, 6);
    assert!(!result.has_more);
    let context = f.context("coffee", 10, 16000).await;
    let encoded = serde_json::to_string(&context).unwrap();
    assert!(!encoded.contains("foreign-source"));
    assert!(!encoded.contains("aaa-foreign-episode"));
    assert!(!encoded.contains("moved"));
    assert!(!encoded.contains("quarantined"));
    assert!(!encoded.contains("foreign-secret-id"));
}

#[tokio::test]
async fn linked_episode_fanout_is_deduplicated_ordered_and_limited_after_scope() {
    let f = Fixture::new().await;
    f.event("anchor", NS, "coffee").await;
    f.event("other", NS, "coffee").await;
    f.event("foreign", "user/bob", "coffee").await;
    for i in 0..12 {
        f.episode(&format!("local-{i:02}"), NS, &["anchor", "other"])
            .await;
        f.episode(&format!("aaa-foreign-{i:02}"), "user/bob", &["foreign"])
            .await;
        sqlx::query(
            "INSERT INTO experience_episode_sources(episode_id,event_id) VALUES (?,'anchor')",
        )
        .bind(format!("aaa-foreign-{i:02}"))
        .execute(&f.pool)
        .await
        .unwrap();
    }
    let query = LinkedEpisodeQuery {
        scope: scope(),
        event_references: vec![
            EventReference::parse("anchor").unwrap(),
            EventReference::parse("other").unwrap(),
        ],
    };
    let page = f
        .store
        .linked_episodes(query.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page, f.store.linked_episodes(query).await.unwrap().unwrap());
    assert_eq!(page.records.len(), LINKED_EPISODE_LIMIT);
    assert!(page.has_more);
    assert_eq!(page.records[0].episode_id, "local-00");
    assert_eq!(page.records[7].episode_id, "local-07");
    assert!(
        f.store
            .linked_episodes(LinkedEpisodeQuery {
                scope: scope(),
                event_references: vec![EventReference::parse("anchor").unwrap(); 65]
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn context_exposes_omitted_source_window_without_claiming_exhaustive_episode_recall() {
    let f = Fixture::new().await;
    f.claim("coffee-claim", NS, "coffee", "active").await;
    for i in 0..65 {
        let id = format!("source-{i:03}");
        f.event(&id, NS, "background observation").await;
        sqlx::query("INSERT INTO evidence_links(claim_id,event_id) VALUES ('coffee-claim',?)")
            .bind(&id)
            .execute(&f.pool)
            .await
            .unwrap();
    }
    f.episode("only-beyond-source-window", NS, &["source-064"])
        .await;
    let context = f.context("coffee", 10, 32000).await;
    assert_budget(&context);
    assert_eq!(context.records.len(), 1);
    assert!(context.rich_episodes.is_empty());
    let omissions = context.omissions.episodes.unwrap();
    assert_eq!(omissions.source_limit, 1);
    assert_eq!(omissions.candidate_limit, 0);
    assert!(omissions.rich_lookup_supported);
    // The original primary provenance is never shortened to meet the source cap.
    assert!(
        serde_json::to_string(&context.records)
            .unwrap()
            .contains("event:source-064")
    );
}
