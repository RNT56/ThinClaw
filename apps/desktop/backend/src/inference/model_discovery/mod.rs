//! Shared model/provider registry and cloud model discovery.
//!
//! ## Architecture
//!
//! ```text
//!   ModelProviderRegistry (Tauri managed state)
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
//! Provider metadata, model discovery, local-model inventory, and provider-key
//! readiness all flow through one cloneable registry. Clones share the same
//! cache and app-wide `SecretStore`; `InferenceRouter` owns a clone of this
//! service rather than a second key-vault path.

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
use tauri::AppHandle;
use tokio::sync::RwLock;

use super::{BackendInfo, Modality};
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

/// Non-chat providers that are not part of the LLM endpoint catalog.
#[derive(Clone, Copy)]
struct SupplementalProvider {
    slug: &'static str,
    display_name: &'static str,
    secret_name: &'static str,
}

const SUPPLEMENTAL_PROVIDERS: &[SupplementalProvider] = &[
    SupplementalProvider {
        slug: "voyage",
        display_name: "Voyage AI",
        secret_name: "voyage",
    },
    SupplementalProvider {
        slug: "elevenlabs",
        display_name: "ElevenLabs",
        secret_name: "elevenlabs",
    },
    SupplementalProvider {
        slug: "deepgram",
        display_name: "Deepgram",
        secret_name: "deepgram",
    },
    SupplementalProvider {
        slug: "stability",
        display_name: "Stability AI",
        secret_name: "stability",
    },
    SupplementalProvider {
        slug: "fal",
        display_name: "fal.ai",
        secret_name: "fal",
    },
];

const EMBEDDING_PROVIDERS: &[&str] = &["openai", "gemini", "voyage", "cohere"];
const TTS_PROVIDERS: &[&str] = &["openai", "elevenlabs", "gemini"];
const STT_PROVIDERS: &[&str] = &["openai", "gemini", "deepgram"];
const DIFFUSION_PROVIDERS: &[&str] = &["openai", "gemini", "stability", "fal", "together"];

/// Providers with an implemented cloud-model discoverer. The boolean marks
/// static registries that do not require a provider key.
const DISCOVERY_PROVIDERS: &[(&str, bool)] = &[
    ("openai", false),
    ("anthropic", false),
    ("gemini", false),
    ("groq", false),
    ("openrouter", false),
    ("mistral", false),
    ("xai", false),
    ("together", false),
    ("cohere", false),
    ("elevenlabs", false),
    ("stability", false),
    ("deepgram", true),
    ("voyage", true),
    ("fal", true),
];

fn supplemental_provider(slug: &str) -> Option<SupplementalProvider> {
    SUPPLEMENTAL_PROVIDERS
        .iter()
        .copied()
        .find(|provider| provider.slug == slug)
}

fn provider_display_name(slug: &str) -> Option<&'static str> {
    thinclaw_config::provider_catalog::endpoint_for(slug)
        .map(|endpoint| endpoint.display_name.as_str())
        .or_else(|| supplemental_provider(slug).map(|provider| provider.display_name))
}

fn provider_secret_name(slug: &str) -> Option<&'static str> {
    thinclaw_config::provider_catalog::endpoint_for(slug)
        .map(|endpoint| endpoint.secret_name.as_str())
        .or_else(|| supplemental_provider(slug).map(|provider| provider.secret_name))
}

fn modality_provider_ids(modality: Modality) -> &'static [&'static str] {
    match modality {
        Modality::Chat => &[],
        Modality::Embedding => EMBEDDING_PROVIDERS,
        Modality::Tts => TTS_PROVIDERS,
        Modality::Stt => STT_PROVIDERS,
        Modality::Diffusion => DIFFUSION_PROVIDERS,
    }
}

fn provider_ids_for_modality(modality: Modality) -> Vec<&'static str> {
    let mut provider_ids = if modality == Modality::Chat {
        thinclaw_config::provider_catalog::all_provider_ids()
    } else {
        modality_provider_ids(modality).to_vec()
    };
    provider_ids.sort_unstable();
    provider_ids
}

/// App-wide registry for provider metadata, provider keys, and local/cloud models.
///
/// Managed as Tauri state and cloned into `InferenceRouter`. Every clone shares
/// the same discovery cache and `SecretStore`.
#[derive(Clone)]
pub struct ModelProviderRegistry {
    cache: Arc<RwLock<HashMap<String, CachedDiscovery>>>,
    secret_store: Arc<SecretStore>,
}

