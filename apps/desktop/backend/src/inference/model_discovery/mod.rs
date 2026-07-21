//! Cloud model discovery — queries provider APIs for available models.
//!
//! ## Architecture
//!
//! ```text
//!   CloudModelRegistry (Tauri managed state)
//!   │
//!   ├── discover(providers)  ── for each provider with key ──┐
//!   │                                                         │
//!   │   ┌─ openai::discover() ──────────────────────────┐    │
//!   │   ├─ anthropic::discover() ───────────────────────┤    │
//!   │   ├─ gemini::discover() ──────────────────────────┤    │
//!   │   ├─ groq::discover() ───────────────────────────┤    │
//!   │   ├─ ...(12 providers)...                         │    │
//!   │   └─ static_registry::discover() ────────────────┘    │
//!   │                                                         │
//!   │   ←── Vec<CloudModelEntry> (cached 30 min) ────────────┘
//!   │
//!   └── get_cached(provider) ── read from cache
//! ```
//!
//! ## Key Constraint
//!
//! All API keys come from `SecretStore`, same as `InferenceRouter`.

pub mod types;

// Provider discovery modules
pub mod anthropic;
pub mod cohere;
pub mod elevenlabs;
pub mod gemini;
pub mod groq;
pub mod mistral;
pub mod openai;
pub mod openrouter;
pub mod stability;
pub mod static_registry;
pub mod together;
pub mod xai;

pub mod classifier;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::secret_store::SecretStore;
use types::*;

/// Cache TTL — models change infrequently, so 30 min is plenty.
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const MODEL_DISCOVERY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MODEL_DISCOVERY_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MODEL_DISCOVERY_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_MODEL_DISCOVERY_ERROR_BYTES: usize = 16 * 1024;
const MAX_DISCOVERED_MODELS: usize = 4_096;
const MAX_MODEL_FIELD_BYTES: usize = 512;
const MAX_MODEL_METADATA_ENTRIES: usize = 32;
const MAX_MODEL_METADATA_BYTES: usize = 64 * 1024;

pub(super) fn http_client(api_key: &str) -> Result<reqwest::Client, String> {
    if api_key.is_empty() || api_key.len() > 4_096 || api_key.chars().any(char::is_control) {
        return Err("The provider API credential is missing or invalid".to_string());
    }

    reqwest::Client::builder()
        .connect_timeout(MODEL_DISCOVERY_CONNECT_TIMEOUT)
        .timeout(MODEL_DISCOVERY_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not build model discovery HTTP client: {error}"))
}

fn safe_error_excerpt(text: &str) -> String {
    let mut excerpt = String::with_capacity(text.len().min(2_048));
    for character in text.chars() {
        if excerpt.len() >= 2_048 {
            break;
        }
        if !character.is_control() || matches!(character, '\n' | '\r' | '\t') {
            excerpt.push(character);
        }
    }
    excerpt.trim().to_string()
}

pub(super) async fn bounded_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    provider: &str,
) -> Result<T, String> {
    let status = response.status();
    if !status.is_success() {
        if matches!(status.as_u16(), 401 | 403) {
            return Err(format!(
                "{provider} rejected the configured API credential (HTTP {status})"
            ));
        }
        let detail =
            thinclaw_core::http_response::bounded_text(response, MAX_MODEL_DISCOVERY_ERROR_BYTES)
                .await
                .ok()
                .map(|text| safe_error_excerpt(&text))
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| "no bounded error detail".to_string());
        return Err(format!("{provider} API error (HTTP {status}): {detail}"));
    }

    thinclaw_core::http_response::bounded_json(response, MAX_MODEL_DISCOVERY_RESPONSE_BYTES)
        .await
        .map_err(|error| format!("Invalid bounded {provider} model response: {error}"))
}

fn valid_model_field(value: &str, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= MAX_MODEL_FIELD_BYTES
        && !value.chars().any(char::is_control)
}

fn validate_pricing(pricing: &ModelPricing) -> bool {
    [
        pricing.input_per_million,
        pricing.output_per_million,
        pricing.per_image,
        pricing.per_minute,
        pricing.per_1k_chars,
    ]
    .into_iter()
    .flatten()
    .all(|value| value.is_finite() && (0.0..=1_000_000_000.0).contains(&value))
}

