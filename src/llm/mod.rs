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
mod provider;
pub mod provider_presets;
mod reasoning;
pub mod response_cache;
pub mod response_cache_ext;
pub mod retry;
mod rig_adapter;
pub mod routing_policy;
pub mod smart_routing;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitBreakerProvider};
pub use failover::{CooldownConfig, FailoverProvider};
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    Role, StreamChunk, StreamChunkStream, ThinkingConfig, ToolCall, ToolCompletionRequest,
    ToolCompletionResponse, ToolDefinition, ToolResult,
};
pub use reasoning::{
    is_silent_reply, ActionPlan, Reasoning, ReasoningContext, RespondOutput, RespondResult,
    TokenUsage, ToolSelection, SILENT_REPLY_TOKEN,
};
pub use response_cache::{CachedProvider, ResponseCacheConfig};
pub use retry::{RetryConfig, RetryProvider};
pub use rig_adapter::RigAdapter;
pub use smart_routing::{SmartRoutingConfig, SmartRoutingProvider, TaskComplexity};

use std::sync::Arc;

use rig::client::CompletionClient;
use secrecy::ExposeSecret;

use crate::config::{LlmBackend, LlmConfig};
use crate::error::LlmError;

/// Create an LLM provider based on configuration.
pub fn create_llm_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.backend {
        LlmBackend::OpenAi => create_openai_provider(config),
        LlmBackend::Anthropic => create_anthropic_provider(config),
        LlmBackend::Ollama => create_ollama_provider(config),
        LlmBackend::OpenAiCompatible => create_openai_compatible_provider(config),
        LlmBackend::Tinfoil => create_tinfoil_provider(config),
    }
}

fn create_openai_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let oai = config.openai.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "openai".to_string(),
    })?;

    let api_key = oai.api_key.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "openai (OPENAI_API_KEY not set)".to_string(),
    })?;

    use rig::providers::openai;

    // Use CompletionsClient (Chat Completions API) instead of the default Client
    // (Responses API). The Responses API path in rig-core panics when tool results
    // are sent back because ironclaw doesn't thread `call_id` through its ToolCall
    // type. The Chat Completions API works correctly with the existing code.
    let client: openai::CompletionsClient = if let Some(ref base_url) = oai.base_url {
        tracing::info!(
            "Using OpenAI direct API (chat completions, model: {}, base_url: {})",
            oai.model,
            base_url,
        );
        openai::Client::builder()
            .base_url(base_url)
            .api_key(api_key.expose_secret())
            .build()
    } else {
        tracing::info!(
            "Using OpenAI direct API (chat completions, model: {}, base_url: default)",
            oai.model,
        );
        openai::Client::new(api_key.expose_secret())
    }
    .map_err(|e| LlmError::RequestFailed {
        provider: "openai".to_string(),
        reason: format!("Failed to create OpenAI client: {}", e),
    })?
    .completions_api();

    let model = client.completion_model(&oai.model);
    Ok(Arc::new(RigAdapter::new(model, &oai.model)))
}

fn create_anthropic_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let anth = config
        .anthropic
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "anthropic".to_string(),
        })?;

    let api_key = anth.api_key.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "anthropic (ANTHROPIC_API_KEY not set)".to_string(),
    })?;

    use rig::providers::anthropic;

    let client: anthropic::Client = if let Some(ref base_url) = anth.base_url {
        anthropic::Client::builder()
            .api_key(api_key.expose_secret())
            .base_url(base_url)
            .build()
    } else {
        anthropic::Client::new(api_key.expose_secret())
    }
    .map_err(|e| LlmError::RequestFailed {
        provider: "anthropic".to_string(),
        reason: format!("Failed to create Anthropic client: {}", e),
    })?;

    let model = client.completion_model(&anth.model);
    tracing::info!(
        "Using Anthropic direct API (model: {}, base_url: {})",
        anth.model,
        anth.base_url.as_deref().unwrap_or("default"),
    );
    Ok(Arc::new(RigAdapter::new(model, &anth.model)))
}

fn create_ollama_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let oll = config.ollama.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "ollama".to_string(),
    })?;

    use rig::client::Nothing;
    use rig::providers::ollama;

    let client: ollama::Client = ollama::Client::builder()
        .base_url(&oll.base_url)
        .api_key(Nothing)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "ollama".to_string(),
            reason: format!("Failed to create Ollama client: {}", e),
        })?;

    let model = client.completion_model(&oll.model);
    tracing::info!(
        "Using Ollama (base_url: {}, model: {})",
        oll.base_url,
        oll.model
    );
    Ok(Arc::new(RigAdapter::new(model, &oll.model)))
}

const TINFOIL_BASE_URL: &str = "https://inference.tinfoil.sh/v1";

