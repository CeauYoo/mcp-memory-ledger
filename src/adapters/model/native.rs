//! Native non-streaming generation adapters. Protocol transforms remain pure.
use super::{
    prompt,
    protocol::{anthropic, responses},
    transport::{Authentication, Transport},
};
use crate::{
    domain::self_revision::{SelfRevisionProposal, SelfRevisionRequest},
    error::AppError,
    ports::{ModelDecision, ModelDecisionRequest, ModelPort},
    support::config::NativeModelConfig,
};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeProtocol {
    OpenAiResponses,
    Anthropic,
}

impl NativeProtocol {
    fn provider(self) -> &'static str {
        match self {
            Self::OpenAiResponses => "openai-responses",
            Self::Anthropic => "anthropic",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativeModel {
    transport: Transport,
    config: NativeModelConfig,
    protocol: NativeProtocol,
}

impl NativeModel {
    pub fn new(config: NativeModelConfig, protocol: NativeProtocol) -> Result<Self, AppError> {
        let provider = protocol.provider();
        config.validate(provider).map_err(AppError::Message)?;
        let (path, auth) = match protocol {
            NativeProtocol::OpenAiResponses => ("/responses", Authentication::Bearer),
            NativeProtocol::Anthropic => ("/messages", Authentication::Anthropic),
        };
        let transport = Transport::new(
            &config.base_url,
            path,
            &config.api_key,
            config.timeout_ms,
            provider,
            auth,
        )?;
        Ok(Self {
            transport,
            config,
            protocol,
        })
    }

    async fn generate(&self, prompt: prompt::Prompt) -> Result<String, AppError> {
        let payload = match self.protocol {
            NativeProtocol::OpenAiResponses => responses::request(&self.config, prompt),
            NativeProtocol::Anthropic => anthropic::request(&self.config, prompt),
        };
        let body = self.transport.send(payload).await?;
        match self.protocol {
            NativeProtocol::OpenAiResponses => responses::text(body),
            NativeProtocol::Anthropic => anthropic::text(body),
        }
    }
}

#[async_trait]
impl ModelPort for NativeModel {
    async fn decide(&self, request: ModelDecisionRequest) -> Result<ModelDecision, AppError> {
        let text = self.generate(prompt::decision(request)?).await?;
        Ok(ModelDecision::new(prompt::nonempty(
            text,
            self.protocol.provider(),
            "model action",
        )?))
    }

    async fn propose_self_revision(
        &self,
        request: SelfRevisionRequest,
    ) -> Result<SelfRevisionProposal, AppError> {
        prompt::proposal(
            self.generate(prompt::self_revision(request)?).await?,
            self.protocol.provider(),
        )
    }
}
