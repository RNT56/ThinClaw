//! Types for cloud model discovery.
//!
//! These types are shared between all provider discovery modules and are
//! serialized to the frontend via Specta (`Type` derive).

use serde::{Deserialize, Serialize};
use specta::Type;

// ─────────────────────────────────────────────────────────────────────────────
// Core types
// ─────────────────────────────────────────────────────────────────────────────

/// Which modality a discovered model serves.
///
/// This is separate from `crate::inference::Modality` because discovery
/// returns richer categories (e.g. a single model can support vision).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ModelCategory {
    Chat,
    Embedding,
    Tts,
    Stt,
    Diffusion,
    /// Model doesn't fit neatly into one category (e.g. multi-modal).
    Other,
}

impl std::fmt::Display for ModelCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Embedding => write!(f, "embedding"),
            Self::Tts => write!(f, "tts"),
            Self::Stt => write!(f, "stt"),
            Self::Diffusion => write!(f, "diffusion"),
            Self::Other => write!(f, "other"),
        }
    }
}

/// Pricing information for a cloud model.
///
/// All prices are in USD.  Fields are optional because not all providers
/// expose pricing via their API.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    /// Cost per million input tokens (chat/embedding).
    pub input_per_million: Option<f64>,
    /// Cost per million output tokens (chat).
    pub output_per_million: Option<f64>,
    /// Cost per image generated (diffusion).
    pub per_image: Option<f64>,
    /// Cost per minute of audio (STT/TTS).
    pub per_minute: Option<f64>,
    /// Cost per 1000 characters (TTS).
    pub per_1k_chars: Option<f64>,
}

/// A single model discovered from a cloud provider's API.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CloudModelEntry {
    /// Model ID as used by the provider API (e.g. `"gpt-4o"`, `"claude-3-5-sonnet-20241022"`).
    pub id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Provider slug matching `SecretStore` key (e.g. `"openai"`, `"anthropic"`, `"gemini"`).
    pub provider: String,
    /// Human-readable provider name (e.g. `"OpenAI"`, `"Anthropic"`).
    pub provider_name: String,
    /// Which modality this model serves.
    pub category: ModelCategory,
    /// Context window size in tokens (chat models).
    pub context_window: Option<u32>,
    /// Maximum output tokens (chat models).
    pub max_output_tokens: Option<u32>,
    /// Whether this model supports image/file input.
    pub supports_vision: bool,
    /// Whether this model supports tool/function calling.
    pub supports_tools: bool,
    /// Whether this model supports streaming responses.
    pub supports_streaming: bool,
    /// Whether this model is deprecated / scheduled for removal.
    pub deprecated: bool,
    /// Pricing info (if available from the provider).
    pub pricing: Option<ModelPricing>,
    /// Embedding dimensions (embedding models only).
    pub embedding_dimensions: Option<u32>,
    /// Freeform metadata from the provider (original JSON fields).
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

/// The result of a discovery call for a specific provider.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDiscoveryResult {
    /// Provider slug (e.g. `"openai"`, `"anthropic"`).
    pub provider: String,
    /// Discovered models.
    pub models: Vec<CloudModelEntry>,
    /// Whether this result came from cache or a fresh API call.
    pub from_cache: bool,
    /// Error message if discovery failed (models will be empty or stale cache).
    pub error: Option<String>,
}

/// Aggregated discovery result for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResult {
    /// Per-provider results.
    pub providers: Vec<ProviderDiscoveryResult>,
    /// Total number of models discovered.
    pub total_models: usize,
    /// Providers that had errors.
    pub errors: Vec<String>,
}
