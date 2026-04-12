//! LLM integration for the agent.
//!
//! Supports multiple backends:
//! - **OpenAI-compatible** (default): Any endpoint speaking the OpenAI Chat Completions API
//! - **OpenAI**: Direct API access with your own key
//! - **Anthropic**: Direct API access with your own key
//! - **Ollama**: Local model inference
//! - **Tinfoil**: Private inference via Tinfoil

pub mod bedrock;
pub mod circuit_breaker;
pub mod cost_tracker;
pub mod costs;
pub mod discovery;
pub mod embeddings;
pub mod extended_context;
pub mod failover;
pub mod gemini;
pub mod llama_cpp;
pub mod llm_hooks;
pub mod llms_txt;
pub mod pricing_sync;
mod provider;
pub(crate) mod provider_factory;
pub mod provider_presets;
mod reasoning;
mod reasoning_tags;
pub mod response_cache;
pub mod response_cache_ext;
pub mod retry;
mod rig_adapter;
pub mod route_planner;
pub mod routing_policy;
pub mod runtime_manager;
pub mod smart_routing;
pub mod usage_tracking;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitBreakerProvider};
pub use failover::{CooldownConfig, FailoverProvider};
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, StreamChunk, StreamChunkStream, ThinkingConfig, ToolCall, ToolCompletionRequest,
    ToolCompletionResponse, ToolDefinition, ToolResult, sanitize_tool_messages,
};
pub use provider_factory::{build_provider_chain, create_llm_provider};
pub use reasoning::{
    ActionPlan, Reasoning, ReasoningContext, RespondOutput, RespondResult, SILENT_REPLY_TOKEN,
    TokenUsage, ToolSelection, is_silent_reply,
};
pub use response_cache::{CachedProvider, ResponseCacheConfig};
pub use retry::{RetryConfig, RetryProvider};
pub use rig_adapter::RigAdapter;
pub use runtime_manager::{
    LlmRuntimeManager, RouteSimulationResult, RouteSimulationScore, RuntimeStatus,
    derive_runtime_defaults, normalize_providers_settings, validate_providers_settings,
};
pub use smart_routing::{SmartRoutingConfig, SmartRoutingProvider, TaskComplexity};
pub use usage_tracking::UsageTrackingProvider;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LlmBackend, LlmConfig, ReliabilityConfig};

    fn test_llm_config() -> LlmConfig {
        LlmConfig {
            backend: LlmBackend::OpenAiCompatible,
            openai: None,
            anthropic: None,
            ollama: None,
            openai_compatible: Some(crate::config::OpenAiCompatibleConfig {
                base_url: "http://localhost:8080".to_string(),
                api_key: None,
                model: "test-model".to_string(),
                extra_headers: Vec::new(),
            }),
            tinfoil: None,
            gemini: None,
            bedrock: None,
            llama_cpp: None,
            reliability: ReliabilityConfig::default(),
        }
    }

    #[test]
    fn test_default_backend_is_openai_compatible() {
        assert_eq!(LlmBackend::default(), LlmBackend::OpenAiCompatible);
    }

    #[test]
    fn test_build_provider_chain_creates_provider() {
        let config = test_llm_config();
        // Without providers_settings, single-provider mode
        let result = build_provider_chain(&config, None);
        assert!(result.is_ok());
        let (llm, cheap) = result.unwrap();
        assert_eq!(llm.model_name(), "test-model");
        assert!(cheap.is_none()); // No cheap model configured
    }

    #[test]
    fn test_build_provider_chain_with_empty_providers() {
        let config = test_llm_config();
        let providers = crate::settings::ProvidersSettings::default();
        // Empty providers settings should still work (no failover, just primary)
        let result = build_provider_chain(&config, Some(&providers));
        assert!(result.is_ok());
        let (llm, _) = result.unwrap();
        assert_eq!(llm.model_name(), "test-model");
    }
}
