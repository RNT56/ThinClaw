//! LLM provider factory and provider chain builder.
//!
//! Contains all backend-specific provider constructors (OpenAI, Anthropic,
//! Ollama, Gemini, Bedrock, llama.cpp, Tinfoil) and the `build_provider_chain`
//! function that composes them with retry, failover, circuit breaker, smart
//! routing, and response caching decorators.

use std::sync::Arc;

use rig::client::CompletionClient;
use secrecy::{ExposeSecret, SecretString};

use crate::RigAdapter;
use crate::circuit_breaker::{CircuitBreakerConfig, CircuitBreakerProvider};
use crate::failover::{
    CooldownConfig, FailoverProvider, LeaseConfig, LeaseSelectionStrategy, ProviderLeaseEntry,
};
use crate::response_cache::{CachedProvider, ResponseCacheConfig};
use crate::retry::{RetryConfig, RetryProvider};
use crate::smart_routing::{SmartRoutingConfig, SmartRoutingProvider};
use thinclaw_config::{LlmBackend, LlmConfig};
use thinclaw_llm_core::{LlmProvider, StreamSupport, TokenCaptureSupport};
use thinclaw_settings::{
    CredentialSelectionStrategy, ProviderCredentialMode, ProvidersSettings, RoutingMode,
};
use thinclaw_types::error::LlmError;

/// Create an LLM provider based on configuration.
pub fn create_llm_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    match config.backend {
        LlmBackend::OpenAi => create_openai_provider(config),
        LlmBackend::Anthropic => create_anthropic_provider(config),
        LlmBackend::Ollama => create_ollama_provider(config),
        LlmBackend::OpenAiCompatible => create_openai_compatible_provider(config),
        LlmBackend::Tinfoil => create_tinfoil_provider(config),
        LlmBackend::Gemini => create_gemini_provider(config),
        LlmBackend::Bedrock => create_bedrock_provider(config),
        LlmBackend::LlamaCpp => create_llama_cpp_provider(config),
    }
}

fn credential_entry_id(provider_slug: &str, index: usize) -> String {
    format!("{provider_slug}:credential:{}", index + 1)
}

fn openai_logprob_capture_support() -> TokenCaptureSupport {
    TokenCaptureSupport {
        exact_tokens_supported: true,
        logprobs_supported: true,
    }
}

fn openai_logprob_request_params() -> serde_json::Value {
    serde_json::json!({
        "logprobs": true,
        "top_logprobs": 0
    })
}

fn provider_prefers_external_oauth_sync(
    providers_settings: Option<&ProvidersSettings>,
    provider_slug: &str,
) -> bool {
    providers_settings
        .and_then(|settings| settings.provider_credential_modes.get(provider_slug))
        .copied()
        == Some(ProviderCredentialMode::ExternalOAuthSync)
}

fn resolved_secret_credentials(
    api_keys: &[SecretString],
    primary: &Option<SecretString>,
) -> Vec<SecretString> {
    if !api_keys.is_empty() {
        return api_keys.to_vec();
    }
    primary.iter().cloned().collect()
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
    // are sent back because thinclaw doesn't thread `call_id` through its ToolCall
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
    Ok(Arc::new(
        RigAdapter::new(model, &oai.model)
            .with_provider_label("openai")
            .with_token_capture(
                openai_logprob_capture_support(),
                Some(openai_logprob_request_params()),
            ),
    ))
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

    let model = client.completion_model(&anth.model).with_prompt_caching();
    tracing::info!(
        "Using Anthropic direct API (model: {}, base_url: {})",
        anth.model,
        anth.base_url.as_deref().unwrap_or("default"),
    );
    Ok(Arc::new(
        RigAdapter::new_with_prompt_caching(model, &anth.model, true)
            .with_provider_label("anthropic"),
    ))
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
    Ok(Arc::new(
        RigAdapter::new(model, &oll.model).with_provider_label("ollama"),
    ))
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
    Ok(Arc::new(
        RigAdapter::new(model, &tf.model).with_provider_label("tinfoil"),
    ))
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
    Ok(Arc::new(
        RigAdapter::new(model, &compat.model)
            .with_provider_label("openai_compatible")
            .with_token_capture(
                openai_logprob_capture_support(),
                Some(openai_logprob_request_params()),
            ),
    ))
}

