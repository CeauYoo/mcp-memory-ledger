//! Shared model construction for explicit local commands.
use crate::{
    adapters::model::{
        mock::MockModel,
        native::{NativeModel, NativeProtocol},
        openai_compatible::OpenAiCompatibleModel,
    },
    error::AppError,
    ports::ModelPort,
    support::config::ModelConfig,
};
pub fn build(config: &ModelConfig) -> Result<Box<dyn ModelPort + Send + Sync>, AppError> {
    Ok(match config {
        ModelConfig::Mock => Box::new(MockModel),
        ModelConfig::OpenAiCompatible(c) => Box::new(OpenAiCompatibleModel::new(c.clone())?),
        ModelConfig::OpenRouter(c) => Box::new(OpenAiCompatibleModel::new_for_provider(
            c.clone(),
            "openrouter",
        )?),
        ModelConfig::OpenAiResponses(c) => Box::new(NativeModel::new(
            c.clone(),
            NativeProtocol::OpenAiResponses,
        )?),
        ModelConfig::Anthropic(c) => {
            Box::new(NativeModel::new(c.clone(), NativeProtocol::Anthropic)?)
        }
    })
}
