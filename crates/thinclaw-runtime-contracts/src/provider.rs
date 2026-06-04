use serde::{Deserialize, Serialize};

/// API compatibility mode for a cloud provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum ApiStyle {
    /// Native OpenAI API.
    #[serde(rename = "openai")]
    OpenAi,
    /// Native Anthropic API.
    #[serde(rename = "anthropic")]
    Anthropic,
    /// OpenAI-compatible endpoint.
    #[serde(rename = "openai_compatible")]
    OpenAiCompatible,
    /// Local Ollama-compatible runtime.
    #[serde(rename = "ollama")]
    Ollama,
}

/// Endpoint configuration for a cloud provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProviderEndpoint {
    /// Provider slug, serialized as `id` in the registry.
    #[serde(rename = "id")]
    pub slug: String,
    pub display_name: String,
    pub base_url: String,
    pub api_style: ApiStyle,
    pub default_model: String,
    pub default_context_size: u32,
    pub supports_streaming: bool,
    pub env_key_name: String,
    pub secret_name: String,
    #[serde(default)]
    pub setup_url: Option<String>,
    #[serde(default)]
    pub suggested_cheap_model: Option<String>,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_provider_json_uses_contract_shape() {
        let entries: Vec<ProviderEndpoint> =
            serde_json::from_str(include_str!("../../../registry/providers.json"))
                .expect("registry/providers.json must match ProviderEndpoint");
        assert!(entries.iter().any(|entry| entry.slug == "openai"));
        assert!(entries.iter().any(|entry| entry.slug == "anthropic"));
        assert!(entries.iter().all(|entry| !entry.secret_name.is_empty()));
    }

    #[test]
    fn minimax_and_cohere_match_current_registry_contract() {
        let entries: Vec<ProviderEndpoint> =
            serde_json::from_str(include_str!("../../../registry/providers.json")).unwrap();
        let minimax = entries
            .iter()
            .find(|entry| entry.slug == "minimax")
            .unwrap();
        assert_eq!(minimax.base_url, "https://api.minimax.io/v1");
        assert_eq!(minimax.default_model, "MiniMax-M2.7");

        let cohere = entries.iter().find(|entry| entry.slug == "cohere").unwrap();
        assert_eq!(cohere.base_url, "https://api.cohere.ai/compatibility/v1");
        assert_eq!(cohere.default_model, "command-a-03-2025");
    }
}
