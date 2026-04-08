//! Built-in provider endpoint catalog.
//!
//! Maps provider IDs to their API endpoint details, default models, and
//! secret store key names. This catalog enables ThinClaw to work with 20+
//! providers without requiring explicit base_url configuration.
//!
//! ## Usage
//!
//! - Headless: user sets `providers.enabled = ["anthropic", "openai"]` in
//!   `config.toml`, and API keys via `SecretsStore` or env vars.
//! - Scrappy: the bridge writes the same settings from the UI config.
//!
//! The catalog is the single source of truth for provider endpoints, replacing
//! both Scrappy's `provider_endpoints.rs` and the bridge's hardcoded match arms.

use std::collections::HashMap;

/// API compatibility mode for a cloud provider.
///
/// Determines which RigAdapter constructor to use when creating the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiStyle {
    /// Native OpenAI API (uses rig `openai::Client`)
    OpenAi,
    /// Native Anthropic API (uses rig `anthropic::Client`)
    Anthropic,
    /// OpenAI-compatible endpoint (uses rig `openai::Client` with custom base_url)
    OpenAiCompatible,
    /// Local Ollama instance
    Ollama,
}

/// Static endpoint configuration for a cloud provider.
#[derive(Debug, Clone)]
pub struct ProviderEndpoint {
    /// Human-readable display name.
    pub display_name: &'static str,
    /// Base URL for API requests.
    pub base_url: &'static str,
    /// API compatibility mode.
    pub api_style: ApiStyle,
    /// Default model to use if none specified by the user.
    pub default_model: &'static str,
    /// Default context window size in tokens.
    pub default_context_size: u32,
    /// Whether the provider supports streaming responses.
    pub supports_streaming: bool,
    /// Environment variable name for the API key (e.g., "OPENAI_API_KEY").
    pub env_key_name: &'static str,
    /// Secret store key name for `inject_llm_keys_from_secrets()`.
    /// This maps to the key used in `SecretsStore.get_decrypted()`.
    pub secret_name: &'static str,
}