pub(crate) fn validate_models(
    expected_provider: &str,
    models: Vec<CloudModelEntry>,
) -> Result<Vec<CloudModelEntry>, String> {
    if models.len() > MAX_DISCOVERED_MODELS {
        return Err(format!(
            "{expected_provider} returned more than {MAX_DISCOVERED_MODELS} models"
        ));
    }

    for model in &models {
        if !valid_model_field(&model.id, false)
            || !valid_model_field(&model.display_name, false)
            || !valid_model_field(&model.provider, false)
            || !valid_model_field(&model.provider_name, false)
            || model.provider != expected_provider
        {
            return Err(format!(
                "{expected_provider} returned invalid model identity metadata"
            ));
        }
        if model.embedding_dimensions == Some(0)
            || model
                .embedding_dimensions
                .is_some_and(|value| value > 1_000_000)
            || model
                .pricing
                .as_ref()
                .is_some_and(|pricing| !validate_pricing(pricing))
            || model.metadata.len() > MAX_MODEL_METADATA_ENTRIES
        {
            return Err(format!(
                "{expected_provider} returned invalid model capability metadata"
            ));
        }
        let metadata_bytes = model
            .metadata
            .iter()
            .try_fold(0_usize, |total, (key, value)| {
                if !valid_model_field(key, false)
                    || value.len() > MAX_MODEL_METADATA_BYTES
                    || value.chars().any(char::is_control)
                {
                    return None;
                }
                total.checked_add(key.len())?.checked_add(value.len())
            });
        if metadata_bytes.is_none_or(|bytes| bytes > MAX_MODEL_METADATA_BYTES) {
            return Err(format!(
                "{expected_provider} returned oversized model metadata"
            ));
        }
    }

    Ok(models)
}

/// Internal cache entry for a provider.
struct CachedDiscovery {
    models: Vec<CloudModelEntry>,
    fetched_at: Instant,
    error: Option<String>,
}

impl CachedDiscovery {
    fn is_expired(&self) -> bool {
        self.fetched_at.elapsed() > CACHE_TTL
    }
}

/// Central registry for cloud-discovered models.
///
/// Managed as Tauri state alongside `InferenceRouter`.
pub struct CloudModelRegistry {
    cache: RwLock<HashMap<String, CachedDiscovery>>,
    secret_store: Arc<SecretStore>,
}

impl CloudModelRegistry {
    pub fn new(secret_store: Arc<SecretStore>) -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
            secret_store,
        }
    }

    /// Discover models from all providers that have API keys configured.
    ///
    /// If `providers` is empty, discovers from ALL providers with keys.
    /// Results are cached; fresh calls are only made for expired or missing entries.
    pub async fn discover(&self, providers: Vec<String>) -> DiscoveryResult {
        let providers = if providers.is_empty() {
            self.available_providers()
        } else {
            providers
        };

        let mut results = Vec::new();
        let mut total_models: u32 = 0;
        let mut errors = Vec::new();

        // Discover in parallel
        let mut handles = Vec::new();
        for provider in &providers {
            let provider = provider.clone();
            let secret_store = self.secret_store.clone();

            // Check if we have a fresh cache
            {
                let cache = self.cache.read().await;
                if let Some(entry) = cache.get(&provider) {
                    if !entry.is_expired() {
                        results.push(ProviderDiscoveryResult {
                            provider: provider.clone(),
                            models: entry.models.clone(),
                            from_cache: true,
                            error: entry.error.clone(),
                        });
                        total_models += entry.models.len() as u32;
                        continue;
                    }
                }
            }

            // Need fresh discovery
            handles.push(tokio::spawn(async move {
                discover_for_provider(&provider, &secret_store).await
            }));
        }

        // Collect parallel results
        for handle in handles {
            match handle.await {
                Ok(result) => {
                    total_models += result.models.len() as u32;
                    if let Some(ref err) = result.error {
                        errors.push(format!("{}: {}", result.provider, err));
                    }

                    // Cache the result
                    {
                        let mut cache = self.cache.write().await;
                        cache.insert(
                            result.provider.clone(),
                            CachedDiscovery {
                                models: result.models.clone(),
                                fetched_at: Instant::now(),
                                error: result.error.clone(),
                            },
                        );
                    }

                    results.push(result);
                }
                Err(e) => {
                    errors.push(format!("Task panic: {}", e));
                }
            }
        }

        DiscoveryResult {
            providers: results,
            total_models,
            errors,
        }
    }

    /// Refresh models for a single provider (ignoring cache).
    pub async fn refresh(&self, provider: &str) -> ProviderDiscoveryResult {
        let result = discover_for_provider(provider, &self.secret_store).await;

        // Update cache
        let mut cache = self.cache.write().await;
        cache.insert(
            provider.to_string(),
            CachedDiscovery {
                models: result.models.clone(),
                fetched_at: Instant::now(),
                error: result.error.clone(),
            },
        );

        result
    }

    /// Get cached models for a provider (returns None if not cached).
    pub async fn get_cached(&self, provider: &str) -> Option<Vec<CloudModelEntry>> {
        let cache = self.cache.read().await;
        cache.get(provider).map(|e| e.models.clone())
    }

    /// All providers that have API keys configured in SecretStore.
    fn available_providers(&self) -> Vec<String> {
        // Check each known provider for a key
        let known = [
            "openai",
            "anthropic",
            "gemini",
            "groq",
            "openrouter",
            "mistral",
            "xai",
            "together",
            "cohere",
            "elevenlabs",
            "stability",
            // Static registries (no key needed, always included)
            "deepgram",
            "voyage",
            "fal",
        ];

        known
            .iter()
            .filter(|p| {
                // Static providers are always available
                matches!(*p, &"deepgram" | &"voyage" | &"fal")
                    || descriptor_secret(&self.secret_store, p).is_some()
            })
            .map(|s| s.to_string())
            .collect()
    }
}

