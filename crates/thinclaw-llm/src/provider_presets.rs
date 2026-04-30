//! Pre-configured LLM provider presets.
//!
//! Thin wrappers around `OpenAiCompatibleConfig` that pre-fill the base URL,
//! model defaults, and env var names for popular providers. All use the
//! OpenAI-compatible chat completions API.

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// Known provider preset identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderPreset {
    /// NVIDIA AI Foundation (build.nvidia.com)
    Nvidia,
    /// Perplexity (pplx-api)
    Perplexity,
    /// MiniMax (hailuoai.com)
    MiniMax,
    /// Zhipu GLM (open.bigmodel.cn)
    Glm,
}

impl ProviderPreset {
    /// Display name.
    pub fn name(&self) -> &str {
        match self {
            Self::Nvidia => "NVIDIA AI",
            Self::Perplexity => "Perplexity",
            Self::MiniMax => "MiniMax",
            Self::Glm => "GLM (Zhipu)",
        }
    }

    /// Default base URL for this provider.
    pub fn default_base_url(&self) -> &str {
        match self {
            Self::Nvidia => "https://integrate.api.nvidia.com/v1",
            Self::Perplexity => "https://api.perplexity.ai",
            Self::MiniMax => "https://api.minimax.io/v1",
            Self::Glm => "https://open.bigmodel.cn/api/paas/v4",
        }
    }

    /// Default model for this provider.
    pub fn default_model(&self) -> &str {
        match self {
            Self::Nvidia => "meta/llama-3.3-70b-instruct",
            Self::Perplexity => "sonar-pro",
            Self::MiniMax => "MiniMax-M2.7",
            Self::Glm => "glm-4-plus",
        }
    }

    /// Env var name for the API key.
    pub fn api_key_env(&self) -> &str {
        match self {
            Self::Nvidia => "NVIDIA_API_KEY",
            Self::Perplexity => "PERPLEXITY_API_KEY",
            Self::MiniMax => "MINIMAX_API_KEY",
            Self::Glm => "GLM_API_KEY",
        }
    }

    /// Env var name for model override.
    pub fn model_env(&self) -> &str {
        match self {
            Self::Nvidia => "NVIDIA_MODEL",
            Self::Perplexity => "PERPLEXITY_MODEL",
            Self::MiniMax => "MINIMAX_MODEL",
            Self::Glm => "GLM_MODEL",
        }
    }

    /// Env var name for base URL override.
    pub fn base_url_env(&self) -> &str {
        match self {
            Self::Nvidia => "NVIDIA_BASE_URL",
            Self::Perplexity => "PERPLEXITY_BASE_URL",
            Self::MiniMax => "MINIMAX_BASE_URL",
            Self::Glm => "GLM_BASE_URL",
        }
    }

    /// Whether this provider supports streaming.
    pub fn supports_streaming(&self) -> bool {
        true // All four support streaming
    }

    /// Whether this provider supports tool/function calling.
    pub fn supports_tools(&self) -> bool {
        match self {
            Self::Nvidia => true,
            Self::Perplexity => false, // Perplexity search-first, limited tool support
            Self::MiniMax => true,
            Self::Glm => true,
        }
    }

    /// Provider-specific extra headers (if any).
    pub fn extra_headers(&self) -> Vec<(String, String)> {
        vec![]
    }

    /// All available presets.
    pub fn all() -> Vec<Self> {
        vec![Self::Nvidia, Self::Perplexity, Self::MiniMax, Self::Glm]
    }

    /// Parse from string (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "nvidia" | "nvidia_api" => Some(Self::Nvidia),
            "perplexity" | "pplx" => Some(Self::Perplexity),
            "minimax" => Some(Self::MiniMax),
            "glm" | "glm5" | "glm-5" | "zhipu" => Some(Self::Glm),
            _ => None,
        }
    }
}

/// Resolved provider preset config.
///
/// Contains the base URL, API key, model, and any extra headers
/// needed to use this provider through the OpenAI-compatible backend.
#[derive(Debug, Clone)]
pub struct ProviderPresetConfig {
    pub preset: ProviderPreset,
    pub base_url: String,
    pub api_key: Option<SecretString>,
    pub model: String,
    pub extra_headers: Vec<(String, String)>,
}

impl ProviderPresetConfig {
    /// Resolve a preset config from environment variables.
    ///
    /// Checks `{PROVIDER}_BASE_URL`, `{PROVIDER}_API_KEY`, `{PROVIDER}_MODEL`
    /// env vars, falling back to preset defaults.
    pub fn from_env(preset: ProviderPreset) -> Result<Self, String> {
        let base_url = optional_env(preset.base_url_env())
            .unwrap_or_else(|| preset.default_base_url().to_string());

        let api_key = optional_env(preset.api_key_env()).map(SecretString::from);

        let model =
            optional_env(preset.model_env()).unwrap_or_else(|| preset.default_model().to_string());

        let extra_headers = preset.extra_headers();

        Ok(Self {
            preset,
            base_url,
            api_key,
            model,
            extra_headers,
        })
    }

    /// Convert to the generic `OpenAiCompatibleConfig` fields.
    ///
    /// This allows the preset to be used with the existing OpenAI-compatible
    /// provider without any code changes.
    pub fn to_compatible_fields(
        &self,
    ) -> (String, Option<SecretString>, String, Vec<(String, String)>) {
        (
            self.base_url.clone(),
            self.api_key.clone(),
            self.model.clone(),
            self.extra_headers.clone(),
        )
    }

