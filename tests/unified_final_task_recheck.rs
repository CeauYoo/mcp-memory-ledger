//! Deterministic assembly-order regression with synthetic local data only.
//! Owner attestation here is local operator evidence, not authenticated identity
//! or execution authority. No runtime hooks, sockets, or provider calls occur.
use agent_llm_mm::{
    adapters::{
        local_embedding::LocalEmbeddingConfig,
        sqlite::{SqliteStore, semantic_index::SemanticIndex},
    },
    application::{
        search_memory::SearchMemoryRecord,
        task::{self, OptionalTaskSelection},
    },
    domain::{
        experience::*,
        experience_use::*,
        owner_attestation::{OwnerAttestationRequest, OwnerRequirementChange},
        task::*,
        types::{MemoryScope, Namespace},
    },
    error::AppError,
    ports::{
        embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
        enhanced_memory_store::{EnhancedMemoryStore, EnhancedRecordPage},
        experience_store::ExperienceStore,
        experience_use_store::ExperienceUseStore,
        memory_read_store::*,
        task_store::TaskStore,
        text_memory_store::*,
    },
};
use async_trait::async_trait;
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

struct MutatingStore {
    inner: SqliteStore,
    mutated: AtomicBool,
    mutate_at_use_read: bool,
}

// Delegate required store APIs rather than duplicating their persistence behavior.
macro_rules! delegate {
    ($trait:ident { $( $name:ident($arg:ident: $ty:ty) -> $result:ty; )* }) => {
        #[async_trait]
        impl $trait for MutatingStore {
            $(async fn $name(&self, $arg: $ty) -> $result {
                self.inner.$name($arg).await
            })*
        }
    };
}
delegate!(MemoryReadStore {
    query_event_records(q: EventRecordQuery) -> Result<Vec<EventReadRecord>, AppError>;
    query_claim_records(q: ClaimRecordQuery) -> Result<Vec<ClaimReadRecord>, AppError>;
    query_episode_records(q: EpisodeRecordQuery) -> Result<Vec<EpisodeReadRecord>, AppError>;
    query_reflection_records(q: ReflectionRecordQuery) -> Result<Vec<ReflectionReadRecord>, AppError>;
    query_scoped_event_ids(q: ScopedEventIdQuery) -> Result<BTreeSet<String>, AppError>;
    query_claim_reflection_history(q: ClaimReflectionHistoryQuery) -> Result<ClaimReflectionHistoryPage, AppError>;
    query_self_model_history(q: SelfModelHistoryQuery) -> Result<SelfModelHistoryPage, AppError>;
});
delegate!(TextMemoryStore {
    recall_text(q: TextMemoryQuery) -> Result<TextMemoryPage, AppError>;
});
delegate!(ExperienceStore {
    create_episode(r: CreateEpisodeRequest) -> Result<ExperienceWriteResult<PersistedEpisode>, AppError>;
    get_episode(r: GetEpisodeRequest) -> Result<Option<PersistedEpisode>, AppError>;
    list_episodes(r: ListEpisodesRequest) -> Result<Vec<PersistedEpisode>, AppError>;
    create_candidate(r: CreateCandidateRequest) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    get_candidate(r: GetCandidateRequest) -> Result<Option<ExperienceCandidate>, AppError>;
    list_candidates(r: ListCandidatesRequest) -> Result<Vec<ExperienceCandidate>, AppError>;
    update_candidate_status(r: UpdateCandidateStatusRequest) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    revise_candidate(r: ReviseCandidateRequest) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    rollback_candidate(r: RollbackCandidateRequest) -> Result<ExperienceWriteResult<ExperienceCandidate>, AppError>;
    recall_candidates(r: RecallCandidatesRequest) -> Result<ExperienceRecall, AppError>;
});
#[async_trait]
impl ExperienceUseStore for MutatingStore {
    async fn prepare_experience_derivation(
        &self,
        r: DeriveExperienceRequest,
    ) -> Result<ExperienceWriteResult<DerivationReservation>, AppError> {
        self.inner.prepare_experience_derivation(r).await
    }
    async fn record_experience_use(
        &self,
        r: RecordExperienceUseRequest,
    ) -> Result<ExperienceWriteResult<ExperienceUse>, AppError> {
        self.inner.record_experience_use(r).await
    }
    async fn list_experience_uses(
        &self,
        r: ListExperienceUsesRequest,
    ) -> Result<Vec<ExperienceUseView>, AppError> {
        let result = self.inner.list_experience_uses(r).await?;
        if self.mutate_at_use_read {
            self.append_owner_prohibition().await?;
        }
        Ok(result)
    }
}
impl MutatingStore {
    async fn append_owner_prohibition(&self) -> Result<(), AppError> {
        assert!(
            !self.mutated.swap(true, Ordering::SeqCst),
            "one bounded mutation expected"
        );
        self.inner
            .attest_task_owner(OwnerAttestationRequest {
                namespace: "project/a".into(),
                task_id: "t".into(),
                import_event_id: "late-owner-prohibition".into(),
                expected_version: 1,
                evidence_text: "Do not deploy".into(),
                change: OwnerRequirementChange::Append {
                    requirement_id: "owner-prohibition".into(),
                    kind: RequirementKind::Prohibition,
                    exact_text: "Do not deploy".into(),
                },
            })
            .await?;
        Ok(())
    }
}
#[async_trait]
impl TaskStore for MutatingStore {
    async fn create_task(
        &self,
        r: CreateTaskRequest,
    ) -> Result<TaskWriteResult<TaskRevision>, AppError> {
        self.inner.create_task(r).await
    }
    async fn revise_task(
        &self,
        r: ReviseTaskRequest,
    ) -> Result<TaskWriteResult<TaskRevision>, AppError> {
        self.inner.revise_task(r).await
    }
    async fn get_task(&self, r: GetTaskRequest) -> Result<Option<TaskRevision>, AppError> {
        self.inner.get_task(r).await
    }
    async fn list_tasks(&self, r: ListTasksRequest) -> Result<Vec<TaskRevision>, AppError> {
        self.inner.list_tasks(r).await
    }
    async fn record_task_checkpoint(
        &self,
        r: RecordTaskCheckpointRequest,
    ) -> Result<TaskWriteResult<TaskCheckpoint>, AppError> {
        self.inner.record_task_checkpoint(r).await
    }
    async fn get_task_snapshot(
        &self,
        namespace: &str,
        task_id: &str,
    ) -> Result<Option<TaskSnapshot>, AppError> {
        self.inner.get_task_snapshot(namespace, task_id).await
    }
}
#[async_trait]
impl EnhancedMemoryStore for MutatingStore {
    async fn scan_enhanced_records(
        &self,
        scope: MemoryScope,
        limit: usize,
    ) -> Result<EnhancedRecordPage, AppError> {
        self.inner.scan_enhanced_records(scope, limit).await
    }
    async fn hydrate_enhanced_record(
        &self,
        scope: MemoryScope,
        reference: &str,
    ) -> Result<Option<SearchMemoryRecord>, AppError> {
        self.inner.hydrate_enhanced_record(scope, reference).await
    }
    async fn selection_source_generation(&self, scope: MemoryScope) -> Result<String, AppError> {
        let before = self
            .inner
            .selection_source_generation(scope.clone())
            .await?;
        assert!(
            !self.mutate_at_use_read,
            "low-budget test must skip optional source reads"
        );
        self.append_owner_prohibition().await?;
        let after = self.inner.selection_source_generation(scope).await?;
        assert_eq!(
            before, after,
            "task authority is independent of semantic source generation"
        );
        Ok(after)
    }
}
struct NeverEmbed(EmbeddingProfile);
#[async_trait]
impl EmbeddingPort for NeverEmbed {
    fn profile(&self) -> &EmbeddingProfile {
        &self.0
    }
    async fn embed(
        &self,
        _: &Namespace,
        _: EmbeddingPurpose,
        _: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        panic!("assembly ordering regression must not invoke embedding")
    }
}
#[tokio::test]
async fn owner_prohibition_added_during_last_source_read_invalidates_resumability() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&root.join("test.sqlite"));
    let inner = SqliteStore::bootstrap(&url).await.unwrap();
    task::create_task(&inner, serde_json::from_value(json!({
        "namespace":"project/a", "task_id":"t", "request_id":"create", "title":"Synthetic task", "reason":"fixture",
        "source":{"kind":"caller_reported"},
        "requirements":[{"requirement_id":"initial", "kind":"requirement", "text":"Keep initial requirement", "state":"reported", "source":{"kind":"caller_reported"}}]
    })).unwrap()).await.unwrap();
    let profile = EmbeddingProfile {
        model: "synthetic".into(),
        model_digest: "a".repeat(64),
        dimensions: 2,
        ..Default::default()
    };
    let config = LocalEmbeddingConfig {
        enabled: true,
        local_only_declared: true,
        namespace_allowlist: vec!["project/a".into()],
        profile: profile.clone(),
        ..Default::default()
    };
    let index = SemanticIndex::new(inner.clone(), config, Arc::new(NeverEmbed(profile))).unwrap();
    let generation = index
        .start(&Namespace::for_project("a"))
        .await
        .unwrap()
        .source_generation;
    let store = MutatingStore {
        inner,
        mutated: AtomicBool::new(false),
        mutate_at_use_read: false,
    };
    let context = task::build_task_resume_context_with_selection(
        &store,
        TaskContextRequest {
            namespace: "project/a".into(),
            task_id: "t".into(),
            max_bytes: 32768,
            reported_environment: Default::default(),
        },
        |_| async move {
            // Synthetic optional bundle exercises assembly, not semantic ranking.
            Ok(OptionalTaskSelection {
                related_memory: Some(json!([])),
                experiences: vec![],
                episodes: vec![],
                report: Some(json!({"source_generation":generation, "selected":[]})),
            })
        },
        5000,
        true,
    )
    .await
    .unwrap();
    assert!(store.mutated.load(Ordering::SeqCst));
    assert_eq!(context.task_version, 1);
    assert_eq!(context.task_version_after_selection, Some(2));
    assert!(!context.can_resume);
    assert!(!context.complete);
    assert!(context.mandatory_snapshot_consistent);
    assert!(!context.snapshot_consistent);
    assert_eq!(
        context.mandatory_requirements.len(),
        1,
        "retain the coherent earlier snapshot, do not silently mix versions"
    );
    assert!(context.owner_provenance.is_empty());
    assert!(
        context
            .diagnostics
            .iter()
            .any(|d| d == "stale_task_or_checkpoint_changed_during_selection_rebuild_context")
    );
    let current = store
        .inner
        .get_task_snapshot("project/a", "t")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.task.owner_provenance.len(), 1);
    assert_eq!(current.task.owner_provenance[0].exact_text, "Do not deploy");
}

