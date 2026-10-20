//! Host policy and mandatory-context protection; synthetic local fixtures only.
use agent_llm_mm::{
    adapters::{local_embedding::LocalEmbeddingConfig, sqlite::SqliteStore},
    application::task,
    domain::{
        event::Event,
        task::TaskContextRequest,
        types::{EventKind, Namespace, Owner},
    },
    ports::{EventStore, StoredEvent},
    support::{
        config::AppConfig,
        retrieval::{self, EnhancedRecallRequest, RetrievalConfig, TaskContextMode},
    },
};
use serde_json::json;

async fn fixture() -> (tempfile::TempDir, SqliteStore) {
    let dir = tempfile::tempdir().unwrap();
    let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&dir.path().join("resume.sqlite"));
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    task::create_task(&store, serde_json::from_value(json!({
        "namespace":"project/resume", "task_id":"resume", "request_id":"create-resume",
        "title":"数据库迁移", "reason":"synthetic integration fixture",
        "source":{"kind":"caller_reported"},
        "requirements":[{"requirement_id":"hard", "kind":"prohibition", "text":"未经批准不得部署", "state":"reported", "source":{"kind":"caller_reported"}}]
    })).unwrap()).await.unwrap();
    store
        .append_event(StoredEvent::new(
            "database-event".into(),
            chrono::Utc::now(),
            Event::new_with_namespace(
                Owner::World,
                Namespace::for_project("resume"),
                EventKind::Observation,
                "数据库",
            )
            .unwrap(),
        ))
        .await
        .unwrap();
    (dir, store)
}
fn request() -> TaskContextRequest {
    TaskContextRequest {
        namespace: "project/resume".into(),
        task_id: "resume".into(),
        max_bytes: 32768,
    }
}
fn recall_request(hybrid: bool) -> EnhancedRecallRequest {
    EnhancedRecallRequest {
        namespace: "project/resume".into(),
        query: "数据库迁移".into(),
        limit: 8,
        max_bytes: 8192,
        hybrid,
    }
}

#[tokio::test]
async fn default_literal_and_host_configured_enhanced_chinese_resume() {
    let (_dir, store) = fixture().await;
    let default = AppConfig::default();
    assert_eq!(
        default.retrieval.task_context_mode,
        TaskContextMode::Literal
    );
    assert!(!default.local_embedding.enabled);
    let literal = retrieval::resume(
        &store,
        request(),
        &default.retrieval,
        &default.local_embedding,
    )
    .await
    .unwrap();
    assert_eq!(literal.mandatory_requirement_count, 1);
    assert!(
        literal
            .related_memory
            .as_ref()
            .unwrap()
            .get("strategy")
            .is_none()
    );
    let config = RetrievalConfig {
        task_context_mode: TaskContextMode::Enhanced,
        optional_deadline_ms: 5000,
        ..Default::default()
    };
    let enhanced = retrieval::resume(&store, request(), &config, &default.local_embedding)
        .await
        .unwrap();
    assert_eq!(
        enhanced.mandatory_requirements,
        literal.mandatory_requirements
    );
    let memory = enhanced.related_memory.unwrap();
    assert_eq!(memory["strategy"], "scoped_lexical_rrf_v1");
    assert!(
        memory["tokens"]
            .as_array()
            .unwrap()
            .contains(&json!("数据库"))
    );
    assert!(
        memory["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["reference"] == "event:database-event")
    );
}

#[tokio::test]
async fn hybrid_tool_request_requires_host_enabled_embeddings() {
    let (_dir, store) = fixture().await;
    let error = retrieval::recall(
        &store,
        recall_request(true),
        &RetrievalConfig::default(),
        &LocalEmbeddingConfig::default(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("host-enabled local embedding"));
}

#[test]
fn mcp_payload_cannot_override_host_policy_or_endpoint() {
    for field in [
        "task_context_mode",
        "optional_deadline_ms",
        "local_embedding",
        "endpoint",
        "namespace_allowlist",
        "aliases",
    ] {
        let mut payload = json!({"namespace":"project/resume","task_id":"resume","max_bytes":8192});
        payload[field] = json!("override");
        assert!(
            serde_json::from_value::<TaskContextRequest>(payload).is_err(),
            "{field}"
        );
        let mut payload =
            json!({"namespace":"project/resume","query":"数据库","limit":8,"max_bytes":8192});
        payload[field] = json!("override");
        assert!(
            serde_json::from_value::<EnhancedRecallRequest>(payload).is_err(),
            "{field}"
        );
    }
}

#[tokio::test]
async fn optional_timeout_preserves_all_mandatory_requirements() {
    let (_dir, store) = fixture().await;
    let started = std::time::Instant::now();
    let result = task::build_task_resume_context_with_recall(
        &store,
        request(),
        |_| async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            Ok(json!({"unexpected":"slow optional result"}))
        },
        100,
    )
    .await
    .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(3));
    assert_eq!(result.mandatory_requirement_count, 1);
    assert_eq!(result.mandatory_requirements[0].text, "未经批准不得部署");
    assert!(result.mandatory_snapshot_consistent);
    assert!(result.related_memory.is_none());
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d == "optional_recall_deadline_exceeded_mandatory_retained")
    );
    assert!(result.serialized_bytes <= result.max_bytes);
}

#[test]
fn retrieval_configuration_loads_and_checks_deadline_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.toml");
    std::fs::write(&path,"[retrieval]\ntask_context_mode = \"enhanced\"\noptional_deadline_ms = 3000\n[retrieval.aliases.\"project/resume\"]\n\"数据库\" = [\"存储\"]\n").unwrap();
    let config = AppConfig::load_from_path(&path).unwrap();
    assert_eq!(
        config.retrieval.task_context_mode,
        TaskContextMode::Enhanced
    );
    assert_eq!(config.retrieval.optional_deadline_ms, 3000);
    assert_eq!(
        config.retrieval.aliases["project/resume"]["数据库"],
        vec!["存储"]
    );
    for deadline in [0, 99, 5001, u64::MAX] {
        assert!(
            RetrievalConfig {
                optional_deadline_ms: deadline,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    for deadline in [100, 2000, 5000] {
        assert!(
            RetrievalConfig {
                optional_deadline_ms: deadline,
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }
    std::fs::write(&path, "[retrieval]\ntask_context_mode = \"remote\"\n").unwrap();
    assert!(AppConfig::load_from_path(&path).is_err());
}
