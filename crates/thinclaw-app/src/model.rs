//! Root-independent LLM model override helpers for binary entrypoints.

use thinclaw_config::llm::{LlmBackend, LlmConfig};

/// Returns the currently configured model identifier for the selected backend.
pub fn overridden_model_for_backend(config: &LlmConfig) -> Option<&str> {
    match config.backend {
        LlmBackend::OpenAi => config
            .openai
            .as_ref()
            .map(|provider| provider.model.as_str()),
        LlmBackend::Anthropic => config
            .anthropic
            .as_ref()
            .map(|provider| provider.model.as_str()),
        LlmBackend::Ollama => config
            .ollama
            .as_ref()
            .map(|provider| provider.model.as_str()),
        LlmBackend::OpenAiCompatible => config
            .openai_compatible
            .as_ref()
            .map(|provider| provider.model.as_str()),
        LlmBackend::Tinfoil => config
            .tinfoil
            .as_ref()
            .map(|provider| provider.model.as_str()),
        LlmBackend::Gemini => config
            .gemini
            .as_ref()
            .map(|provider| provider.model.as_str()),
        LlmBackend::Bedrock => config
            .bedrock
            .as_ref()
            .map(|provider| provider.model_id.as_str()),
        LlmBackend::LlamaCpp => config
            .llama_cpp
            .as_ref()
            .map(|provider| provider.model.as_str()),
    }
}

/// Applies a model override to whichever provider config matches the selected backend.
pub fn apply_model_override(config: &mut LlmConfig, model: impl Into<String>) {
    let model = model.into();
    match config.backend {
        LlmBackend::OpenAi => {
            if let Some(ref mut provider) = config.openai {
                provider.model = model;
            }
        }
        LlmBackend::Anthropic => {
            if let Some(ref mut provider) = config.anthropic {
                provider.model = model;
            }
        }
        LlmBackend::Ollama => {
            if let Some(ref mut provider) = config.ollama {
                provider.model = model;
            }
        }
        LlmBackend::OpenAiCompatible => {
            if let Some(ref mut provider) = config.openai_compatible {
                provider.model = model;
            }
        }
        LlmBackend::Tinfoil => {
            if let Some(ref mut provider) = config.tinfoil {
                provider.model = model;
            }
        }
        LlmBackend::Gemini => {
            if let Some(ref mut provider) = config.gemini {
                provider.model = model;
            }
        }
        LlmBackend::Bedrock => {
            if let Some(ref mut provider) = config.bedrock {
                provider.model_id = model;
            }
        }
        LlmBackend::LlamaCpp => {
            if let Some(ref mut provider) = config.llama_cpp {
                provider.model = model;
            }
        }
    }
}
