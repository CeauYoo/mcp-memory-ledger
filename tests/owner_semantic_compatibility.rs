//! Cross-slice, synthetic compatibility: no network, user data or runtime hooks.
use agent_llm_mm::{
    adapters::{
        local_embedding::LocalEmbeddingConfig,
        sqlite::{
            SqliteStore, migrate_database, open_current_database, reset_semantic_index,
            semantic_index::SemanticIndex,
        },
    },
    application::task,
    domain::{
        feedback::FeedbackSourceKind,
        owner_attestation::{OwnerAttestationRequest, OwnerRequirementChange},
        task::{
            CreateTaskRequest, GetTaskRequest, RequirementChange, RequirementKind,
            RequirementState, ReviseTaskRequest, TaskRequirement, TaskSource,
        },
        types::Namespace,
    },
    error::AppError,
    ports::embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
};
use async_trait::async_trait;
use std::sync::Arc;
struct Fake(EmbeddingProfile);
#[async_trait]
impl EmbeddingPort for Fake {
    fn profile(&self) -> &EmbeddingProfile {
        &self.0
    }
    async fn embed(
        &self,
        _: &Namespace,
        _: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
}
fn source() -> TaskSource {
    TaskSource {
        kind: FeedbackSourceKind::CallerReported,
        reference: None,
    }
}
#[tokio::test]
async fn schema11_migration_backup_and_derived_reset_preserve_owner_authority() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let path = root.join("combined.sqlite");
    let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&path);
    let store = SqliteStore::bootstrap(&url).await.unwrap();
    let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query("INSERT INTO events(event_id,recorded_at,owner,namespace,kind,summary) VALUES('fact','2026-01-01T00:00:00Z','world','project/a','observation','synthetic fact')").execute(&pool).await.unwrap();
    let config = LocalEmbeddingConfig {
        enabled: true,
        local_only_declared: true,
        namespace_allowlist: vec!["project/a".into()],
        profile: EmbeddingProfile {
            model: "synthetic".into(),
            model_digest: "a".repeat(64),
            dimensions: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    let index = SemanticIndex::new(
        store,
        config.clone(),
        Arc::new(Fake(config.profile.clone())),
    )
    .unwrap();
    let ns = Namespace::for_project("a");
    let started = index.start(&ns).await.unwrap();
    assert!(
        index
            .resume(&ns, &started.generation, 4)
            .await
            .unwrap()
            .complete
    );
    // Model an existing schema10 database with its valid optional index installed.
    sqlx::raw_sql("DROP TABLE task_owner_provenance; DROP TABLE task_owner_events; DELETE FROM schema_migrations WHERE version=11; PRAGMA user_version=10;").execute(&pool).await.unwrap();
    assert!(open_current_database(&url).await.is_err());
    assert!(migrate_database(&url).await.unwrap().is_current());
    let store = open_current_database(&url).await.unwrap();
    let reopened = SemanticIndex::new(
        store.clone(),
        config.clone(),
        Arc::new(Fake(config.profile.clone())),
    )
    .unwrap();
    assert!(reopened.status(&ns).await.unwrap().complete);
    task::create_task(
        &store,
        CreateTaskRequest {
            namespace: "project/a".into(),
            task_id: "t".into(),
            request_id: "create".into(),
            title: "synthetic task".into(),
            requirements: vec![TaskRequirement {
                requirement_id: "base".into(),
                kind: RequirementKind::Requirement,
                text: "fixture".into(),
                state: RequirementState::Reported,
                source: source(),
                replaces: None,
            }],
            source: source(),
            reason: "fixture".into(),
        },
    )
    .await
    .unwrap();
    store
        .attest_task_owner(OwnerAttestationRequest {
            namespace: "project/a".into(),
            task_id: "t".into(),
            import_event_id: "owner-compat".into(),
            expected_version: 1,
            evidence_text: "Do not deploy".into(),
            change: OwnerRequirementChange::Append {
                requirement_id: "protected".into(),
                kind: RequirementKind::Prohibition,
                exact_text: "Do not deploy".into(),
            },
        })
        .await
        .unwrap();
    let backup = root.join("backup.sqlite");
    let restored = root.join("restored.sqlite");
    agent_llm_mm::support::database_backup::transfer_database("backup", &path, &backup)
        .await
        .unwrap();
    agent_llm_mm::support::database_backup::transfer_database("restore", &backup, &restored)
        .await
        .unwrap();
    let restored_url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&restored);
    let restored_store = open_current_database(&restored_url).await.unwrap();
    let restored_index = SemanticIndex::new(
        restored_store.clone(),
        config.clone(),
        Arc::new(Fake(config.profile)),
    )
    .unwrap();
    assert_eq!(
        restored_index.status(&ns).await.unwrap().generation,
        started.generation
    );
    reset_semantic_index(&restored_url).await.unwrap();
    let task = task::get_task(
        &restored_store,
        GetTaskRequest {
            namespace: "project/a".into(),
            task_id: "t".into(),
            version: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(task.owner_provenance.len(), 1);
    assert_eq!(task.owner_provenance[0].exact_text, "Do not deploy");
    assert!(
        task::revise_task(
            &restored_store,
            ReviseTaskRequest {
                namespace: "project/a".into(),
                task_id: "t".into(),
                request_id: "forbidden".into(),
                expected_version: 2,
                changes: vec![RequirementChange::Withdraw {
                    requirement_id: "protected".into()
                }],
                status: None,
                source: source(),
                reason: "ordinary cannot remove imported requirement".into()
            }
        )
        .await
        .is_err()
    );
}