fn create_tinfoil_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let tf = config
        .tinfoil
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "tinfoil".to_string(),
        })?;

    let api_key = tf.api_key.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "tinfoil (TINFOIL_API_KEY not set)".to_string(),
    })?;

    use rig::providers::openai;

    let client: openai::Client = openai::Client::builder()
        .base_url(TINFOIL_BASE_URL)
        .api_key(api_key.expose_secret())
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "tinfoil".to_string(),
            reason: format!("Failed to create Tinfoil client: {}", e),
        })?;

    // Tinfoil currently only supports the Chat Completions API and not the newer Responses API,
    // so we must explicitly select the completions API here (unlike other OpenAI-compatible providers).
    let client = client.completions_api();
    let model = client.completion_model(&tf.model);
    tracing::info!("Using Tinfoil private inference (model: {})", tf.model);
    Ok(Arc::new(RigAdapter::new(model, &tf.model)))
}

fn create_openai_compatible_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let compat = config
        .openai_compatible
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "openai_compatible".to_string(),
        })?;

    use rig::providers::openai;

    let mut extra_headers = reqwest::header::HeaderMap::new();
    for (key, value) in &compat.extra_headers {
        let name = match reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(header = %key, error = %e, "Skipping LLM_EXTRA_HEADERS entry: invalid header name");
                continue;
            }
        };
        let val = match reqwest::header::HeaderValue::from_str(value) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(header = %key, error = %e, "Skipping LLM_EXTRA_HEADERS entry: invalid header value");
                continue;
            }
        };
        extra_headers.insert(name, val);
    }

    let client: openai::CompletionsClient = openai::Client::builder()
        .base_url(&compat.base_url)
        .api_key(
            compat
                .api_key
                .as_ref()
                .map(|k| k.expose_secret().to_string())
                .unwrap_or_else(|| "no-key".to_string()),
        )
        .http_headers(extra_headers)
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "openai_compatible".to_string(),
            reason: format!("Failed to create OpenAI-compatible client: {}", e),
        })?
        .completions_api();

    let model = client.completion_model(&compat.model);
    tracing::info!(
        "Using OpenAI-compatible endpoint (chat completions, base_url: {}, model: {})",
        compat.base_url,
        compat.model
    );
    Ok(Arc::new(RigAdapter::new(model, &compat.model)))
}

/// Create an LLM provider from a catalog entry.
///
/// Used to instantiate fallback providers for the FailoverProvider chain.
/// The provider is identified by its catalog slug and model name.
fn create_provider_for_catalog_entry(
    provider_slug: &str,
    model: &str,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    use crate::config::provider_catalog::{endpoint_for, ApiStyle};

    let endpoint = endpoint_for(provider_slug).ok_or_else(|| LlmError::RequestFailed {
        provider: provider_slug.to_string(),
        reason: format!("Unknown provider '{}' in catalog", provider_slug),
    })?;

    // Retrieve API key from the injected vars overlay
    let api_key_str = crate::config::helpers::optional_env(endpoint.env_key_name).map_err(|e| {
        LlmError::RequestFailed {
            provider: provider_slug.to_string(),
            reason: format!("Failed to read env var '{}': {}", endpoint.env_key_name, e),
        }
    })?;

    match endpoint.api_style {
        ApiStyle::OpenAi => {
            // Native OpenAI provider
            let key = api_key_str.ok_or_else(|| LlmError::AuthFailed {
                provider: format!("{} ({} not set)", provider_slug, endpoint.env_key_name),
            })?;

            use rig::providers::openai;
            let client: openai::CompletionsClient = openai::Client::builder()
                .base_url(endpoint.base_url)
                .api_key(&key)
                .build()
                .map_err(|e| LlmError::RequestFailed {
                    provider: provider_slug.to_string(),
                    reason: format!("Failed to create OpenAI client: {}", e),
                })?
                .completions_api();

            let m = client.completion_model(model);
            tracing::info!(
                "Created provider '{}' (OpenAI native, model: {})",
                provider_slug,
                model
            );
            Ok(Arc::new(RigAdapter::new(m, model)))
        }
        ApiStyle::Anthropic => {
            // Native Anthropic provider
            let key = api_key_str.ok_or_else(|| LlmError::AuthFailed {
                provider: format!("{} ({} not set)", provider_slug, endpoint.env_key_name),
            })?;

            use rig::providers::anthropic;
            let client: anthropic::Client =
                anthropic::Client::new(&key).map_err(|e| LlmError::RequestFailed {
                    provider: provider_slug.to_string(),
                    reason: format!("Failed to create Anthropic client: {}", e),
                })?;

            let m = client.completion_model(model);
            tracing::info!(
                "Created provider '{}' (Anthropic native, model: {})",
                provider_slug,
                model
            );
            Ok(Arc::new(RigAdapter::new(m, model)))
        }
        ApiStyle::OpenAiCompatible => {
            // OpenAI-compatible endpoint (groq, gemini, mistral, xai, etc.)
            let key = api_key_str.unwrap_or_else(|| "no-key".to_string());

            use rig::providers::openai;
            let client: openai::CompletionsClient = openai::Client::builder()
                .base_url(endpoint.base_url)
                .api_key(&key)
                .build()
                .map_err(|e| LlmError::RequestFailed {
                    provider: provider_slug.to_string(),
                    reason: format!("Failed to create OpenAI-compatible client: {}", e),
                })?
                .completions_api();

            let m = client.completion_model(model);
            tracing::info!(
                "Created provider '{}' (OpenAI-compatible, base: {}, model: {})",
                provider_slug,
                endpoint.base_url,
                model
            );
            Ok(Arc::new(RigAdapter::new(m, model)))
        }
        ApiStyle::Ollama => {
            // Ollama doesn't need an API key
            use rig::client::Nothing;
            use rig::providers::ollama;
            let base_url = crate::config::helpers::optional_env("OLLAMA_BASE_URL")
                .ok()
                .flatten()
                .unwrap_or_else(|| endpoint.base_url.to_string());

            let client: ollama::Client = ollama::Client::builder()
                .base_url(&base_url)
                .api_key(Nothing)
                .build()
                .map_err(|e| LlmError::RequestFailed {
                    provider: provider_slug.to_string(),
                    reason: format!("Failed to create Ollama client: {}", e),
                })?;

            let m = client.completion_model(model);
            tracing::info!(
                "Created provider '{}' (Ollama, base: {}, model: {})",
                provider_slug,
                base_url,
                model
            );
            Ok(Arc::new(RigAdapter::new(m, model)))
        }
    }
}

