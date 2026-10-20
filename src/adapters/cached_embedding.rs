//! Optional derived-vector cache; an uncached adapter remains fully functional.
use crate::{
    adapters::sqlite::SqliteStore,
    domain::types::Namespace,
    error::AppError,
    ports::embedding::{EmbeddingPort, EmbeddingProfile, EmbeddingPurpose},
};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
pub struct CachedEmbeddingAdapter {
    inner: Arc<dyn EmbeddingPort>,
    store: SqliteStore,
    namespace_allowlist: Vec<String>,
    warnings: Mutex<Vec<String>>,
}
impl CachedEmbeddingAdapter {
    pub fn new(
        inner: Arc<dyn EmbeddingPort>,
        store: SqliteStore,
        namespace_allowlist: Vec<String>,
    ) -> Result<Self, AppError> {
        if namespace_allowlist.is_empty()
            || namespace_allowlist
                .iter()
                .any(|n| Namespace::parse(n.clone()).is_err())
        {
            return Err(AppError::InvalidParams(
                "embedding cache requires exact namespace allowlist".into(),
            ));
        }
        Ok(Self {
            inner,
            store,
            namespace_allowlist,
            warnings: Mutex::new(vec![]),
        })
    }
    fn warning(&self, message: &str) {
        if let Ok(mut warnings) = self.warnings.lock()
            && warnings.len() < 8
            && !warnings.iter().any(|w| w == message)
        {
            warnings.push(message.into());
        }
    }
}
#[async_trait]
impl EmbeddingPort for CachedEmbeddingAdapter {
    fn profile(&self) -> &EmbeddingProfile {
        self.inner.profile()
    }
    fn take_warnings(&self) -> Vec<String> {
        let mut result = self
            .warnings
            .lock()
            .map(|mut w| std::mem::take(&mut *w))
            .unwrap_or_default();
        result.extend(self.inner.take_warnings());
        result
    }
    async fn embed(
        &self,
        namespace: &Namespace,
        purpose: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError> {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            if !self
                .namespace_allowlist
                .iter()
                .any(|n| n == namespace.as_str())
            {
                return Err(AppError::InvalidParams(
                    "embedding cache namespace not allowed".into(),
                ));
            }
            let prefix = match purpose {
                EmbeddingPurpose::Document => &self.profile().document_prefix,
                EmbeddingPurpose::Query => &self.profile().query_prefix,
            };
            let bytes = texts.iter().try_fold(0usize, |n, t| {
                n.checked_add(t.len())?.checked_add(prefix.len())
            });
            if texts.is_empty() || texts.len() > 64 || bytes.is_none_or(|n| n > 256 * 1024) {
                return Err(AppError::InvalidParams(
                    "embedding cache input budget exceeded".into(),
                ));
            }
            let profile = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(self.profile()).map_err(|_| {
                    AppError::Message("embedding profile serialization failed".into())
                })?)
            );
            let purpose_key = match purpose {
                EmbeddingPurpose::Document => "document",
                EmbeddingPurpose::Query => "query",
            };
            let digests: Vec<String> = texts
                .iter()
                .map(|text| format!("{:x}", Sha256::digest(text.as_bytes())))
                .collect();
            let generation = self.store.embedding_cache_generation().await.ok();
            match self
                .store
                .load_cached_embeddings(
                    namespace.as_str(),
                    &profile,
                    purpose_key,
                    &digests,
                    self.profile().dimensions,
                    generation.as_deref().unwrap_or(""),
                )
                .await
            {
                Ok(Some(vectors)) => return Ok(vectors),
                Ok(None) => {}
                Err(_) => {
                    self.warning("derived_embedding_cache_invalid_or_unavailable_regenerated")
                }
            }
            let vectors = self.inner.embed(namespace, purpose, texts).await?;
            if let Some(generation) = generation {
                if self
                    .store
                    .store_cached_embeddings(
                        namespace.as_str(),
                        &profile,
                        purpose_key,
                        &digests,
                        &vectors,
                        &generation,
                    )
                    .await
                    .is_err()
                {
                    self.warning("derived_embedding_cache_write_unavailable_vectors_not_cached");
                }
            } else {
                self.warning("derived_embedding_cache_write_unavailable_vectors_not_cached");
            }
            Ok(vectors)
        })
        .await
        .map_err(|_| AppError::Message("cached embedding total deadline exceeded".into()))?
    }
}
