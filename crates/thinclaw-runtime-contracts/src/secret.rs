use serde::{Deserialize, Serialize};

/// Runtime family requesting secret access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SecretConsumer {
    DirectWorkbench,
    ThinClawAgent,
    GatewayProxy,
    Extension,
    System,
}

/// How a secret may be used by a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum SecretAccessMode {
    Status,
    ExplicitUse,
    RuntimeInjection,
}

/// Canonical secret identity plus compatibility aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct SecretDescriptor {
    pub canonical_name: String,
    #[serde(default)]
    pub provider_slug: Option<String>,
    #[serde(default)]
    pub env_key_name: Option<String>,
    #[serde(default)]
    pub legacy_aliases: Vec<String>,
    #[serde(default)]
    pub allowed_consumers: Vec<SecretConsumer>,
}

/// Provider credential metadata safe to expose to clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ProviderCredentialDescriptor {
    pub provider_slug: String,
    pub display_name: String,
    pub secret_name: String,
    pub env_key_name: String,
    #[serde(default)]
    pub setup_url: Option<String>,
    #[serde(default)]
    pub credential_ready: bool,
}

/// Return the canonical secret name for a provider or legacy secret alias.
pub fn canonical_secret_name(name: &str) -> &str {
    match name {
        "anthropic" | "ANTHROPIC_API_KEY" | "llm_anthropic_api_key" => "llm_anthropic_api_key",
        "openai" | "OPENAI_API_KEY" | "llm_openai_api_key" => "llm_openai_api_key",
        "openrouter" | "openai_compatible" | "LLM_API_KEY" | "llm_compatible_api_key" => {
            "llm_compatible_api_key"
        }
        "brave" | "BRAVE_SEARCH_API_KEY" | "search_brave_api_key" => "search_brave_api_key",
        "huggingface" | "HUGGINGFACE_TOKEN" | "HF_TOKEN" | "hf_token" => "hf_token",
        other => other,
    }
}

/// Compatibility aliases that should be read during migration and lookup.
pub fn legacy_secret_aliases(canonical_name: &str) -> &'static [&'static str] {
    match canonical_secret_name(canonical_name) {
        "llm_anthropic_api_key" => &["anthropic", "ANTHROPIC_API_KEY"],
        "llm_openai_api_key" => &["openai", "OPENAI_API_KEY"],
        "llm_compatible_api_key" => &["openrouter", "openai_compatible", "LLM_API_KEY"],
        "search_brave_api_key" => &["brave", "BRAVE_SEARCH_API_KEY"],
        "hf_token" => &["huggingface", "HUGGINGFACE_TOKEN", "HF_TOKEN"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_legacy_provider_names() {
        assert_eq!(canonical_secret_name("anthropic"), "llm_anthropic_api_key");
        assert_eq!(
            canonical_secret_name("LLM_API_KEY"),
            "llm_compatible_api_key"
        );
        assert_eq!(canonical_secret_name("HF_TOKEN"), "hf_token");
        assert_eq!(canonical_secret_name("cohere"), "cohere");
    }
}