#[tokio::test]
async fn owner_prohibition_during_use_read_is_rechecked_when_optional_budget_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let url = agent_llm_mm::support::sqlite_url::sqlite_file_url(&root.join("test.sqlite"));
    let inner = SqliteStore::bootstrap(&url).await.unwrap();
    task::create_task(&inner, serde_json::from_value(json!({
        "namespace":"project/a", "task_id":"t", "request_id":"create", "title":"Synthetic task", "reason":"fixture",
        "source":{"kind":"caller_reported"},
        "requirements":[{"requirement_id":"initial", "kind":"requirement", "text":"Keep initial requirement", "state":"reported", "source":{"kind":"caller_reported"}}]
    })).unwrap()).await.unwrap();
    let request = |max_bytes| TaskContextRequest {
        namespace: "project/a".into(),
        task_id: "t".into(),
        max_bytes,
        reported_environment: Default::default(),
    };
    // Measure a complete context instead of depending on fixed serialization sizes.
    // 256 bytes cover new stale/skip diagnostics but not the 1024-byte optional reserve.
    let baseline = task::build_task_resume_context_with_selection(
        &inner,
        request(32768),
        |_| async {
            Ok(OptionalTaskSelection {
                related_memory: None,
                experiences: vec![],
                episodes: vec![],
                report: None,
            })
        },
        5000,
        true,
    )
    .await
    .unwrap();
    assert!(baseline.can_resume);
    let budget = baseline.serialized_bytes + 256;
    let store = MutatingStore {
        inner,
        mutated: AtomicBool::new(false),
        mutate_at_use_read: true,
    };
    let context = task::build_task_resume_context_with_selection(
        &store,
        request(budget),
        |_| async { panic!("low budget must skip optional selection") },
        5000,
        true,
    )
    .await
    .unwrap();
    assert!(store.mutated.load(Ordering::SeqCst));
    assert_eq!(context.task_version, 1);
    assert_eq!(context.task_version_after_selection, Some(2));
    assert!(!context.can_resume);
    assert!(!context.complete);
    // This describes only the retained original mandatory read snapshot, not
    // cross-store atomicity or freshness at the time a caller acts on the output.
    assert!(context.mandatory_snapshot_consistent);
    assert_eq!(context.mandatory_requirements.len(), 1);
    assert!(context.owner_provenance.is_empty());
    assert!(context.serialized_bytes <= budget);
    assert!(
        context
            .diagnostics
            .iter()
            .any(|d| d == "optional_recall_skipped_for_required_task_budget")
    );
    assert!(
        context
            .diagnostics
            .iter()
            .any(|d| d == "stale_task_or_checkpoint_changed_during_selection_rebuild_context")
    );
}