fn runtime_extra_headers() -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    let raw = thinclaw_config::helpers::optional_env("LLM_EXTRA_HEADERS")
        .ok()
        .flatten()
        .unwrap_or_default();

    for part in raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Some((key, value)) = part.split_once(':') else {
            tracing::warn!(entry = %part, "Skipping malformed LLM_EXTRA_HEADERS entry");
            continue;
        };

        let name = match reqwest::header::HeaderName::from_bytes(key.trim().as_bytes()) {
            Ok(name) => name,
            Err(err) => {
                tracing::warn!(header = %key, error = %err, "Skipping invalid header name");
                continue;
            }
        };
        let value = match reqwest::header::HeaderValue::from_str(value.trim()) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(header = %key, error = %err, "Skipping invalid header value");
                continue;
            }
        };
        headers.insert(name, value);
    }

    headers
}

/// Create an LLM provider from a catalog entry.
///
/// Used to instantiate fallback providers for the FailoverProvider chain,
/// and by the dispatcher for agent-driven model switching (`llm_select` tool).
/// The provider is identified by its catalog slug and model name.
pub fn create_provider_for_catalog_entry(
    provider_slug: &str,
    model: &str,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    create_provider_for_catalog_entry_with_api_key(provider_slug, model, None)
}

/// Create an LLM provider from a catalog entry using scoped runtime credentials.
///
/// When more than one credential is resolved for the provider, the returned
/// provider is a credential-level failover chain. This avoids depending on the
/// process-wide config overlay for secrets loaded from the encrypted store.
pub fn create_provider_for_catalog_entry_with_settings(
    provider_slug: &str,
    model: &str,
    providers_settings: Option<&ProvidersSettings>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let entries =
        create_provider_variants_for_catalog_entry(provider_slug, model, providers_settings)?;
    if entries.len() == 1 {
        Ok(Arc::clone(&entries[0].provider))
    } else {
        Ok(Arc::new(FailoverProvider::with_entries(
            entries,
            CooldownConfig::default(),
            LeaseConfig::default(),
        )?))
    }
}

fn create_provider_for_catalog_entry_with_api_key(
    provider_slug: &str,
    model: &str,
    api_key_override: Option<&str>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    use thinclaw_config::provider_catalog::{ApiStyle, endpoint_for};

    let endpoint = endpoint_for(provider_slug).ok_or_else(|| LlmError::RequestFailed {
        provider: provider_slug.to_string(),
        reason: format!("Unknown provider '{}' in catalog", provider_slug),
    })?;
    let stream_support = if endpoint.supports_streaming {
        StreamSupport::Native
    } else {
        StreamSupport::Unsupported
    };

    // Retrieve API key from the injected vars overlay
    let api_key_str = if let Some(api_key_override) = api_key_override {
        Some(api_key_override.to_string())
    } else {
        thinclaw_config::helpers::optional_env(&endpoint.env_key_name)
            .map_err(|e| LlmError::RequestFailed {
                provider: provider_slug.to_string(),
                reason: format!("Failed to read env var '{}': {}", endpoint.env_key_name, e),
            })?
            .or_else(|| {
                if provider_slug == "openrouter" {
                    thinclaw_config::helpers::optional_env("LLM_API_KEY")
                        .ok()
                        .flatten()
                } else {
                    None
                }
            })
    };

    match endpoint.api_style {
        ApiStyle::OpenAi => {
            // Native OpenAI provider
            let key = api_key_str.ok_or_else(|| LlmError::AuthFailed {
                provider: format!("{} ({} not set)", provider_slug, endpoint.env_key_name),
            })?;

            use rig::providers::openai;
            let client: openai::CompletionsClient = openai::Client::builder()
                .base_url(&endpoint.base_url)
                .api_key(&key)
                .http_headers(runtime_extra_headers())
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
            Ok(Arc::new(
                RigAdapter::new_with_stream_support(m, model, stream_support)
                    .with_provider_label(provider_slug)
                    .with_token_capture(
                        openai_logprob_capture_support(),
                        Some(openai_logprob_request_params()),
                    ),
            ))
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
            Ok(Arc::new(
                RigAdapter::new_with_prompt_caching_and_stream_support(
                    m,
                    model,
                    true,
                    stream_support,
                )
                .with_provider_label(provider_slug),
            ))
        }
        ApiStyle::OpenAiCompatible => {
            // OpenAI-compatible endpoint (groq, gemini, mistral, xai, etc.)
            let key = api_key_str.unwrap_or_else(|| "no-key".to_string());

            use rig::providers::openai;
            let client: openai::CompletionsClient = openai::Client::builder()
                .base_url(&endpoint.base_url)
                .api_key(&key)
                .http_headers(runtime_extra_headers())
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
            Ok(Arc::new(
                RigAdapter::new_with_stream_support(m, model, stream_support)
                    .with_provider_label(provider_slug)
                    .with_token_capture(
                        openai_logprob_capture_support(),
                        Some(openai_logprob_request_params()),
                    ),
            ))
        }
        ApiStyle::Ollama => {
            // Ollama doesn't need an API key
            use rig::client::Nothing;
            use rig::providers::ollama;
            let base_url = thinclaw_config::helpers::optional_env("OLLAMA_BASE_URL")
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
            Ok(Arc::new(
                RigAdapter::new_with_stream_support(m, model, stream_support)
                    .with_provider_label(provider_slug),
            ))
        }
    }
}

