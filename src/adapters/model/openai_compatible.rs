use super::{
    prompt,
    protocol::chat_completions,
    transport::{Authentication, Transport},
};
use crate::{
    domain::self_revision::{SelfRevisionProposal, SelfRevisionRequest},
    error::AppError,
    ports::{ModelDecision, ModelDecisionRequest, ModelPort},
    support::config::OpenAiCompatibleConfig,
};
use async_trait::async_trait;

/// Compatibility adapter for existing Chat Completions and OpenRouter configuration.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleModel {
    transport: Transport,
    model: String,
    provider_name: &'static str,
}

impl OpenAiCompatibleModel {
    pub fn new(config: OpenAiCompatibleConfig) -> Result<Self, AppError> {
        Self::new_for_provider(config, "openai-compatible")
    }

    pub fn new_for_provider(
        config: OpenAiCompatibleConfig,
        provider_name: &'static str,
    ) -> Result<Self, AppError> {
        Ok(Self {
            transport: Transport::new(
                &config.base_url,
                "/chat/completions",
                &config.api_key,
                config.timeout_ms,
                provider_name,
                Authentication::Bearer,
            )?,
            model: config.model,
            provider_name,
        })
    }

    async fn generate(&self, prompt: prompt::Prompt) -> Result<String, AppError> {
        let body = self
            .transport
            .send(chat_completions::request(&self.model, prompt))
            .await?;
        chat_completions::text(body, self.provider_name)
    }
}

#[async_trait]
impl ModelPort for OpenAiCompatibleModel {
    async fn decide(&self, request: ModelDecisionRequest) -> Result<ModelDecision, AppError> {
        let text = self.generate(prompt::decision(request)?).await?;
        Ok(ModelDecision::new(prompt::nonempty(
            text,
            self.provider_name,
            "model action",
        )?))
    }

    async fn propose_self_revision(
        &self,
        request: SelfRevisionRequest,
    ) -> Result<SelfRevisionProposal, AppError> {
        prompt::proposal(
            self.generate(prompt::self_revision(request)?).await?,
            self.provider_name,
        )
    }
}
