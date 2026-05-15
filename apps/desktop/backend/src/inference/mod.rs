//! Inference routing layer.
//!
//! `InferenceRouter` is the central abstraction that routes every AI modality
//! (chat, embedding, TTS, STT, diffusion) to either local sidecars or cloud
//! provider APIs.  Each modality has its own backend trait and can be
//! configured independently.
//!
//! ## Architecture
//!
//! ```text
//!                         ┌─────────────────────────┐
//!                         │    InferenceRouter       │
//!                         │  (Tauri managed state)   │
//!                         └────────┬────────────────┘
//!          ┌──────┬──────┬────────┼──────┬──────────┐
//!          ▼      ▼      ▼        ▼      ▼          ▼
//!        Chat  Embed   TTS      STT  Diffusion  Discovery
//!         │      │      │        │      │          │
//!     ┌───┴──┐ ┌─┴─┐  ┌─┴─┐  ┌──┴──┐ ┌─┴──┐   ┌──┴──┐
//!     │Local │ │Lo │  │Lo │  │ Lo  │ │Lo  │   │Live │
//!     │Cloud │ │Cld│  │Cld│  │ Cld │ │Cld │   │APIs │
//!     └──────┘ └───┘  └───┘  └─────┘ └────┘   └─────┘
//! ```
//!
//! ## Key Constraint
//!
//! All API keys are read from `SecretStore` (live reads from keychain cache),
//! never from `OpenClawConfig` (which is a stale snapshot).

pub mod chat;
pub mod diffusion;
pub mod embedding;
pub mod model_discovery;
pub mod provider_endpoints;
pub mod router;
pub mod stt;
pub mod tts;

use std::collections::{HashMap, HashSet};

pub use model_discovery::CloudModelRegistry;
pub use router::InferenceRouter;

use serde::{Deserialize, Serialize};
use specta::Type;

// ─────────────────────────────────────────────────────────────────────────────
// Shared types
// ─────────────────────────────────────────────────────────────────────────────

/// Information about an active or available inference backend.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BackendInfo {
    /// Machine-readable identifier, e.g. `"local"`, `"openai"`, `"gemini"`.
    pub id: String,
    /// Human-readable display name, e.g. `"OpenAI"`, `"Local (llama.cpp)"`.
    pub display_name: String,
    /// Whether this backend runs locally (no network needed).
    pub is_local: bool,
    /// Currently active model identifier, if any.
    pub model_id: Option<String>,
    /// Whether the backend is currently available (has API key, server running, etc.).
    pub available: bool,
}

/// Which AI modality a backend serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Chat,
    Embedding,
    Tts,
    Stt,
    Diffusion,
}

impl std::fmt::Display for Modality {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Embedding => write!(f, "embedding"),
            Self::Tts => write!(f, "tts"),
            Self::Stt => write!(f, "stt"),
            Self::Diffusion => write!(f, "diffusion"),
        }
    }
}

/// Audio format used by TTS/STT backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    /// Raw PCM (16-bit signed, mono).
    Pcm,
    /// MP3 encoded.
    Mp3,
    /// WAV container.
    Wav,
    /// Opus encoded.
    Opus,
}

/// Voice metadata for TTS backends.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct VoiceInfo {
    /// Machine-readable ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Optional language/locale code.
    pub language: Option<String>,
    /// Optional gender hint.
    pub gender: Option<String>,
    /// Whether this is the default voice.
    pub is_default: bool,
}

/// Result type used by all backend trait methods.
pub type InferenceResult<T> = Result<T, InferenceError>;

/// Error type for inference operations.
#[derive(Debug, Clone)]
pub struct InferenceError {
    pub message: String,
    pub kind: InferenceErrorKind,
}

/// Categorized inference error kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InferenceErrorKind {
    /// API key missing or invalid.
    AuthError,
    /// Provider returned a rate limit error.
    RateLimited,
    /// Network or connection error.
    NetworkError,
    /// Model not available or not found.
    ModelNotFound,
    /// The local server is not running.
    ServerNotRunning,
    /// Provider returned an unexpected response.
    ProviderError,
    /// Configuration error (wrong settings, etc.).
    ConfigError,
    /// Generic / uncategorized error.
    Other,
}

