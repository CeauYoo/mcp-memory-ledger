//! Host-owned retrieval policy. Tool arguments cannot enable network embeddings.
use crate::{
    adapters::{
        local_embedding::{LocalEmbeddingConfig, OllamaEmbeddingAdapter},
        sqlite::SqliteStore,
    },
    application::{
        recall_memory_enhanced::{self, EnhancedRecallConfig, EnhancedRecallInput},
        task,
    },
    domain::task::{TaskContextRequest, TaskResumeContext},
    error::AppError,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskContextMode {
    #[default]
    Literal,
    Enhanced,
    Hybrid,
    Unified,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetrievalConfig {
    pub task_context_mode: TaskContextMode,
    pub optional_deadline_ms: u64,
    pub aliases:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, Vec<String>>>,
}
impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            task_context_mode: TaskContextMode::Literal,
            optional_deadline_ms: 2000,
            aliases: Default::default(),
        }
    }
}
impl RetrievalConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !(100..=5000).contains(&self.optional_deadline_ms) {
            return Err("retrieval.optional_deadline_ms must be 100..5000".into());
        }
        if self.aliases.len() > 128 {
            return Err("retrieval aliases exceed namespace limit".into());
        }
        Ok(())
    }
    pub fn enhanced(&self) -> EnhancedRecallConfig {
        EnhancedRecallConfig {
            aliases: self.aliases.clone(),
        }
    }
}

fn build_embedding(
    store: &SqliteStore,
    config: &LocalEmbeddingConfig,
) -> Result<std::sync::Arc<dyn crate::ports::embedding::EmbeddingPort>, AppError> {
    let adapter: std::sync::Arc<dyn crate::ports::embedding::EmbeddingPort> =
        std::sync::Arc::new(OllamaEmbeddingAdapter::new(config.clone())?);
    if config.cache_enabled {
        Ok(std::sync::Arc::new(
            crate::adapters::cached_embedding::CachedEmbeddingAdapter::new(
                adapter,
                store.clone(),
                config.namespace_allowlist.clone(),
            )?,
        ))
    } else {
        Ok(adapter)
    }
}

