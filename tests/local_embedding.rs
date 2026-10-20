//! Synthetic loopback HTTP only. Never launches or contacts a real model.
use agent_llm_mm::{
    adapters::local_embedding::{LocalEmbeddingConfig, OllamaEmbeddingAdapter},
    domain::types::Namespace,
    ports::embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
};
use axum::{
    Json, Router,
    routing::{get, post},
};
use serde_json::{Value, json};
fn config(endpoint: String) -> LocalEmbeddingConfig {
    LocalEmbeddingConfig {
        enabled: true,
        cache_enabled: false,
        local_only_declared: true,
        namespace_allowlist: vec!["project/fixture".into()],
        endpoint,
        timeout_ms: 500,
        profile: EmbeddingProfile {
            model: "fixture:latest".into(),
            model_digest: "a".repeat(64),
            dimensions: 3,
            document_prefix: "doc: ".into(),
            query_prefix: "query: ".into(),
        },
    }
}
async fn fixture(mode: &'static str) -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route("/api/tags", get(move || async move {
        Json(json!({"models":[{"name":"fixture:latest", "digest": if mode == "digest" { "b".repeat(64) } else { "a".repeat(64) }}]}))
    })).route("/api/embed", post(move |Json(body): Json<Value>| async move {
        assert_eq!(body["model"], "fixture:latest");
        assert_eq!(body["truncate"], false);
        assert_eq!(body["dimensions"], 3);
        assert!(body["input"][0].as_str().unwrap().starts_with("query: "));
        if mode == "timeout" { tokio::time::sleep(std::time::Duration::from_millis(150)).await; }
        let vector = if mode == "dimension" { json!([1.0,2.0]) } else if mode == "zero" { json!([0.0,0.0,0.0]) } else { json!([0.25,0.5,0.75]) };
        Json(json!({"model":"fixture:latest", "embeddings":[vector]}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (endpoint, handle)
}
#[test]
fn disabled_and_remote_endpoints_are_rejected() {
    assert!(!LocalEmbeddingConfig::default().enabled);
    assert!(OllamaEmbeddingAdapter::new(LocalEmbeddingConfig::default()).is_err());
    for endpoint in [
        "http://localhost:11434",
        "http://2130706433",
        "http://127.1",
        "http://0177.0.0.1",
        "http://example.com",
        "http://10.0.0.1",
        "http://127.0.0.1/path",
        "http://user:password@127.0.0.1",
        "http://127.0.0.1/?secret=x",
        "http://127.0.0.1/#x",
    ] {
        assert!(
            OllamaEmbeddingAdapter::new(config(endpoint.into())).is_err(),
            "{endpoint}"
        );
    }
    let mut c = config("http://127.0.0.1:1".into());
    c.local_only_declared = false;
    assert!(OllamaEmbeddingAdapter::new(c).is_err());
}
#[tokio::test]
async fn returns_actual_fixture_vectors_and_blocks_other_namespaces() {
    let (endpoint, server) = fixture("ok").await;
    let adapter = OllamaEmbeddingAdapter::new(config(endpoint)).unwrap();
    let vectors = adapter
        .embed(
            &Namespace::for_project("fixture"),
            EmbeddingPurpose::Query,
            &["synthetic".into()],
        )
        .await
        .unwrap();
    assert_eq!(vectors, vec![vec![0.25, 0.5, 0.75]]);
    assert!(
        adapter
            .embed(
                &Namespace::for_project("other"),
                EmbeddingPurpose::Query,
                &["never sent".into()]
            )
            .await
            .is_err()
    );
    assert!(
        adapter
            .embed(
                &Namespace::for_project("fixture"),
                EmbeddingPurpose::Query,
                &["x".repeat(256 * 1024)]
            )
            .await
            .is_err()
    );
    server.abort();
}
#[tokio::test]
async fn rejects_digest_dimension_zero_and_total_timeout() {
    for mode in ["digest", "dimension", "zero", "timeout"] {
        let (endpoint, server) = fixture(mode).await;
        let mut c = config(endpoint);
        if mode == "timeout" {
            c.timeout_ms = 30;
        }
        let adapter = OllamaEmbeddingAdapter::new(c).unwrap();
        assert!(
            adapter
                .embed(
                    &Namespace::for_project("fixture"),
                    EmbeddingPurpose::Query,
                    &["synthetic".into()]
                )
                .await
                .is_err(),
            "{mode}"
        );
        server.abort();
    }
}
#[tokio::test]
async fn rejects_redirect_without_following() {
    let app = Router::new().route(
        "/api/tags",
        get(|| async { axum::response::Redirect::temporary("http://127.0.0.1:1/forbidden") }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let adapter = OllamaEmbeddingAdapter::new(config(endpoint)).unwrap();
    let error = adapter
        .embed(
            &Namespace::for_project("fixture"),
            EmbeddingPurpose::Query,
            &["synthetic".into()],
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("non-success"));
    server.abort();
}

#[tokio::test]
async fn rejects_identity_drift_after_embedding_and_oversized_responses() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    for oversized in [false, true] {
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/api/tags", get(move || {
                let calls = calls.clone();
                async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    Json(json!({"models":[{"name":"fixture:latest","digest":if n == 0 {"a".repeat(64)} else {"b".repeat(64)}}]}))
                }
            }))
            .route("/api/embed", post(move || async move {
                if oversized {
                    "x".repeat(4 * 1024 * 1024 + 1)
                } else {
                    json!({"model":"fixture:latest","embeddings":[[1.0,2.0,3.0]]}).to_string()
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let adapter = OllamaEmbeddingAdapter::new(config(endpoint)).unwrap();
        let error = adapter
            .embed(
                &Namespace::for_project("fixture"),
                EmbeddingPurpose::Query,
                &["synthetic".into()],
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains(if oversized {
            "response exceeds"
        } else {
            "digest mismatch"
        }));
        server.abort();
    }
}