fn resolve_catalog_api_keys(
    provider_slug: &str,
    env_key_name: &str,
    providers_settings: Option<&ProvidersSettings>,
) -> Result<Vec<String>, LlmError> {
    if provider_prefers_external_oauth_sync(providers_settings, provider_slug)
        && let Some(value) = thinclaw_config::helpers::synced_oauth_env(env_key_name)
        && !value.trim().is_empty()
    {
        return Ok(vec![value]);
    }

    if let Some(values) = providers_settings
        .and_then(|settings| settings.resolved_provider_api_keys.get(provider_slug))
        .filter(|values| !values.is_empty())
    {
        return Ok(values
            .iter()
            .map(|value| value.expose_secret().trim().to_string())
            .filter(|value| !value.is_empty())
            .collect());
    }

    fn append_from_env(target: &mut Vec<String>, env_name: &str) -> Result<(), LlmError> {
        if let Some(value) = thinclaw_config::helpers::optional_env(env_name).map_err(|e| {
            LlmError::RequestFailed {
                provider: env_name.to_string(),
                reason: format!("Failed to read env var '{}': {}", env_name, e),
            }
        })? && !value.trim().is_empty()
            && !target.iter().any(|existing| existing == value.trim())
        {
            target.push(value.trim().to_string());
        }
        Ok(())
    }

    fn append_list_from_env(target: &mut Vec<String>, env_name: &str) -> Result<(), LlmError> {
        if let Some(raw) = thinclaw_config::helpers::optional_env(env_name).map_err(|e| {
            LlmError::RequestFailed {
                provider: env_name.to_string(),
                reason: format!("Failed to read env var '{}': {}", env_name, e),
            }
        })? {
            for value in raw
                .split([',', '\n'])
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !target.iter().any(|existing| existing == value) {
                    target.push(value.to_string());
                }
            }
        }
        Ok(())
    }

    let mut keys = Vec::new();
    append_from_env(&mut keys, env_key_name)?;
    let plural_env = format!("{env_key_name}S");
    append_list_from_env(&mut keys, &plural_env)?;

    if provider_slug == "openrouter" {
        append_from_env(&mut keys, "LLM_API_KEY")?;
        append_list_from_env(&mut keys, "LLM_API_KEYS")?;
    }

    Ok(keys)
}

fn create_provider_variants_for_catalog_entry(
    provider_slug: &str,
    model: &str,
    providers_settings: Option<&ProvidersSettings>,
) -> Result<Vec<ProviderLeaseEntry>, LlmError> {
    let endpoint =
        thinclaw_config::provider_catalog::endpoint_for(provider_slug).ok_or_else(|| {
            LlmError::RequestFailed {
                provider: provider_slug.to_string(),
                reason: format!("Unknown provider '{}' in catalog", provider_slug),
            }
        })?;
    let api_keys =
        resolve_catalog_api_keys(provider_slug, &endpoint.env_key_name, providers_settings)?;
    if api_keys.is_empty() {
        return Ok(vec![ProviderLeaseEntry::new(
            create_provider_for_catalog_entry(provider_slug, model)?,
            credential_entry_id(provider_slug, 0),
        )]);
    }

    let mut entries = Vec::with_capacity(api_keys.len());
    for (idx, api_key) in api_keys.iter().enumerate() {
        entries.push(ProviderLeaseEntry::new(
            create_provider_for_catalog_entry_with_api_key(provider_slug, model, Some(api_key))?,
            credential_entry_id(provider_slug, idx),
        ));
    }
    Ok(entries)
}

/// Probe a provider/model pair with a tiny completion before switching to it.
///
/// This catches runtime-only failures such as invalid or revoked model IDs
/// before they poison an active conversation with a broken override.
pub async fn probe_provider_model(provider_slug: &str, model: &str) -> Result<(), LlmError> {
    let provider = create_provider_for_catalog_entry(provider_slug, model)?;
    let request =
        thinclaw_llm_core::CompletionRequest::new(vec![thinclaw_llm_core::ChatMessage::user(
            "Reply with exactly OK.",
        )])
        .with_max_tokens(4)
        .with_temperature(0.0);

    match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        provider.complete(request),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(LlmError::RequestFailed {
            provider: provider_slug.to_string(),
            reason: format!("Timed out while probing model '{}'", model),
        }),
    }
}

