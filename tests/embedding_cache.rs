//! Disposable cache contracts use deterministic synthetic vectors, no network.
use agent_llm_mm::{
    adapters::{cached_embedding::CachedEmbeddingAdapter, sqlite::SqliteStore},
    domain::types::Namespace,
    error::AppError,
    ports::embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
};
use async_trait::async_trait;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
struct FixtureEmbedding {
    profile: EmbeddingProfile,
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl EmbeddingPort for FixtureEmbedding {
    fn profile(&self) -> &EmbeddingProfile {
        &self.profile
    }
    async fn embed(
        &self,
        _: &Namespace,
        _: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(texts.iter().map(|_| vec![0.25, 0.75]).collect())
    }
}
fn inner(calls: Arc<AtomicUsize>, prefix: &str) -> Arc<dyn EmbeddingPort> {
    Arc::new(FixtureEmbedding {
        calls,
        profile: EmbeddingProfile {
            model: "synthetic".into(),
            model_digest: "a".repeat(64),
            dimensions: 2,
            document_prefix: prefix.into(),
            query_prefix: "query".into(),
        },
    })
}
#[tokio::test]
async fn cache_scopes_profile_content_and_purpose_and_clear_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&dir.path().join("cache.sqlite"));
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    store.rebuild_embedding_cache().await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = CachedEmbeddingAdapter::new(
        inner(calls.clone(), "doc"),
        store.clone(),
        vec!["project/a".into(), "project/b".into()],
    )
    .unwrap();
    let ns = Namespace::for_project("a");
    for _ in 0..2 {
        assert_eq!(
            cache
                .embed(&ns, EmbeddingPurpose::Document, &["fixture".into()])
                .await
                .unwrap(),
            vec![vec![0.25, 0.75]]
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    cache
        .embed(
            &Namespace::for_project("b"),
            EmbeddingPurpose::Document,
            &["fixture".into()],
        )
        .await
        .unwrap();
    cache
        .embed(&ns, EmbeddingPurpose::Query, &["fixture".into()])
        .await
        .unwrap();
    cache
        .embed(&ns, EmbeddingPurpose::Document, &["changed".into()])
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    let changed = CachedEmbeddingAdapter::new(
        inner(calls.clone(), "new-prefix"),
        store.clone(),
        vec!["project/a".into()],
    )
    .unwrap();
    changed
        .embed(&ns, EmbeddingPurpose::Document, &["fixture".into()])
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 5);
    assert!(
        cache
            .embed(
                &Namespace::for_project("denied"),
                EmbeddingPurpose::Document,
                &["fixture".into()]
            )
            .await
            .is_err()
    );
    store.clear_embedding_cache().await.unwrap();
    cache
        .embed(&ns, EmbeddingPurpose::Document, &["fixture".into()])
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    store.rebuild_embedding_cache().await.unwrap();
    cache
        .embed(&ns, EmbeddingPurpose::Document, &["fixture".into()])
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 7);
}
#[tokio::test]
async fn corrupt_or_missing_cache_regenerates_with_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&dir.path().join("cache.sqlite"));
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let cache = CachedEmbeddingAdapter::new(
        inner(calls.clone(), "doc"),
        store.clone(),
        vec!["project/a".into()],
    )
    .unwrap();
    let ns = Namespace::for_project("a");
    cache
        .embed(&ns, EmbeddingPurpose::Document, &["fixture".into()])
        .await
        .unwrap();
    assert!(!cache.take_warnings().is_empty());
    store.rebuild_embedding_cache().await.unwrap();
    cache
        .embed(&ns, EmbeddingPurpose::Document, &["fixture".into()])
        .await
        .unwrap();
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    for corruption in [
        "UPDATE derived_embedding_cache SET vector=zeroblob(5000000)",
        "UPDATE derived_embedding_cache SET checksum=printf('%064d',0)||char(0)||zeroblob(5000000)",
        "UPDATE derived_embedding_cache SET vector='bad type'",
        "UPDATE derived_embedding_cache SET checksum='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'",
    ] {
        sqlx::query(corruption).execute(&pool).await.unwrap();
        let before = calls.load(Ordering::SeqCst);
        cache
            .embed(&ns, EmbeddingPurpose::Document, &["fixture".into()])
            .await
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), before + 1);
        assert!(
            cache
                .take_warnings()
                .iter()
                .any(|w| w.contains("invalid_or_unavailable"))
        );
    }
    pool.close().await;
}

#[tokio::test]
async fn reset_prevents_in_flight_old_generation_from_repopulating() {
    struct Paused {
        profile: EmbeddingProfile,
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }
    #[async_trait]
    impl EmbeddingPort for Paused {
        fn profile(&self) -> &EmbeddingProfile {
            &self.profile
        }
        async fn embed(
            &self,
            _: &Namespace,
            _: EmbeddingPurpose,
            _: &[String],
        ) -> Result<Vec<Vec<f32>>, AppError> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(vec![vec![1.0, 2.0]])
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let url =
        agent_llm_mm::support::sqlite_url::sqlite_file_url(&dir.path().join("generation.sqlite"));
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    store.rebuild_embedding_cache().await.unwrap();
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let profile = inner(Arc::new(AtomicUsize::new(0)), "doc")
        .profile()
        .clone();
    let paused = Arc::new(Paused {
        profile,
        started: started.clone(),
        release: release.clone(),
    });
    let cache = Arc::new(
        CachedEmbeddingAdapter::new(paused, store.clone(), vec!["project/a".into()]).unwrap(),
    );
    let worker = {
        let cache = cache.clone();
        tokio::spawn(async move {
            cache
                .embed(
                    &Namespace::for_project("a"),
                    EmbeddingPurpose::Document,
                    &["fixture".into()],
                )
                .await
                .unwrap()
        })
    };
    started.notified().await;
    store.rebuild_embedding_cache().await.unwrap();
    release.notify_one();
    worker.await.unwrap();
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM derived_embedding_cache")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        cache
            .take_warnings()
            .iter()
            .any(|w| w.contains("write_unavailable"))
    );
    pool.close().await;
}