/// Create a cheap model provider from a "provider/model" string.
///
/// Used for SmartRoutingProvider's cheap model split.
fn create_cheap_model_provider(cheap_model_spec: &str) -> Result<Arc<dyn LlmProvider>, LlmError> {
    if let Some((provider, model)) = cheap_model_spec.split_once('/') {
        create_provider_for_catalog_entry(provider, model)
    } else {
        Err(LlmError::RequestFailed {
            provider: "smart_routing".to_string(),
            reason: format!(
                "Invalid cheap_model format '{}'. Expected 'provider/model'.",
                cheap_model_spec
            ),
        })
    }
}

/// Build the full LLM provider chain with multi-provider support.
///
/// Applies decorators in this order:
/// 1. Raw primary provider (from LlmConfig — the user's selected backend)
/// 2. FailoverProvider wrapping primary + all enabled fallback providers
/// 3. RetryProvider (per-provider retry with exponential backoff)
/// 4. SmartRoutingProvider (cheap/primary split when cheap model is configured)
/// 5. CircuitBreakerProvider (fast-fail when backend is degraded)
/// 6. CachedProvider (in-memory response cache)
///
/// When `providers_settings` is `Some`, creates additional providers from the
/// catalog for each enabled provider that has an API key available. This enables
/// multi-provider failover using the already-implemented `FailoverProvider`.
///
/// Also returns a separate cheap LLM provider for heartbeat/evaluation.
///
/// This is the single source of truth for provider chain construction,
/// called by both `main.rs` and `app.rs`.
#[allow(clippy::type_complexity)]
pub fn build_provider_chain(
    config: &LlmConfig,
    providers_settings: Option<&crate::settings::ProvidersSettings>,
) -> Result<(Arc<dyn LlmProvider>, Option<Arc<dyn LlmProvider>>), LlmError> {
    let rel = &config.reliability;

    let primary = create_llm_provider(config)?;
    let primary_model_name = primary.model_name().to_string();
    tracing::info!("Primary LLM provider initialized: {}", primary_model_name);

    // ── 1. Build multi-provider failover chain ───────────────────────────
    let llm: Arc<dyn LlmProvider> = if let Some(ps) = providers_settings {
        let mut all_providers: Vec<Arc<dyn LlmProvider>> = vec![primary.clone()];

        // Determine fallback providers from ProvidersSettings.
        // Use explicit fallback_chain if provided, otherwise auto-build
        // from enabled providers.
        let fallbacks: Vec<(String, String)> = if !ps.fallback_chain.is_empty() {
            // Explicit chain: parse "provider/model" entries
            ps.fallback_chain
                .iter()
                .filter_map(|entry| {
                    entry
                        .split_once('/')
                        .map(|(p, m)| (p.to_string(), m.to_string()))
                })
                .collect()
        } else {
            // Auto-build: use all enabled providers that aren't the primary
            let catalog = crate::config::provider_catalog::catalog();
            ps.enabled
                .iter()
                .filter(|slug| {
                    // Skip if this is the primary provider
                    ps.primary.as_deref() != Some(slug.as_str())
                })
                .filter_map(|slug| {
                    let endpoint = catalog.get(slug.as_str())?;
                    // Determine model: first from allowed_models, else default
                    let model = ps
                        .allowed_models
                        .get(slug.as_str())
                        .and_then(|m| m.first().cloned())
                        .unwrap_or_else(|| endpoint.default_model.to_string());
                    Some((slug.clone(), model))
                })
                .collect()
        };

        for (provider_slug, model) in &fallbacks {
            match create_provider_for_catalog_entry(provider_slug, model) {
                Ok(p) => {
                    tracing::info!(
                        "Failover provider added: '{}' (model: {})",
                        provider_slug,
                        model
                    );
                    all_providers.push(p);
                }
                Err(e) => {
                    tracing::warn!("Skipping fallback provider '{}': {}", provider_slug, e);
                }
            }
        }

        if all_providers.len() > 1 {
            let cooldown = CooldownConfig {
                failure_threshold: rel.failover_cooldown_threshold,
                cooldown_duration: std::time::Duration::from_secs(rel.failover_cooldown_secs),
            };
            tracing::info!(
                "FailoverProvider enabled with {} providers (cooldown: {}s, threshold: {})",
                all_providers.len(),
                rel.failover_cooldown_secs,
                rel.failover_cooldown_threshold,
            );
            Arc::new(FailoverProvider::with_cooldown(all_providers, cooldown)?)
        } else {
            primary
        }
    } else {
        primary
    };

    // ── 2. Retry ─────────────────────────────────────────────────────────
    let retry_config = RetryConfig {
        max_retries: rel.max_retries,
    };
    let llm: Arc<dyn LlmProvider> = if retry_config.max_retries > 0 {
        tracing::info!(
            max_retries = retry_config.max_retries,
            "LLM retry wrapper enabled"
        );
        Arc::new(RetryProvider::new(llm, retry_config.clone()))
    } else {
        llm
    };

    // ── 3. Smart routing (cheap/primary split) ───────────────────────────
    // Determine cheap model: explicit config > providers_settings > none
    let cheap_model_spec = rel
        .cheap_model
        .clone()
        .or_else(|| providers_settings.and_then(|ps| ps.cheap_model.clone()));

    let cheap_llm: Option<Arc<dyn LlmProvider>> = if let Some(ref spec) = cheap_model_spec {
        match create_cheap_model_provider(spec) {
            Ok(p) => {
                tracing::info!("Smart routing cheap model: {}", spec);
                Some(p)
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to create cheap model provider '{}': {}. \
                     Smart routing disabled.",
                    spec,
                    e
                );
                None
            }
        }
    } else {
        None
    };

    let llm: Arc<dyn LlmProvider> = if let Some(ref cheap) = cheap_llm {
        tracing::info!("SmartRoutingProvider enabled (primary + cheap model)");
        Arc::new(SmartRoutingProvider::new(
            llm,
            cheap.clone(),
            SmartRoutingConfig::default(),
        ))
    } else {
        llm
    };

    // ── 4. Circuit breaker ───────────────────────────────────────────────
    let llm: Arc<dyn LlmProvider> = if let Some(threshold) = rel.circuit_breaker_threshold {
        let cb_config = CircuitBreakerConfig {
            failure_threshold: threshold,
            recovery_timeout: std::time::Duration::from_secs(rel.circuit_breaker_recovery_secs),
            ..CircuitBreakerConfig::default()
        };
        tracing::info!(
            threshold,
            recovery_secs = rel.circuit_breaker_recovery_secs,
            "LLM circuit breaker enabled"
        );
        Arc::new(CircuitBreakerProvider::new(llm, cb_config))
    } else {
        llm
    };

    // ── 5. Response cache ────────────────────────────────────────────────
    let llm: Arc<dyn LlmProvider> = if rel.response_cache_enabled {
        let rc_config = ResponseCacheConfig {
            ttl: std::time::Duration::from_secs(rel.response_cache_ttl_secs),
            max_entries: rel.response_cache_max_entries,
        };
        tracing::info!(
            ttl_secs = rel.response_cache_ttl_secs,
            max_entries = rel.response_cache_max_entries,
            "LLM response cache enabled"
        );
        Arc::new(CachedProvider::new(llm, rc_config))
    } else {
        llm
    };

    Ok((llm, cheap_llm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LlmBackend, ReliabilityConfig};

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
