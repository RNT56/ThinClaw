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
        let mut total_models = 0;
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
                        total_models += entry.models.len();
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
                    total_models += result.models.len();
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
                matches!(*p, &"deepgram" | &"voyage" | &"fal") || self.secret_store.get(p).is_some()
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
    let api_key = secret_store.get(provider);

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
        Ok(models) => ProviderDiscoveryResult {
            provider: provider.to_string(),
            models,
            from_cache: false,
            error: None,
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
