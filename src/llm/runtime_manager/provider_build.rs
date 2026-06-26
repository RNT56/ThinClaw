//! Provider construction and routing-policy assembly helpers.
//!
//! Free functions that turn provider slugs / model specs into concrete
//! `LlmProvider` instances, build the core routing policy from settings, and
//! derive route metadata (logical role, capability metadata, kill switch).

use std::sync::Arc;

use crate::config::{Config, LlmBackend};
use crate::error::LlmError;
use crate::llm::provider::LlmProvider;
use crate::llm::provider_factory::{
    create_llm_provider, create_provider_for_catalog_entry_with_settings,
};
use crate::llm::routing_policy::{
    RoutingPolicy, RoutingPolicyConfig, build_routing_policy as build_core_routing_policy,
};
use crate::settings::{ProvidersSettings, RoutingMode};

use super::provider_slots::provider_slot_selectors;
use super::types::ProviderModelRole;

pub(super) fn logical_role_for_target(target: &str) -> &'static str {
    if target == "cheap" || target.ends_with("@cheap") {
        "cheap"
    } else if target == "primary" || target.ends_with("@primary") {
        "primary"
    } else {
        "direct"
    }
}

pub(super) fn capability_metadata_for_route(
    provider_slug: Option<&str>,
    model_id: Option<&str>,
) -> crate::llm::routing_policy::ProviderCapabilitiesMetadata {
    let mut metadata = crate::llm::routing_policy::ProviderCapabilitiesMetadata::default();
    if let Some(model_id) = model_id {
        let compat = crate::config::model_compat::find_model(model_id).or_else(|| {
            model_id
                .split_once('/')
                .and_then(|(_, model)| crate::config::model_compat::find_model(model))
        });
        if let Some(compat) = compat {
            metadata.supports_streaming = Some(compat.supports_streaming);
            metadata.supports_tools = Some(compat.supports_tools);
            metadata.supports_vision = Some(compat.supports_vision);
            metadata.supports_thinking = Some(compat.supports_thinking);
            metadata.max_context_tokens = Some(compat.context_window);
        }
    }

    if let Some(slug) = provider_slug
        && let Some(endpoint) = crate::config::provider_catalog::endpoint_for(slug)
    {
        if metadata.supports_streaming.is_none() {
            metadata.supports_streaming = Some(endpoint.supports_streaming);
        }
        if metadata.max_context_tokens.is_none() {
            metadata.max_context_tokens = Some(endpoint.default_context_size);
        }
    }

    // Explicitly-known limitation: Perplexity models currently do not expose
    // function/tool calling in ThinClaw's routing layer.
    if provider_slug == Some("perplexity") {
        metadata.supports_tools = Some(false);
    }

    metadata
}

pub(super) fn routing_kill_switch_enabled() -> bool {
    std::env::var("THINCLAW_ROUTING_KILL_SWITCH")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

pub(super) fn create_provider_for_runtime_slug(
    provider: &str,
    model: &str,
    base_config: &crate::config::LlmConfig,
    providers_settings: Option<&ProvidersSettings>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    if crate::config::provider_catalog::endpoint_for(provider).is_some() {
        return create_provider_for_catalog_entry_with_settings(
            provider,
            model,
            providers_settings,
        );
    }

    let mut llm_config = base_config.clone();
    llm_config.backend = match provider {
        "openai" => LlmBackend::OpenAi,
        "anthropic" => LlmBackend::Anthropic,
        "gemini" => LlmBackend::Gemini,
        "tinfoil" => LlmBackend::Tinfoil,
        "ollama" => LlmBackend::Ollama,
        "openai_compatible" | "openrouter" => LlmBackend::OpenAiCompatible,
        "bedrock" => LlmBackend::Bedrock,
        "llama_cpp" => LlmBackend::LlamaCpp,
        other => {
            return Err(LlmError::RequestFailed {
                provider: "runtime".to_string(),
                reason: format!("Unknown provider slug '{}'", other),
            });
        }
    };

    apply_model_override(&mut llm_config, model);
    create_llm_provider(&llm_config)
}

pub(super) fn apply_model_override(config: &mut crate::config::LlmConfig, model: &str) {
    match config.backend {
        LlmBackend::OpenAi => {
            if let Some(ref mut openai) = config.openai {
                openai.model = model.to_string();
            }
        }
        LlmBackend::Anthropic => {
            if let Some(ref mut anthropic) = config.anthropic {
                anthropic.model = model.to_string();
            }
        }
        LlmBackend::Ollama => {
            if let Some(ref mut ollama) = config.ollama {
                ollama.model = model.to_string();
            }
        }
        LlmBackend::OpenAiCompatible => {
            if let Some(ref mut compat) = config.openai_compatible {
                compat.model = model.to_string();
            }
        }
        LlmBackend::Tinfoil => {
            if let Some(ref mut tinfoil) = config.tinfoil {
                tinfoil.model = model.to_string();
            }
        }
        LlmBackend::Gemini => {
            if let Some(ref mut gemini) = config.gemini {
                gemini.model = model.to_string();
            }
        }
        LlmBackend::Bedrock => {
            if let Some(ref mut bedrock) = config.bedrock {
                bedrock.model_id = model.to_string();
            }
        }
        LlmBackend::LlamaCpp => {
            if let Some(ref mut llama_cpp) = config.llama_cpp {
                llama_cpp.model = model.to_string();
            }
        }
    }
}

pub(super) fn build_routing_policy(settings: &ProvidersSettings) -> RoutingPolicy {
    let prefer_cheap_default = settings.routing_mode == RoutingMode::Policy
        && settings.smart_routing_enabled
        && !provider_slot_selectors(settings, ProviderModelRole::Cheap).is_empty();

    build_core_routing_policy(RoutingPolicyConfig {
        smart_routing_enabled: settings.smart_routing_enabled,
        prefer_cheap_default,
        rules: &settings.policy_rules,
    })
}

pub(super) fn legacy_primary_slug_from_config(config: &Config) -> Option<String> {
    match config.llm.backend {
        LlmBackend::OpenAi => Some("openai".to_string()),
        LlmBackend::Anthropic => Some("anthropic".to_string()),
        LlmBackend::Ollama => Some("ollama".to_string()),
        LlmBackend::OpenAiCompatible => config
            .llm
            .openai_compatible
            .as_ref()
            .and_then(|compat| {
                if compat.base_url.contains("openrouter.ai") {
                    Some("openrouter".to_string())
                } else {
                    None
                }
            })
            .or_else(|| Some("openai_compatible".to_string())),
        LlmBackend::Tinfoil => Some("tinfoil".to_string()),
        LlmBackend::Gemini => Some("gemini".to_string()),
        LlmBackend::Bedrock => Some("bedrock".to_string()),
        LlmBackend::LlamaCpp => Some("llama_cpp".to_string()),
    }
}