impl ModelProviderRegistry {
    pub fn new(secret_store: Arc<SecretStore>) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            secret_store,
        }
    }

    /// Read a provider key through the canonical descriptor/alias contract.
    pub fn provider_secret(&self, provider: &str) -> Option<String> {
        let secret_name = provider_secret_name(provider).unwrap_or(provider);
        descriptor_secret(&self.secret_store, secret_name)
            .or_else(|| {
                (secret_name != provider)
                    .then(|| descriptor_secret(&self.secret_store, provider))
                    .flatten()
            })
            .filter(|value| !value.trim().is_empty())
    }

    pub fn has_provider_secret(&self, provider: &str) -> bool {
        self.provider_secret(provider).is_some()
    }

    pub fn secret_store(&self) -> &SecretStore {
        &self.secret_store
    }

    /// List every configured backend for a modality from one metadata source.
    pub fn available_backends_for(&self, modality: Modality) -> Vec<BackendInfo> {
        let local_display_name = match modality {
            Modality::Chat => "Local (llama.cpp / MLX)",
            Modality::Embedding => "Local (llama-server / mlx-embed)",
            Modality::Tts => "Local (Piper)",
            Modality::Stt => "Local (Whisper)",
            Modality::Diffusion => "Local (sd.cpp / mflux)",
        };
        let mut backends = vec![BackendInfo {
            id: "local".to_string(),
            display_name: local_display_name.to_string(),
            is_local: true,
            model_id: None,
            available: true,
        }];

        backends.extend(
            provider_ids_for_modality(modality)
                .into_iter()
                .filter_map(|provider| {
                    Some(BackendInfo {
                        id: provider.to_string(),
                        display_name: provider_display_name(provider)?.to_string(),
                        is_local: false,
                        model_id: None,
                        available: self.has_provider_secret(provider),
                    })
                }),
        );
        backends
    }

    /// Scan the local model inventory through the same shared registry seam.
    pub async fn list_local_models(
        &self,
        app: &AppHandle,
    ) -> Result<Vec<crate::model_manager::ModelFile>, String> {
        crate::model_manager::list_models(app.clone())
            .await
            .map_err(Into::into)
    }

    /// Snapshot the active local runtime for both Desktop product modes.
    pub async fn local_runtime_snapshot(
        &self,
        sidecar: &crate::sidecar::SidecarManager,
        engine_manager: &crate::engine::EngineManager,
    ) -> thinclaw_runtime_contracts::LocalRuntimeSnapshot {
        crate::engine::local_runtime_snapshot(sidecar, engine_manager).await
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
            let registry = self.clone();

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
                discover_for_provider(&provider, &registry).await
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
        let result = discover_for_provider(provider, self).await;

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
        DISCOVERY_PROVIDERS
            .iter()
            .filter(|(provider, is_static)| *is_static || self.has_provider_secret(provider))
            .map(|(provider, _)| (*provider).to_string())
            .collect()
    }
}

/// Dispatch discovery to the appropriate provider module.
async fn discover_for_provider(
    provider: &str,
    registry: &ModelProviderRegistry,
) -> ProviderDiscoveryResult {
    let api_key = registry.provider_secret(provider);

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

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn registry() -> ModelProviderRegistry {
        ModelProviderRegistry::new(Arc::new(SecretStore::new()))
    }

    #[test]
    fn registry_clones_share_cache_and_key_vault() {
        let registry = registry();
        let clone = registry.clone();
        assert!(Arc::ptr_eq(&registry.cache, &clone.cache));
        assert!(Arc::ptr_eq(&registry.secret_store, &clone.secret_store));
    }

    #[test]
    fn chat_provider_ids_come_from_provider_catalog() {
        let provider_ids = provider_ids_for_modality(Modality::Chat);
        let ids: HashSet<_> = provider_ids.iter().copied().collect();

        assert_eq!(
            provider_ids.len(),
            thinclaw_config::provider_catalog::catalog().len()
        );
        for provider in thinclaw_config::provider_catalog::all_provider_ids() {
            assert!(
                ids.contains(provider),
                "missing catalog provider {provider}"
            );
        }
    }

    #[test]
    fn modality_backend_lists_are_duplicate_free() {
        for modality in [
            Modality::Chat,
            Modality::Embedding,
            Modality::Tts,
            Modality::Stt,
            Modality::Diffusion,
        ] {
            let provider_ids = provider_ids_for_modality(modality);
            let ids: HashSet<_> = provider_ids.iter().copied().collect();
            assert_eq!(
                ids.len(),
                provider_ids.len(),
                "duplicate {modality} backend"
            );
        }
    }

    #[test]
    fn every_direct_backend_has_provider_metadata() {
        for modality in [
            Modality::Embedding,
            Modality::Tts,
            Modality::Stt,
            Modality::Diffusion,
        ] {
            for provider in modality_provider_ids(modality) {
                assert!(
                    provider_display_name(provider).is_some(),
                    "missing display name for {provider}"
                );
                assert!(
                    provider_secret_name(provider).is_some(),
                    "missing secret name for {provider}"
                );
            }
        }
    }
}