/// Lazy-initialized catalog of all known cloud providers.
///
/// The key is the **provider slug** (matches keychain names, UI identifiers,
/// and the `providers.enabled` config values).
pub fn catalog() -> &'static HashMap<&'static str, ProviderEndpoint> {
    use std::sync::LazyLock;

    static CATALOG: LazyLock<HashMap<&'static str, ProviderEndpoint>> = LazyLock::new(|| {
        let entries: Vec<(&str, ProviderEndpoint)> = vec![
            (
                "anthropic",
                ProviderEndpoint {
                    display_name: "Anthropic",
                    base_url: "https://api.anthropic.com/v1",
                    api_style: ApiStyle::Anthropic,
                    default_model: "claude-sonnet-4-20250514",
                    default_context_size: 200_000,
                    supports_streaming: true,
                    env_key_name: "ANTHROPIC_API_KEY",
                    secret_name: "llm_anthropic_api_key",
                },
            ),
            (
                "openai",
                ProviderEndpoint {
                    display_name: "OpenAI",
                    base_url: "https://api.openai.com/v1",
                    api_style: ApiStyle::OpenAi,
                    default_model: "gpt-4o",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "OPENAI_API_KEY",
                    secret_name: "llm_openai_api_key",
                },
            ),
            (
                "gemini",
                ProviderEndpoint {
                    display_name: "Google Gemini",
                    base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "gemini-2.5-flash",
                    default_context_size: 1_000_000,
                    supports_streaming: true,
                    env_key_name: "GEMINI_API_KEY",
                    secret_name: "gemini",
                },
            ),
            (
                "groq",
                ProviderEndpoint {
                    display_name: "Groq",
                    base_url: "https://api.groq.com/openai/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "llama-3.3-70b-versatile",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "GROQ_API_KEY",
                    secret_name: "groq",
                },
            ),
            (
                "openrouter",
                ProviderEndpoint {
                    display_name: "OpenRouter",
                    base_url: "https://openrouter.ai/api/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "anthropic/claude-sonnet-4-20250514",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "OPENROUTER_API_KEY",
                    secret_name: "llm_compatible_api_key",
                },
            ),
            (
                "mistral",
                ProviderEndpoint {
                    display_name: "Mistral AI",
                    base_url: "https://api.mistral.ai/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "mistral-large-latest",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "MISTRAL_API_KEY",
                    secret_name: "mistral",
                },
            ),
            (
                "xai",
                ProviderEndpoint {
                    display_name: "xAI (Grok)",
                    base_url: "https://api.x.ai/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "grok-3",
                    default_context_size: 131_072,
                    supports_streaming: true,
                    env_key_name: "XAI_API_KEY",
                    secret_name: "xai",
                },
            ),
            (
                "together",
                ProviderEndpoint {
                    display_name: "Together AI",
                    base_url: "https://api.together.xyz/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "TOGETHER_API_KEY",
                    secret_name: "together",
                },
            ),
            (
                "venice",
                ProviderEndpoint {
                    display_name: "Venice AI",
                    base_url: "https://api.venice.ai/api/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "llama-3.3-70b",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "VENICE_API_KEY",
                    secret_name: "venice",
                },
            ),
            (
                "moonshot",
                ProviderEndpoint {
                    display_name: "Moonshot (Kimi)",
                    base_url: "https://api.moonshot.ai/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "moonshot-v1-auto",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "MOONSHOT_API_KEY",
                    secret_name: "moonshot",
                },
            ),
            (
                "minimax",
                ProviderEndpoint {
                    display_name: "MiniMax",
                    base_url: "https://api.minimax.io/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "MiniMax-M2.7",
                    default_context_size: 1_000_000,
                    supports_streaming: true,
                    env_key_name: "MINIMAX_API_KEY",
                    secret_name: "minimax",
                },
            ),
            (
                "nvidia",
                ProviderEndpoint {
                    display_name: "NVIDIA NIM",
                    base_url: "https://integrate.api.nvidia.com/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "meta/llama-3.3-70b-instruct",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "NVIDIA_API_KEY",
                    secret_name: "nvidia",
                },
            ),
            (
                "deepseek",
                ProviderEndpoint {
                    display_name: "DeepSeek",
                    base_url: "https://api.deepseek.com/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "deepseek-chat",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "DEEPSEEK_API_KEY",
                    secret_name: "deepseek",
                },
            ),
            (
                "cerebras",
                ProviderEndpoint {
                    display_name: "Cerebras",
                    base_url: "https://api.cerebras.ai/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "llama-3.3-70b",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "CEREBRAS_API_KEY",
                    secret_name: "cerebras",
                },
            ),
            (
                "cohere",
                ProviderEndpoint {
                    display_name: "Cohere",
                    base_url: "https://api.cohere.ai/compatibility/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "command-a-03-2025",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "COHERE_API_KEY",
                    secret_name: "cohere",
                },
            ),
            (
                "tinfoil",
                ProviderEndpoint {
                    display_name: "Tinfoil",
                    base_url: "https://inference.tinfoil.sh/v1",
                    api_style: ApiStyle::OpenAiCompatible,
                    default_model: "kimi-k2-5",
                    default_context_size: 128_000,
                    supports_streaming: true,
                    env_key_name: "TINFOIL_API_KEY",
                    secret_name: "llm_tinfoil_api_key",
                },
            ),
        ];

        entries.into_iter().collect()
    });

    &CATALOG
}

/// Look up a provider's endpoint configuration by its slug.
pub fn endpoint_for(provider_id: &str) -> Option<&'static ProviderEndpoint> {
    catalog().get(provider_id)
}

/// Get all provider slugs.
pub fn all_provider_ids() -> Vec<&'static str> {
    catalog().keys().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_not_empty() {
        assert!(!catalog().is_empty());
    }

    #[test]
    fn known_providers_exist() {
        assert!(endpoint_for("anthropic").is_some());
        assert!(endpoint_for("openai").is_some());
        assert!(endpoint_for("groq").is_some());
        assert!(endpoint_for("gemini").is_some());
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert!(endpoint_for("nonexistent").is_none());
    }

    #[test]
    fn anthropic_uses_native_api() {
        let ep = endpoint_for("anthropic").unwrap();
        assert_eq!(ep.api_style, ApiStyle::Anthropic);
    }

    #[test]
    fn openai_uses_native_api() {
        let ep = endpoint_for("openai").unwrap();
        assert_eq!(ep.api_style, ApiStyle::OpenAi);
    }

    #[test]
    fn groq_uses_compatible_api() {
        let ep = endpoint_for("groq").unwrap();
        assert_eq!(ep.api_style, ApiStyle::OpenAiCompatible);
    }

    #[test]
    fn minimax_uses_current_openai_compat_endpoint() {
        let ep = endpoint_for("minimax").unwrap();
        assert_eq!(ep.base_url, "https://api.minimax.io/v1");
        assert_eq!(ep.default_model, "MiniMax-M2.7");
    }

    #[test]
    fn cohere_uses_compatibility_api() {
        let ep = endpoint_for("cohere").unwrap();
        assert_eq!(ep.base_url, "https://api.cohere.ai/compatibility/v1");
        assert_eq!(ep.default_model, "command-a-03-2025");
    }

    #[test]
    fn xiaomi_is_not_in_catalog() {
        assert!(endpoint_for("xiaomi").is_none());
    }
}