    /// Whether the preset has an API key configured.
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }
}

/// Detect if any provider preset is configured via environment variables.
///
/// Returns the first preset that has an API key set.
pub fn detect_preset() -> Option<ProviderPreset> {
    for preset in ProviderPreset::all() {
        if optional_env(preset.api_key_env()).is_some() {
            return Some(preset);
        }
    }
    None
}

/// List all presets with their configuration status.
pub fn list_presets() -> Vec<(ProviderPreset, bool)> {
    ProviderPreset::all()
        .into_iter()
        .map(|p| {
            let has_key = optional_env(p.api_key_env())
                .map(|value| !value.is_empty())
                .unwrap_or(false);
            (p, has_key)
        })
        .collect()
}

fn optional_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_names() {
        assert_eq!(ProviderPreset::Nvidia.name(), "NVIDIA AI");
        assert_eq!(ProviderPreset::Perplexity.name(), "Perplexity");
        assert_eq!(ProviderPreset::MiniMax.name(), "MiniMax");
        assert_eq!(ProviderPreset::Glm.name(), "GLM (Zhipu)");
    }

    #[test]
    fn test_preset_base_urls() {
        assert!(ProviderPreset::Nvidia.default_base_url().contains("nvidia"));
        assert!(
            ProviderPreset::Perplexity
                .default_base_url()
                .contains("perplexity")
        );
        assert!(
            ProviderPreset::MiniMax
                .default_base_url()
                .contains("minimax")
        );
        assert!(ProviderPreset::Glm.default_base_url().contains("bigmodel"));
    }

    #[test]
    fn test_preset_default_models() {
        assert!(ProviderPreset::Nvidia.default_model().contains("llama"));
        assert!(ProviderPreset::Perplexity.default_model().contains("sonar"));
        assert!(ProviderPreset::MiniMax.default_model().contains("MiniMax"));
        assert!(ProviderPreset::Glm.default_model().contains("glm"));
    }

    #[test]
    fn test_preset_from_str() {
        assert_eq!(
            ProviderPreset::from_str("nvidia"),
            Some(ProviderPreset::Nvidia)
        );
        assert_eq!(
            ProviderPreset::from_str("NVIDIA"),
            Some(ProviderPreset::Nvidia)
        );
        assert_eq!(
            ProviderPreset::from_str("pplx"),
            Some(ProviderPreset::Perplexity)
        );
        assert_eq!(
            ProviderPreset::from_str("minimax"),
            Some(ProviderPreset::MiniMax)
        );
        assert_eq!(ProviderPreset::from_str("glm"), Some(ProviderPreset::Glm));
        assert_eq!(ProviderPreset::from_str("zhipu"), Some(ProviderPreset::Glm));
        assert_eq!(ProviderPreset::from_str("unknown"), None);
    }

    #[test]
    fn test_preset_all() {
        assert_eq!(ProviderPreset::all().len(), 4);
    }

    #[test]
    fn test_preset_tools_support() {
        assert!(ProviderPreset::Nvidia.supports_tools());
        assert!(!ProviderPreset::Perplexity.supports_tools());
        assert!(ProviderPreset::MiniMax.supports_tools());
        assert!(ProviderPreset::Glm.supports_tools());
    }

    #[test]
    fn test_preset_streaming() {
        for preset in ProviderPreset::all() {
            assert!(preset.supports_streaming());
        }
    }

    #[test]
    fn test_minimax_defaults_match_current_api() {
        assert_eq!(
            ProviderPreset::MiniMax.default_base_url(),
            "https://api.minimax.io/v1"
        );
        assert_eq!(ProviderPreset::MiniMax.default_model(), "MiniMax-M2.7");
    }

    #[test]
    fn test_presets_do_not_require_extra_headers() {
        assert!(ProviderPreset::MiniMax.extra_headers().is_empty());
        assert!(ProviderPreset::Nvidia.extra_headers().is_empty());
        assert!(ProviderPreset::Perplexity.extra_headers().is_empty());
        assert!(ProviderPreset::Glm.extra_headers().is_empty());
    }

    #[test]
    fn test_from_env_defaults() {
        let config = ProviderPresetConfig::from_env(ProviderPreset::Nvidia).unwrap();
        assert_eq!(config.preset, ProviderPreset::Nvidia);
        assert!(config.base_url.contains("nvidia"));
        assert!(config.model.contains("llama"));
        assert!(!config.has_api_key()); // No env var set in tests
    }

    #[test]
    fn test_to_compatible_fields() {
        let config = ProviderPresetConfig::from_env(ProviderPreset::Perplexity).unwrap();
        let (url, _key, model, headers) = config.to_compatible_fields();
        assert!(url.contains("perplexity"));
        assert!(model.contains("sonar"));
        assert!(headers.is_empty());
    }

    #[test]
    fn test_preset_serializable() {
        let json = serde_json::to_string(&ProviderPreset::Nvidia).unwrap();
        assert_eq!(json, "\"nvidia\"");
        let deser: ProviderPreset = serde_json::from_str("\"perplexity\"").unwrap();
        assert_eq!(deser, ProviderPreset::Perplexity);
    }
}
