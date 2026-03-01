//! Shared provider endpoint configuration.
//!
//! Maps `provider_id` → `{ base_url, api_compat, default_model, default_context_size }`.
//! Used by both `resolve_provider()` and model discovery modules to eliminate
//! base_url duplication.

use serde::{Deserialize, Serialize};
use specta::Type;

/// API compatibility mode for a cloud provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ApiCompat {
    /// OpenAI-compatible `/v1/chat/completions` endpoint.
    OpenAi,
    /// Anthropic Messages API.
    Anthropic,
    /// Google Gemini REST API.
    Gemini,
    /// Cohere `/v1/chat` API.
    Cohere,
    /// AWS Bedrock SDK.
    Bedrock,
}

impl ApiCompat {
    /// Convert to the `ProviderKind` used by `UnifiedProvider`.
    pub fn to_provider_kind(self) -> crate::rig_lib::unified_provider::ProviderKind {
        use crate::rig_lib::unified_provider::ProviderKind;
        match self {
            Self::OpenAi => ProviderKind::OpenAI,
            Self::Anthropic => ProviderKind::Anthropic,
            Self::Gemini => ProviderKind::Gemini,
            Self::Cohere => ProviderKind::OpenAI, // Cohere v2 uses OpenAI-compat
            Self::Bedrock => ProviderKind::OpenAI, // placeholder
        }
    }
}

/// Static endpoint configuration for a cloud chat provider.
#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    /// Human-readable name.
    pub display_name: &'static str,
    /// Base URL for API requests.
    pub base_url: &'static str,
    /// API compatibility mode.
    pub api_compat: ApiCompat,
    /// Default model to use if none specified.
    pub default_model: &'static str,
    /// Default context size in tokens.
    pub default_context_size: u32,
    /// Whether the provider supports streaming.
    pub supports_streaming: bool,
}

/// Look up a provider's endpoint configuration by its keychain slug.
pub fn endpoint_for(provider_id: &str) -> Option<&'static ProviderEndpoint> {
    PROVIDER_ENDPOINTS
        .iter()
        .find(|(id, _)| *id == provider_id)
        .map(|(_, ep)| ep)
}

/// All known cloud chat provider endpoints.
///
/// The first element of each tuple is the **keychain slug** (matches
/// `keychain::PROVIDERS` and `SecretStore.get(slug)`).
pub static PROVIDER_ENDPOINTS: &[(&str, ProviderEndpoint)] = &[
    (
        "anthropic",
        ProviderEndpoint {
            display_name: "Anthropic",
            base_url: "https://api.anthropic.com/v1",
            api_compat: ApiCompat::Anthropic,
            default_model: "claude-sonnet-4-20250514",
            default_context_size: 200_000,
            supports_streaming: true,
        },
    ),
    (
        "openai",
        ProviderEndpoint {
            display_name: "OpenAI",
            base_url: "https://api.openai.com/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "gpt-4o",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "gemini",
        ProviderEndpoint {
            display_name: "Google Gemini",
            base_url: "https://generativelanguage.googleapis.com/v1beta/models",
            api_compat: ApiCompat::Gemini,
            default_model: "gemini-2.5-flash",
            default_context_size: 1_000_000,
            supports_streaming: true,
        },
    ),
    (
        "groq",
        ProviderEndpoint {
            display_name: "Groq",
            base_url: "https://api.groq.com/openai/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "llama-3.3-70b-versatile",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "openrouter",
        ProviderEndpoint {
            display_name: "OpenRouter",
            base_url: "https://openrouter.ai/api/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "anthropic/claude-sonnet-4-20250514",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "mistral",
        ProviderEndpoint {
            display_name: "Mistral AI",
            base_url: "https://api.mistral.ai/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "mistral-large-latest",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "xai",
        ProviderEndpoint {
            display_name: "xAI (Grok)",
            base_url: "https://api.x.ai/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "grok-3",
            default_context_size: 131_072,
            supports_streaming: true,
        },
    ),
    (
        "together",
        ProviderEndpoint {
            display_name: "Together AI",
            base_url: "https://api.together.xyz/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "venice",
        ProviderEndpoint {
            display_name: "Venice AI",
            base_url: "https://api.venice.ai/api/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "llama-3.3-70b",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "moonshot",
        ProviderEndpoint {
            display_name: "Moonshot (Kimi)",
            base_url: "https://api.moonshot.ai/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "moonshot-v1-auto",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "minimax",
        ProviderEndpoint {
            display_name: "MiniMax",
            base_url: "https://api.minimax.chat/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "MiniMax-Text-01",
            default_context_size: 1_000_000,
            supports_streaming: true,
        },
    ),
    (
        "nvidia",
        ProviderEndpoint {
            display_name: "NVIDIA NIM",
            base_url: "https://integrate.api.nvidia.com/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "meta/llama-3.3-70b-instruct",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "cohere",
        ProviderEndpoint {
            display_name: "Cohere",
            base_url: "https://api.cohere.com/v2",
            api_compat: ApiCompat::Cohere,
            default_model: "command-r-plus",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
    (
        "xiaomi",
        ProviderEndpoint {
            display_name: "Xiaomi",
            base_url: "https://api.xiaomi.com/v1",
            api_compat: ApiCompat::OpenAi,
            default_model: "MiMo-7B-RL",
            default_context_size: 128_000,
            supports_streaming: true,
        },
    ),
];