/// Create a cheap model provider from a "provider/model" string.
///
/// Used for SmartRoutingProvider's cheap model split.
fn create_cheap_model_provider(
    cheap_model_spec: &str,
    config: &LlmConfig,
    providers_settings: Option<&ProvidersSettings>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    if let Some((provider, model)) = cheap_model_spec.split_once('/') {
        let entries = if thinclaw_config::provider_catalog::endpoint_for(provider).is_some() {
            create_provider_variants_for_catalog_entry(provider, model, providers_settings)?
        } else {
            create_provider_variants_for_non_catalog_slug(provider, model, config)?
        };

        if entries.len() == 1 {
            Ok(Arc::clone(&entries[0].provider))
        } else {
            Ok(Arc::new(FailoverProvider::with_entries(
                entries,
                CooldownConfig::default(),
                LeaseConfig::default(),
            )?))
        }
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

/// Create a Gemini provider via Google's OpenAI-compatible endpoint.
///
/// Google AI Studio provides an OpenAI-compatible gateway at
/// `generativelanguage.googleapis.com/v1beta/openai`. This allows Gemini
/// models to work with the standard RigAdapter without needing a custom
/// HTTP client. The native Gemini adapter in `gemini.rs` remains available
/// for consumers that need the raw Gemini API format.
fn create_gemini_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let gem = config.gemini.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "gemini".to_string(),
    })?;

    let api_key = gem.api_key.as_ref().ok_or_else(|| LlmError::AuthFailed {
        provider: "gemini (GEMINI_API_KEY not set)".to_string(),
    })?;

    use rig::providers::openai;

    let client: openai::CompletionsClient = openai::Client::builder()
        .base_url(&gem.base_url)
        .api_key(api_key.expose_secret())
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "gemini".to_string(),
            reason: format!("Failed to create Gemini OpenAI-compat client: {}", e),
        })?
        .completions_api();

    let model = client.completion_model(&gem.model);
    tracing::info!(
        "Using Google Gemini (model: {}, base_url: {})",
        gem.model,
        gem.base_url,
    );
    Ok(Arc::new(
        RigAdapter::new(model, &gem.model)
            .with_provider_label("gemini")
            .with_token_capture(
                openai_logprob_capture_support(),
                Some(openai_logprob_request_params()),
            ),
    ))
}

/// Create a Bedrock provider using the native OpenAI-compatible Mantle endpoint.
///
/// ThinClaw now prefers Bedrock's native OpenAI-compatible API with
/// `BEDROCK_API_KEY`. A legacy proxy URL remains supported as a fallback.
fn create_bedrock_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let br = config
        .bedrock
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "bedrock".to_string(),
        })?;

    use rig::providers::openai;

    if let Some(api_key) = br.api_key.as_ref() {
        let base_url = crate::discovery::bedrock_mantle_base_url(&br.region);
        let client: openai::CompletionsClient = openai::Client::builder()
            .base_url(&base_url)
            .api_key(api_key.expose_secret())
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "bedrock".to_string(),
                reason: format!("Failed to create Bedrock client: {}", e),
            })?
            .completions_api();

        let model = client.completion_model(&br.model_id);
        tracing::info!(
            "Using AWS Bedrock Mantle (region: {}, base_url: {}, model_id: {})",
            br.region,
            base_url,
            br.model_id,
        );
        Ok(Arc::new(
            RigAdapter::new(model, &br.model_id).with_provider_label("bedrock"),
        ))
    } else if let Some(proxy) = br.proxy_url.clone() {
        let key = br
            .proxy_api_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_string())
            .unwrap_or_else(|| "no-key".to_string());
        let client: openai::CompletionsClient = openai::Client::builder()
            .base_url(&proxy)
            .api_key(&key)
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "bedrock".to_string(),
                reason: format!("Failed to create legacy Bedrock proxy client: {}", e),
            })?
            .completions_api();

        let model = client.completion_model(&br.model_id);
        tracing::info!(
            "Using legacy Bedrock proxy fallback (proxy: {}, model_id: {})",
            proxy,
            br.model_id,
        );
        Ok(Arc::new(
            RigAdapter::new(model, &br.model_id).with_provider_label("bedrock"),
        ))
    } else {
        Err(LlmError::RequestFailed {
            provider: "bedrock".to_string(),
            reason: "BEDROCK_API_KEY must be set to use Bedrock's native OpenAI-compatible Mantle endpoint. \
                     Legacy fallback: configure BEDROCK_PROXY_URL (and optionally BEDROCK_PROXY_API_KEY) \
                     to use an older proxy-based Bedrock gateway."
                .to_string(),
        })
    }
}