impl std::fmt::Display for InferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for InferenceError {}

impl InferenceError {
    pub fn auth(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            kind: InferenceErrorKind::AuthError,
        }
    }

    pub fn network(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            kind: InferenceErrorKind::NetworkError,
        }
    }

    pub fn server_not_running(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            kind: InferenceErrorKind::ServerNotRunning,
        }
    }

    pub fn provider(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            kind: InferenceErrorKind::ProviderError,
        }
    }

    pub fn config(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            kind: InferenceErrorKind::ConfigError,
        }
    }

    pub fn model_not_found(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            kind: InferenceErrorKind::ModelNotFound,
        }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            kind: InferenceErrorKind::Other,
        }
    }
}

/// Convert InferenceError to a user-facing String for Tauri commands.
impl From<InferenceError> for String {
    fn from(err: InferenceError) -> String {
        err.message
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri Commands
// ─────────────────────────────────────────────────────────────────────────────

/// Per-modality backend status returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModalityBackends {
    pub modality: Modality,
    /// The currently active backend (None if nothing configured).
    pub active: Option<BackendInfo>,
    /// All backends that COULD be activated (including local + cloud with keys).
    pub available: Vec<BackendInfo>,
}

/// Returns the active and available backends for all 5 modalities.
#[tauri::command]
#[specta::specta]
pub async fn get_inference_backends(
    router: tauri::State<'_, InferenceRouter>,
) -> Result<Vec<ModalityBackends>, String> {
    let active_list = router.active_backends().await;
    let mut result = Vec::with_capacity(5);

    for (modality, active) in &active_list {
        result.push(ModalityBackends {
            modality: *modality,
            active: active.clone(),
            available: router.available_backends_for(*modality),
        });
    }

    Ok(result)
}

/// Hot-swap the active backend for a given modality.
///
/// This updates the user config and then reconfigures the router, which
/// will construct the appropriate cloud backend immediately (API-key-only
/// providers) or mark local backends for deferred construction.
#[tauri::command]
#[specta::specta]
pub async fn update_inference_backend(
    app: tauri::AppHandle,
    router: tauri::State<'_, InferenceRouter>,
    config_manager: tauri::State<'_, crate::config::ConfigManager>,
    modality: Modality,
    backend_id: String,
) -> Result<(), String> {
    tracing::info!(
        "[inference] Updating {} backend to '{}'",
        modality,
        backend_id
    );

    // Update the config first
    let mut config = config_manager.get_config();
    match modality {
        Modality::Chat => config.chat_backend = Some(backend_id.clone()),
        Modality::Embedding => config.embedding_backend = Some(backend_id.clone()),
        Modality::Tts => config.tts_backend = Some(backend_id.clone()),
        Modality::Stt => config.stt_backend = Some(backend_id.clone()),
        Modality::Diffusion => config.diffusion_backend = Some(backend_id.clone()),
    }
    config_manager.save_config(&config);

    // Reconfigure the entire router from the updated config.
    // This constructs cloud backends immediately (they only need an API key)
    // and clears local backends (they're set lazily when sidecars start).
    let result = router.reconfigure(&config).await;

    if backend_id == "local" {
        tracing::info!(
            "[inference] {} set to local — backend will be activated when sidecar starts",
            modality
        );
    } else {
        tracing::info!(
            "[inference] {} backend reconfigured to '{}'",
            modality,
            backend_id
        );
    }

    // Check if embedding dimensions changed (callers may need to rebuild vector indices)
    if result.embedding_dims_changed() {
        tracing::warn!(
            "[inference] ⚠️ Embedding dimensions changed: {} → {}. Vector indices may need rebuilding!",
            result.old_embedding_dims,
            result.new_embedding_dims
        );
        // Notify frontend so the user can re-ingest documents
        use tauri::Emitter;
        let _ = app.emit("embedding_dims_changed", serde_json::json!({
            "old_dims": result.old_embedding_dims,
            "new_dims": result.new_embedding_dims,
            "message": format!(
                "Embedding dimensions changed from {} to {}. Previously ingested documents should be re-imported for best results.",
                result.old_embedding_dims, result.new_embedding_dims
            ),
        }));
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Cloud Model Discovery commands
// ─────────────────────────────────────────────────────────────────────────────

fn json_str_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn json_bool_field(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn json_u32_field(value: &serde_json::Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok())
}

fn remote_provider_slugs(
    providers_config: &serde_json::Value,
    requested: &[String],
) -> Vec<String> {
    if !requested.is_empty() {
        return requested.to_vec();
    }

    let mut seen = HashSet::new();
    let mut slugs = Vec::new();

    for key in ["primary_provider", "preferred_cheap_provider"] {
        if let Some(slug) = providers_config.get(key).and_then(|v| v.as_str()) {
            if seen.insert(slug.to_string()) {
                slugs.push(slug.to_string());
            }
        }
    }

    if let Some(providers) = providers_config.get("providers").and_then(|v| v.as_array()) {
        for provider in providers {
            let Some(slug) = provider.get("slug").and_then(|v| v.as_str()) else {
                continue;
            };
            let should_include = json_bool_field(provider, "enabled")
                || json_bool_field(provider, "primary")
                || json_bool_field(provider, "preferred_cheap")
                || json_bool_field(provider, "credential_ready")
                || json_bool_field(provider, "has_key")
                || !json_bool_field(provider, "auth_required");
            if should_include && seen.insert(slug.to_string()) {
                slugs.push(slug.to_string());
            }
        }
    }

    slugs
}

fn remote_provider_models_to_discovery(
    response: serde_json::Value,
) -> model_discovery::types::ProviderDiscoveryResult {
    let provider = json_str_field(&response, "slug").unwrap_or_else(|| "unknown".to_string());
    let provider_name =
        json_str_field(&response, "display_name").unwrap_or_else(|| provider.clone());
    let discovery_status =
        json_str_field(&response, "discovery_status").unwrap_or_else(|| "unknown".to_string());
    let error = json_str_field(&response, "error");

    let models = response
        .get("models")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    remote_model_option_to_entry(&provider, &provider_name, &discovery_status, item)
                })
                .collect()
        })
        .unwrap_or_default();

    model_discovery::types::ProviderDiscoveryResult {
        provider,
        models,
        from_cache: discovery_status == "cached",
        error,
    }
}

