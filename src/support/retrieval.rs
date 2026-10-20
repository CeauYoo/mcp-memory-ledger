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
    if config.task_context_mode == TaskContextMode::Hybrid && !embedding.enabled {
        return Err(AppError::InvalidParams(
            "hybrid task context requires host-enabled local embedding".into(),
        ));
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
        Recall(EnhancedRecallRequest),
        Declare(crate::domain::recall_applicability::DeclareRecallApplicabilityRequest),
    }
    let request = match args {
        [_, operation] if operation == "cache-rebuild" => Request::CacheRebuild,
        [_, operation] if operation == "cache-clear" => Request::CacheClear,
        [_, flag, body] if flag == "--json" && body.len() <= 16384 => {
            Request::Recall(serde_json::from_str(body)?)
        }
        [_, operation, flag, body]
            if operation == "declare" && flag == "--json" && body.len() <= 32768 =>
        {
            Request::Declare(serde_json::from_str(body)?)
        }
        _ => anyhow::bail!("usage: retrieval [declare] --json '<bounded request>'"),
    };
    let config = crate::support::config::AppConfig::load().map_err(anyhow::Error::msg)?;
    config.validate().map_err(anyhow::Error::msg)?;
    let store = crate::adapters::sqlite::open_current_database(&config.database_url).await?;
    let result = match request {
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