/// Create a llama.cpp provider via its OpenAI-compatible HTTP server.
///
/// llama.cpp's `--server` mode (`llama-server`) exposes an OpenAI-compatible
/// Chat Completions API. This function connects to that server.
/// For native FFI-based inference, the trait and types in `llama_cpp.rs`
/// are available when compiled with the `llama-cpp` feature.
fn create_llama_cpp_provider(config: &LlmConfig) -> Result<Arc<dyn LlmProvider>, LlmError> {
    let lc = config
        .llama_cpp
        .as_ref()
        .ok_or_else(|| LlmError::AuthFailed {
            provider: "llama_cpp".to_string(),
        })?;

    use rig::providers::openai;

    // llama.cpp server uses OpenAI-compatible endpoint, no API key required.
    let client: openai::CompletionsClient = openai::Client::builder()
        .base_url(&lc.server_url)
        .api_key("no-key") // llama.cpp server doesn't require auth
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "llama_cpp".to_string(),
            reason: format!("Failed to create llama.cpp client: {}", e),
        })?
        .completions_api();

    let model = client.completion_model(&lc.model);
    tracing::info!(
        "Using llama.cpp server (url: {}, model: {})",
        lc.server_url,
        lc.model,
    );
    Ok(Arc::new(
        RigAdapter::new(model, &lc.model)
            .with_provider_label("llama_cpp")
            .with_token_capture(
                openai_logprob_capture_support(),
                Some(openai_logprob_request_params()),
            ),
    ))
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
    providers_settings: Option<&ProvidersSettings>,
) -> Result<(Arc<dyn LlmProvider>, Option<Arc<dyn LlmProvider>>), LlmError> {
    let rel = &config.reliability;

    let primary_entries = create_primary_providers(config, providers_settings)?;
    let primary = primary_entries
        .first()
        .map(|entry| Arc::clone(&entry.provider))
        .ok_or_else(|| LlmError::RequestFailed {
            provider: "llm".to_string(),
            reason: "No primary providers could be constructed".to_string(),
        })?;
    let primary_model_name = primary.model_name().to_string();
    tracing::info!("Primary LLM provider initialized: {}", primary_model_name);

    // ── 1. Build multi-provider failover chain ───────────────────────────
    let mut all_entries = primary_entries;

    if let Some(ps) = providers_settings {
        // Determine fallback providers from ProvidersSettings.
        // Use explicit fallback_chain if provided, otherwise auto-build
        // from enabled providers.
        let mut fallbacks: Vec<(String, String)> = if !ps.fallback_chain.is_empty() {
            // Explicit chain: parse "provider/model" or "provider@slot" entries
            ps.fallback_chain
                .iter()
                .filter_map(|entry| resolve_fallback_entry(ps, entry))
                .collect()
        } else {
            // Auto-build: use all enabled providers that aren't the primary
            ps.enabled
                .iter()
                .filter(|slug| {
                    // Skip if this is the primary provider
                    ps.primary.as_deref() != Some(slug.as_str())
                })
                .filter_map(|slug| {
                    fallback_model_for_slug(ps, slug).map(|model| (slug.clone(), model))
                })
                .collect()
        };

        if let Some(fallback_model) = rel.fallback_model.as_ref()
            && let Some((provider_slug, model)) = fallback_model.split_once('/')
        {
            let extra = (provider_slug.to_string(), model.to_string());
            if !fallbacks.contains(&extra) {
                tracing::info!("Adding fallback model from env: {}", fallback_model);
                fallbacks.push(extra);
            }
        }

        append_fallbacks(&mut all_entries, &fallbacks, config, providers_settings);
    }

    let llm: Arc<dyn LlmProvider> =
        wrap_failover(primary.clone(), all_entries, rel, providers_settings)?;

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
        match create_cheap_model_provider(spec, config, providers_settings) {
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

    let smart_routing_enabled = providers_settings
        .map(|ps| ps.smart_routing_enabled && ps.routing_mode == RoutingMode::CheapSplit)
        .unwrap_or(cheap_model_spec.is_some());
    let cascade_enabled = providers_settings
        .map(|ps| ps.smart_routing_cascade)
        .unwrap_or(rel.smart_routing_cascade);

    let llm: Arc<dyn LlmProvider> = if smart_routing_enabled {
        if let Some(ref cheap) = cheap_llm {
            tracing::info!("SmartRoutingProvider enabled (primary + cheap model)");
            Arc::new(SmartRoutingProvider::new(
                llm,
                cheap.clone(),
                SmartRoutingConfig {
                    cascade_enabled,
                    ..SmartRoutingConfig::default()
                },
            ))
        } else {
            llm
        }
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

fn create_llm_provider_variants(config: &LlmConfig) -> Result<Vec<ProviderLeaseEntry>, LlmError> {
    match config.backend {
        LlmBackend::OpenAi => {
            let openai = config.openai.as_ref().ok_or_else(|| LlmError::AuthFailed {
                provider: "openai".to_string(),
            })?;
            let keys = resolved_secret_credentials(&openai.api_keys, &openai.api_key);
            if keys.is_empty() {
                return Ok(vec![ProviderLeaseEntry::new(
                    create_openai_provider(config)?,
                    credential_entry_id("openai", 0),
                )]);
            }

            let mut entries = Vec::with_capacity(keys.len());
            for (idx, key) in keys.into_iter().enumerate() {
                let mut variant = config.clone();
                if let Some(openai) = variant.openai.as_mut() {
                    openai.api_key = Some(key.clone());
                    openai.api_keys = vec![key];
                }
                entries.push(ProviderLeaseEntry::new(
                    create_openai_provider(&variant)?,
                    credential_entry_id("openai", idx),
                ));
            }
            Ok(entries)
        }
        LlmBackend::Anthropic => {
            let anthropic = config
                .anthropic
                .as_ref()
                .ok_or_else(|| LlmError::AuthFailed {
                    provider: "anthropic".to_string(),
                })?;
            let keys = resolved_secret_credentials(&anthropic.api_keys, &anthropic.api_key);
            if keys.is_empty() {
                return Ok(vec![ProviderLeaseEntry::new(
                    create_anthropic_provider(config)?,
                    credential_entry_id("anthropic", 0),
                )]);
            }

            let mut entries = Vec::with_capacity(keys.len());
            for (idx, key) in keys.into_iter().enumerate() {
                let mut variant = config.clone();
                if let Some(anthropic) = variant.anthropic.as_mut() {
                    anthropic.api_key = Some(key.clone());
                    anthropic.api_keys = vec![key];
                }
                entries.push(ProviderLeaseEntry::new(
                    create_anthropic_provider(&variant)?,
                    credential_entry_id("anthropic", idx),
                ));
            }
            Ok(entries)
        }
        LlmBackend::OpenAiCompatible => {
            let compat = config
                .openai_compatible
                .as_ref()
                .ok_or_else(|| LlmError::AuthFailed {
                    provider: "openai_compatible".to_string(),
                })?;
            let keys = resolved_secret_credentials(&compat.api_keys, &compat.api_key);
            if keys.is_empty() {
                return Ok(vec![ProviderLeaseEntry::new(
                    create_openai_compatible_provider(config)?,
                    credential_entry_id("openai_compatible", 0),
                )]);
            }

            let mut entries = Vec::with_capacity(keys.len());
            for (idx, key) in keys.into_iter().enumerate() {
                let mut variant = config.clone();
                if let Some(compat) = variant.openai_compatible.as_mut() {
                    compat.api_key = Some(key.clone());
                    compat.api_keys = vec![key];
                }
                entries.push(ProviderLeaseEntry::new(
                    create_openai_compatible_provider(&variant)?,
                    credential_entry_id("openai_compatible", idx),
                ));
            }
            Ok(entries)
        }
        LlmBackend::Tinfoil => {
            let tinfoil = config
                .tinfoil
                .as_ref()
                .ok_or_else(|| LlmError::AuthFailed {
                    provider: "tinfoil".to_string(),
                })?;
            let keys = resolved_secret_credentials(&tinfoil.api_keys, &tinfoil.api_key);
            if keys.is_empty() {
                return Ok(vec![ProviderLeaseEntry::new(
                    create_tinfoil_provider(config)?,
                    credential_entry_id("tinfoil", 0),
                )]);
            }

            let mut entries = Vec::with_capacity(keys.len());
            for (idx, key) in keys.into_iter().enumerate() {
                let mut variant = config.clone();
                if let Some(tinfoil) = variant.tinfoil.as_mut() {
                    tinfoil.api_key = Some(key.clone());
                    tinfoil.api_keys = vec![key];
                }
                entries.push(ProviderLeaseEntry::new(
                    create_tinfoil_provider(&variant)?,
                    credential_entry_id("tinfoil", idx),
                ));
            }
            Ok(entries)
        }
        LlmBackend::Gemini => {
            let gemini = config.gemini.as_ref().ok_or_else(|| LlmError::AuthFailed {
                provider: "gemini".to_string(),
            })?;
            let keys = resolved_secret_credentials(&gemini.api_keys, &gemini.api_key);
            if keys.is_empty() {
                return Ok(vec![ProviderLeaseEntry::new(
                    create_gemini_provider(config)?,
                    credential_entry_id("gemini", 0),
                )]);
            }

            let mut entries = Vec::with_capacity(keys.len());
            for (idx, key) in keys.into_iter().enumerate() {
                let mut variant = config.clone();
                if let Some(gemini) = variant.gemini.as_mut() {
                    gemini.api_key = Some(key.clone());
                    gemini.api_keys = vec![key];
                }
                entries.push(ProviderLeaseEntry::new(
                    create_gemini_provider(&variant)?,
                    credential_entry_id("gemini", idx),
                ));
            }
            Ok(entries)
        }
        LlmBackend::Bedrock => {
            let bedrock = config
                .bedrock
                .as_ref()
                .ok_or_else(|| LlmError::AuthFailed {
                    provider: "bedrock".to_string(),
                })?;
            let keys = resolved_secret_credentials(&bedrock.api_keys, &bedrock.api_key);
            if keys.is_empty() {
                return Ok(vec![ProviderLeaseEntry::new(
                    create_bedrock_provider(config)?,
                    credential_entry_id("bedrock", 0),
                )]);
            }

            let mut entries = Vec::with_capacity(keys.len());
            for (idx, key) in keys.into_iter().enumerate() {
                let mut variant = config.clone();
                if let Some(bedrock) = variant.bedrock.as_mut() {
                    bedrock.api_key = Some(key.clone());
                    bedrock.api_keys = vec![key];
                }
                entries.push(ProviderLeaseEntry::new(
                    create_bedrock_provider(&variant)?,
                    credential_entry_id("bedrock", idx),
                ));
            }
            Ok(entries)
        }
        LlmBackend::Ollama => Ok(vec![ProviderLeaseEntry::new(
            create_ollama_provider(config)?,
            credential_entry_id("ollama", 0),
        )]),
        LlmBackend::LlamaCpp => Ok(vec![ProviderLeaseEntry::new(
            create_llama_cpp_provider(config)?,
            credential_entry_id("llama_cpp", 0),
        )]),
    }
}

fn create_primary_providers(
    config: &LlmConfig,
    providers_settings: Option<&ProvidersSettings>,
) -> Result<Vec<ProviderLeaseEntry>, LlmError> {
    if let Some(ps) = providers_settings
        && let Some(primary_slug) = ps.primary.as_deref()
    {
        let model = provider_primary_model_for_slug(ps, primary_slug)
            .or_else(|| ps.primary_model.clone())
            .or_else(|| {
                thinclaw_config::provider_catalog::endpoint_for(primary_slug)
                    .map(|endpoint| endpoint.default_model.to_string())
            });

        if let Some(model) = model {
            if thinclaw_config::provider_catalog::endpoint_for(primary_slug).is_some() {
                return create_provider_variants_for_catalog_entry(
                    primary_slug,
                    &model,
                    providers_settings,
                );
            }
            return create_provider_variants_for_non_catalog_slug(primary_slug, &model, config);
        }
    }

    create_llm_provider_variants(config)
}

fn create_provider_variants_for_non_catalog_slug(
    provider_slug: &str,
    model: &str,
    config: &LlmConfig,
) -> Result<Vec<ProviderLeaseEntry>, LlmError> {
    let mut llm_config = config.clone();
    llm_config.backend = match provider_slug {
        "ollama" => LlmBackend::Ollama,
        "openai_compatible" => LlmBackend::OpenAiCompatible,
        "bedrock" => LlmBackend::Bedrock,
        "llama_cpp" => LlmBackend::LlamaCpp,
        other => {
            return Err(LlmError::RequestFailed {
                provider: other.to_string(),
                reason: "Unsupported non-catalog provider slug".to_string(),
            });
        }
    };

    match llm_config.backend {
        LlmBackend::Ollama => {
            if let Some(ref mut ollama) = llm_config.ollama {
                ollama.model = model.to_string();
            }
        }
        LlmBackend::OpenAiCompatible => {
            if let Some(ref mut compat) = llm_config.openai_compatible {
                compat.model = model.to_string();
            }
        }
        LlmBackend::Bedrock => {
            if let Some(ref mut bedrock) = llm_config.bedrock {
                bedrock.model_id = model.to_string();
            }
        }
        LlmBackend::LlamaCpp => {
            if let Some(ref mut llama_cpp) = llm_config.llama_cpp {
                llama_cpp.model = model.to_string();
            }
        }
        _ => {}
    }

    create_llm_provider_variants(&llm_config)
}

fn append_fallbacks(
    all_providers: &mut Vec<ProviderLeaseEntry>,
    fallbacks: &[(String, String)],
    config: &LlmConfig,
    providers_settings: Option<&ProvidersSettings>,
) {
    for (provider_slug, model) in fallbacks {
        let provider = if thinclaw_config::provider_catalog::endpoint_for(provider_slug).is_some() {
            create_provider_variants_for_catalog_entry(provider_slug, model, providers_settings)
        } else {
            create_provider_variants_for_non_catalog_slug(provider_slug, model, config)
        };

        match provider {
            Ok(mut providers) => {
                tracing::info!(
                    "Failover provider added: '{}' (model: {}, credentials: {})",
                    provider_slug,
                    model,
                    providers.len()
                );
                all_providers.append(&mut providers);
            }
            Err(e) => {
                tracing::warn!("Skipping fallback provider '{}': {}", provider_slug, e);
            }
        }
    }
}

fn fallback_model_for_slug(ps: &ProvidersSettings, slug: &str) -> Option<String> {
    provider_primary_model_for_slug(ps, slug).or_else(|| {
        thinclaw_config::provider_catalog::endpoint_for(slug)
            .map(|endpoint| endpoint.default_model.to_string())
    })
}

fn provider_primary_model_for_slug(ps: &ProvidersSettings, slug: &str) -> Option<String> {
    ps.provider_models
        .get(slug)
        .and_then(|slots| slots.primary.clone())
        .or_else(|| {
            if ps.primary.as_deref() == Some(slug) {
                ps.primary_model.clone()
            } else {
                ps.allowed_models
                    .get(slug)
                    .and_then(|models| models.first().cloned())
            }
        })
        .or_else(|| match slug {
            "ollama" => Some("llama3".to_string()),
            "openai_compatible" => Some("default".to_string()),
            "bedrock" => Some("anthropic.claude-3-sonnet-20240229-v1:0".to_string()),
            "llama_cpp" => Some("llama-local".to_string()),
            _ => None,
        })
}

fn provider_cheap_model_for_slug(ps: &ProvidersSettings, slug: &str) -> Option<String> {
    ps.provider_models
        .get(slug)
        .and_then(|slots| slots.cheap.clone())
        .or_else(|| {
            ps.cheap_model
                .as_deref()
                .and_then(|spec| spec.split_once('/'))
                .and_then(|(cheap_slug, model)| {
                    if cheap_slug == slug {
                        Some(model.to_string())
                    } else {
                        None
                    }
                })
        })
        .or_else(|| provider_primary_model_for_slug(ps, slug))
}

fn resolve_fallback_entry(ps: &ProvidersSettings, entry: &str) -> Option<(String, String)> {
    if let Some((provider, model)) = entry.split_once('/') {
        return Some((provider.to_string(), model.to_string()));
    }
    if let Some(provider) = entry.strip_suffix("@primary") {
        return provider_primary_model_for_slug(ps, provider)
            .map(|model| (provider.to_string(), model));
    }
    if let Some(provider) = entry.strip_suffix("@cheap") {
        return provider_cheap_model_for_slug(ps, provider)
            .map(|model| (provider.to_string(), model));
    }
    None
}

fn wrap_failover(
    primary: Arc<dyn LlmProvider>,
    all_providers: Vec<ProviderLeaseEntry>,
    rel: &thinclaw_config::ReliabilityConfig,
    providers_settings: Option<&ProvidersSettings>,
) -> Result<Arc<dyn LlmProvider>, LlmError> {
    if all_providers.len() > 1 {
        let cooldown = CooldownConfig {
            failure_threshold: rel.failover_cooldown_threshold,
            cooldown_duration: std::time::Duration::from_secs(rel.failover_cooldown_secs),
        };
        let lease = providers_settings
            .map(|ps| LeaseConfig {
                max_concurrent: ps.credential_max_concurrent.max(1),
                selection_strategy: match ps.credential_selection_strategy {
                    CredentialSelectionStrategy::FillFirst => LeaseSelectionStrategy::FillFirst,
                    CredentialSelectionStrategy::RoundRobin => LeaseSelectionStrategy::RoundRobin,
                    CredentialSelectionStrategy::LeastUsed => LeaseSelectionStrategy::LeastUsed,
                    CredentialSelectionStrategy::Random => LeaseSelectionStrategy::Random,
                },
            })
            .unwrap_or_default();
        tracing::info!(
            "FailoverProvider enabled with {} credential entries (cooldown: {}s, threshold: {}, lease cap: {}, selection: {:?})",
            all_providers.len(),
            rel.failover_cooldown_secs,
            rel.failover_cooldown_threshold,
            lease.max_concurrent,
            lease.selection_strategy,
        );
        Ok(Arc::new(FailoverProvider::with_entries(
            all_providers,
            cooldown,
            lease,
        )?))
    } else {
        Ok(primary)
    }
}