/// Dispatch discovery to the appropriate provider module.
async fn discover_for_provider(
    provider: &str,
    secret_store: &SecretStore,
) -> ProviderDiscoveryResult {
    let api_key = descriptor_secret(secret_store, provider);

    let result = match provider {
        "openai" => {
            if let Some(key) = api_key {
                openai::discover(&key).await
            } else {
                Err("No OpenAI API key".into())
            }
        }
        "anthropic" => {
            if let Some(key) = api_key {
                anthropic::discover(&key).await
            } else {
                Err("No Anthropic API key".into())
            }
        }
        "gemini" => {
            if let Some(key) = api_key {
                gemini::discover(&key).await
            } else {
                Err("No Gemini API key".into())
            }
        }
        "groq" => {
            if let Some(key) = api_key {
                groq::discover(&key).await
            } else {
                Err("No Groq API key".into())
            }
        }
        "openrouter" => {
            if let Some(key) = api_key {
                openrouter::discover(&key).await
            } else {
                Err("No OpenRouter API key".into())
            }
        }
        "mistral" => {
            if let Some(key) = api_key {
                mistral::discover(&key).await
            } else {
                Err("No Mistral API key".into())
            }
        }
        "xai" => {
            if let Some(key) = api_key {
                xai::discover(&key).await
            } else {
                Err("No xAI API key".into())
            }
        }
        "together" => {
            if let Some(key) = api_key {
                together::discover(&key).await
            } else {
                Err("No Together API key".into())
            }
        }
        "cohere" => {
            if let Some(key) = api_key {
                cohere::discover(&key).await
            } else {
                Err("No Cohere API key".into())
            }
        }
        "elevenlabs" => {
            if let Some(key) = api_key {
                elevenlabs::discover(&key).await
            } else {
                Err("No ElevenLabs API key".into())
            }
        }
        "stability" => {
            if let Some(key) = api_key {
                stability::discover(&key).await
            } else {
                Err("No Stability AI API key".into())
            }
        }
        // Static registries (no API key needed)
        "deepgram" | "voyage" | "fal" => Ok(static_registry::discover(provider)),
        _ => Err(format!("Unknown provider: {}", provider)),
    };

    match result {
        Ok(models) => match validate_models(provider, models) {
            Ok(models) => ProviderDiscoveryResult {
                provider: provider.to_string(),
                models,
                from_cache: false,
                error: None,
            },
            Err(error) => ProviderDiscoveryResult {
                provider: provider.to_string(),
                models: vec![],
                from_cache: false,
                error: Some(error),
            },
        },
        Err(e) => {
            tracing::warn!("[model_discovery] Failed to discover {}: {}", provider, e);
            ProviderDiscoveryResult {
                provider: provider.to_string(),
                models: vec![],
                from_cache: false,
                error: Some(e),
            }
        }
    }
}

fn descriptor_secret(secret_store: &SecretStore, name: &str) -> Option<String> {
    thinclaw_runtime_contracts::descriptor_for_secret_name(name)
        .and_then(|descriptor| secret_store.get_descriptor_secret(&descriptor))
        .or_else(|| secret_store.get(name))
}
