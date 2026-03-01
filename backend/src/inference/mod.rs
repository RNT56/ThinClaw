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
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Cloud Model Discovery commands
// ─────────────────────────────────────────────────────────────────────────────

/// Discover cloud models from all providers (or a specific list).
///
/// Returns models grouped by provider. Results are cached for 30 minutes.
/// Pass an empty `providers` array to discover from ALL providers with keys.
#[tauri::command]
#[specta::specta]
pub async fn discover_cloud_models(
    registry: tauri::State<'_, CloudModelRegistry>,
    providers: Vec<String>,
) -> Result<model_discovery::types::DiscoveryResult, String> {
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
    registry: tauri::State<'_, CloudModelRegistry>,
    provider: String,
) -> Result<model_discovery::types::ProviderDiscoveryResult, String> {
    tracing::info!("[model_discovery] Refreshing models for '{}'", provider);
    Ok(registry.refresh(&provider).await)
}