fn remote_model_option_to_entry(
    provider: &str,
    provider_name: &str,
    discovery_status: &str,
    item: &serde_json::Value,
) -> Option<model_discovery::types::CloudModelEntry> {
    let id = json_str_field(item, "id")?;
    let display_name = json_str_field(item, "label").unwrap_or_else(|| id.clone());
    let mut metadata = HashMap::new();
    if let Some(source) = json_str_field(item, "source") {
        metadata.insert("source".to_string(), source);
    }
    metadata.insert("discovery_status".to_string(), discovery_status.to_string());
    metadata.insert(
        "recommended_primary".to_string(),
        json_bool_field(item, "recommended_primary").to_string(),
    );
    metadata.insert(
        "recommended_cheap".to_string(),
        json_bool_field(item, "recommended_cheap").to_string(),
    );

    Some(model_discovery::types::CloudModelEntry {
        id,
        display_name,
        provider: provider.to_string(),
        provider_name: provider_name.to_string(),
        category: model_discovery::types::ModelCategory::Chat,
        context_window: json_u32_field(item, "context_length"),
        max_output_tokens: None,
        supports_vision: false,
        supports_tools: false,
        supports_streaming: true,
        deprecated: false,
        pricing: None,
        embedding_dimensions: None,
        metadata,
    })
}