pub async fn resume(
    store: &SqliteStore,
    request: TaskContextRequest,
    config: &RetrievalConfig,
    embedding: &LocalEmbeddingConfig,
) -> Result<TaskResumeContext, AppError> {
    if matches!(
        config.task_context_mode,
        TaskContextMode::Hybrid | TaskContextMode::Unified
    ) && !embedding.enabled
    {
        return Err(AppError::InvalidParams(
            "hybrid task context requires host-enabled local embedding".into(),
        ));
    }
    let reported_environment = request.reported_environment.clone();
    if config.task_context_mode == TaskContextMode::Unified {
        return task::build_task_resume_context_with_selection(
            store,
            request,
            |input| async move {
                let index = crate::adapters::sqlite::semantic_index::SemanticIndex::new(
                    store.clone(),
                    embedding.clone(),
                    build_embedding(store, embedding)?,
                )?;
                super::unified_selection::execute(&index, input, &reported_environment).await
            },
            config.optional_deadline_ms,
            true,
        )
        .await;
    }
    if config.task_context_mode == TaskContextMode::Literal {
        return task::build_task_resume_context_with_recall(
            store,
            request,
            |input| async move {
                serde_json::to_value(
                    crate::application::build_task_context::execute(store, input).await?,
                )
                .map_err(|_| AppError::Message("literal context serialization failed".into()))
            },
            config.optional_deadline_ms,
        )
        .await;
    }
    let policy = config.enhanced();
    task::build_task_resume_context_with_recall(
        store,
        request,
        |input| async move {
            let adapter =
                if config.task_context_mode == TaskContextMode::Hybrid && embedding.enabled {
                    Some(build_embedding(store, embedding)?)
                } else {
                    None
                };
            let result = recall_memory_enhanced::execute_with_embedding(
                store,
                EnhancedRecallInput {
                    namespace: input.namespace,
                    query: input.query,
                    limit: input.limit,
                    max_bytes: input.max_bytes,
                    reported_environment,
                },
                &policy,
                adapter.as_deref(),
            )
            .await?;
            serde_json::to_value(result)
                .map_err(|_| AppError::Message("enhanced context serialization failed".into()))
        },
        config.optional_deadline_ms,
    )
    .await
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnhancedRecallRequest {
    pub namespace: String,
    pub query: String,
    pub limit: usize,
    pub max_bytes: usize,
    #[serde(default)]
    pub hybrid: bool,
    #[serde(default)]
    pub reported_environment: crate::domain::recall_applicability::EnvironmentFacts,
}
pub async fn recall(
    store: &SqliteStore,
    request: EnhancedRecallRequest,
    config: &RetrievalConfig,
    embedding: &LocalEmbeddingConfig,
) -> Result<serde_json::Value, AppError> {
    let namespace = crate::domain::types::Namespace::parse(request.namespace)?;
    let adapter = if request.hybrid {
        if !embedding.enabled {
            return Err(AppError::InvalidParams(
                "hybrid recall requires host-enabled local embedding".into(),
            ));
        }
        Some(build_embedding(store, embedding)?)
    } else {
        None
    };
    let policy = config.enhanced();
    let result = recall_memory_enhanced::execute_with_embedding(
        store,
        EnhancedRecallInput {
            namespace,
            query: request.query,
            limit: request.limit,
            max_bytes: request.max_bytes,
            reported_environment: request.reported_environment,
        },
        &policy,
        adapter.as_deref(),
    )
    .await?;
    serde_json::to_value(result)
        .map_err(|_| AppError::Message("recall serialization failed".into()))
}

pub async fn try_run_cli(args: &[String]) -> anyhow::Result<bool> {
    if args.first().map(String::as_str) != Some("retrieval") {
        return Ok(false);
    }
    enum Request {
        CacheRebuild,
        CacheClear,
        IndexReset,
        Semantic(String, SemanticIndexRequest),
        Recall(EnhancedRecallRequest),
        Declare(crate::domain::recall_applicability::DeclareRecallApplicabilityRequest),
    }
    let request = match args {
        [_, operation] if operation == "cache-rebuild" => Request::CacheRebuild,
        [_, operation] if operation == "cache-clear" => Request::CacheClear,
        [_, operation] if operation == "index-reset" => Request::IndexReset,
        [_, flag, body] if flag == "--json" && body.len() <= 16384 => {
            Request::Recall(serde_json::from_str(body)?)
        }
        [_, operation, flag, body]
            if operation == "declare" && flag == "--json" && body.len() <= 32768 =>
        {
            Request::Declare(serde_json::from_str(body)?)
        }
        [_, operation, flag, body]
            if [
                "index-start",
                "index-resume",
                "index-status",
                "index-search",
            ]
            .contains(&operation.as_str())
                && flag == "--json"
                && body.len() <= 16384 =>
        {
            Request::Semantic(operation.clone(), serde_json::from_str(body)?)
        }
        _ => anyhow::bail!(
            "usage: retrieval [declare|index-start|index-resume|index-status|index-search] --json '<bounded request>'"
        ),
    };
    let config = crate::support::config::AppConfig::load().map_err(anyhow::Error::msg)?;
    config.validate().map_err(anyhow::Error::msg)?;
    if matches!(&request, Request::IndexReset) {
        crate::adapters::sqlite::reset_semantic_index(&config.database_url).await?;
        println!(
            "{}",
            serde_json::json!({"semantic_index":"removed_all_namespaces", "authority":"unchanged"})
        );
        return Ok(true);
    }
    let store = crate::adapters::sqlite::open_current_database(&config.database_url).await?;
    let result = match request {
        Request::IndexReset => unreachable!("handled before opening indexed database"),
        Request::Semantic(operation, request) => {
            semantic_index_command(&store, &config.local_embedding, &operation, request).await?
        }
        Request::CacheRebuild => {
            store.rebuild_embedding_cache().await?;
            serde_json::json!({"derived_cache":"reset", "vectors":"regenerated_on_next_authorized_use"})
        }
        Request::CacheClear => {
            store.clear_embedding_cache().await?;
            serde_json::json!({"derived_cache":"cleared"})
        }
        Request::Recall(request) => {
            recall(&store, request, &config.retrieval, &config.local_embedding).await?
        }
        Request::Declare(request) => {
            serde_json::to_value(store.declare_recall_applicability(request).await?)?
        }
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(true)
}

/// Only operation parameters are accepted here; model, endpoint and namespace
/// permissions always come from host configuration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SemanticIndexRequest {
    namespace: String,
    generation: Option<String>,
    batch_size: Option<usize>,
    query: Option<String>,
    cursor: Option<crate::adapters::sqlite::semantic_index::SemanticSearchCursor>,
    page_size: Option<usize>,
}
async fn semantic_index_command(
    store: &SqliteStore,
    config: &LocalEmbeddingConfig,
    operation: &str,
    request: SemanticIndexRequest,
) -> Result<serde_json::Value, AppError> {
    use crate::adapters::sqlite::semantic_index::SemanticIndex;
    let namespace = crate::domain::types::Namespace::parse(request.namespace)?;
    let index = SemanticIndex::new(
        store.clone(),
        config.clone(),
        build_embedding(store, config)?,
    )?;
    let value = match operation {
        "index-start" => serde_json::to_value(index.start(&namespace).await?),
        "index-status" => serde_json::to_value(index.status(&namespace).await?),
        "index-resume" => serde_json::to_value(
            index
                .resume(
                    &namespace,
                    request
                        .generation
                        .as_deref()
                        .ok_or_else(|| AppError::InvalidParams("generation required".into()))?,
                    request.batch_size.unwrap_or(4),
                )
                .await?,
        ),
        "index-search" => serde_json::to_value(
            index
                .search_page(
                    &namespace,
                    request
                        .generation
                        .as_deref()
                        .ok_or_else(|| AppError::InvalidParams("generation required".into()))?,
                    request
                        .query
                        .as_deref()
                        .ok_or_else(|| AppError::InvalidParams("query required".into()))?,
                    request.cursor,
                    request.page_size.unwrap_or(32),
                )
                .await?,
        ),
        _ => {
            return Err(AppError::InvalidParams(
                "unknown semantic index operation".into(),
            ));
        }
    };
    value.map_err(|_| AppError::Message("semantic index result serialization failed".into()))
}
