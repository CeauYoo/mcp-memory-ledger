//! Optional embedding boundary. Callers must scope and bound documents before embedding.
use crate::{domain::types::Namespace, error::AppError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingProfile {
    pub model: String,
    pub model_digest: String,
    pub dimensions: usize,
    pub document_prefix: String,
    pub query_prefix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingPurpose {
    Document,
    Query,
}

#[async_trait]
pub trait EmbeddingPort: Send + Sync {
    fn profile(&self) -> &EmbeddingProfile;
    fn take_warnings(&self) -> Vec<String> {
        Vec::new()
    }
    async fn embed(
        &self,
        namespace: &Namespace,
        purpose: EmbeddingPurpose,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, AppError>;
}