async fn remote_discover_cloud_models(
    proxy: crate::openclaw::remote_proxy::RemoteGatewayProxy,
    providers: Vec<String>,
) -> Result<model_discovery::types::DiscoveryResult, String> {
    let config = proxy.get_providers_config().await?;
    let slugs = remote_provider_slugs(&config, &providers);
    let mut provider_results = Vec::new();
    let mut errors = Vec::new();

    for slug in slugs {
        match proxy.get_provider_models(&slug).await {
            Ok(response) => {
                let result = remote_provider_models_to_discovery(response);
                if let Some(error) = result.error.as_ref() {
                    errors.push(format!("{}: {}", result.provider, error));
                }
                provider_results.push(result);
            }
            Err(error) => errors.push(format!("{}: {}", slug, error)),
        }
    }

    let total_models = provider_results
        .iter()
        .map(|provider| provider.models.len() as u32)
        .sum();

    Ok(model_discovery::types::DiscoveryResult {
        providers: provider_results,
        total_models,
        errors,
    })
}

/// Discover cloud models from all providers (or a specific list).
///
/// Returns models grouped by provider. Results are cached for 30 minutes.
/// Pass an empty `providers` array to discover from ALL providers with keys.
#[tauri::command]
#[specta::specta]
pub async fn discover_cloud_models(
    ironclaw: tauri::State<'_, crate::openclaw::ironclaw_bridge::IronClawState>,
    registry: tauri::State<'_, CloudModelRegistry>,
    providers: Vec<String>,
) -> Result<model_discovery::types::DiscoveryResult, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return remote_discover_cloud_models(proxy, providers).await;
    }

    tracing::info!(
        "[model_discovery] Discovering models for {} providers",
        if providers.is_empty() {
            "all".to_string()
        } else {
            providers.len().to_string()
        }
    );
    let result = registry.discover(providers).await;
    tracing::info!(
        "[model_discovery] Discovery complete: {} models from {} providers ({} errors)",
        result.total_models,
        result.providers.len(),
        result.errors.len()
    );
    Ok(result)
}

/// Refresh models for a single provider (bypasses cache).
#[tauri::command]
#[specta::specta]
pub async fn refresh_cloud_models(
    ironclaw: tauri::State<'_, crate::openclaw::ironclaw_bridge::IronClawState>,
    registry: tauri::State<'_, CloudModelRegistry>,
    provider: String,
) -> Result<model_discovery::types::ProviderDiscoveryResult, String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy
            .get_provider_models(&provider)
            .await
            .map(remote_provider_models_to_discovery);
    }

    tracing::info!("[model_discovery] Refreshing models for '{}'", provider);
    Ok(registry.refresh(&provider).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_provider_slugs_prefers_requested_list() {
        let config = serde_json::json!({
            "primary_provider": "openai",
            "providers": [{ "slug": "anthropic", "enabled": true }]
        });
        assert_eq!(
            remote_provider_slugs(&config, &["gemini".to_string()]),
            vec!["gemini".to_string()]
        );
    }

    #[test]
    fn remote_provider_slugs_uses_configured_remote_providers() {
        let config = serde_json::json!({
            "primary_provider": "openai",
            "preferred_cheap_provider": "anthropic",
            "providers": [
                { "slug": "openai", "enabled": true, "auth_required": true },
                { "slug": "gemini", "credential_ready": true, "auth_required": true },
                { "slug": "ollama", "auth_required": false },
                { "slug": "xai", "auth_required": true }
            ]
        });

        assert_eq!(
            remote_provider_slugs(&config, &[]),
            vec![
                "openai".to_string(),
                "anthropic".to_string(),
                "gemini".to_string(),
                "ollama".to_string()
            ]
        );
    }

    #[test]
    fn remote_provider_models_map_to_discovery_result() {
        let result = remote_provider_models_to_discovery(serde_json::json!({
            "slug": "openai",
            "display_name": "OpenAI",
            "discovery_status": "discovered",
            "error": null,
            "models": [
                {
                    "id": "gpt-4.1",
                    "label": "GPT-4.1",
                    "context_length": 1048576,
                    "source": "live",
                    "recommended_primary": true,
                    "recommended_cheap": false
                }
            ]
        }));

        assert_eq!(result.provider, "openai");
        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].display_name, "GPT-4.1");
        assert_eq!(result.models[0].context_window, Some(1_048_576));
        assert_eq!(
            result.models[0].metadata.get("recommended_primary"),
            Some(&"true".to_string())
        );
    }
}
