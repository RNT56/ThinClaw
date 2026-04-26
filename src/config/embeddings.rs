use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;
use crate::workspace::EmbeddingProvider;

/// Embeddings provider configuration.
#[derive(Debug, Clone)]
pub struct EmbeddingsConfig {
    /// Whether embeddings are enabled.
    pub enabled: bool,
    /// Provider to use: "openai", "ollama", or "bedrock"
    pub provider: String,
    /// OpenAI API key (for OpenAI provider).
    pub openai_api_key: Option<SecretString>,
    /// Model to use for embeddings.
    pub model: String,
    /// Ollama base URL (for Ollama provider). Defaults to http://localhost:11434.
    pub ollama_base_url: String,
    /// Embedding vector dimension. Inferred from the model name when not set explicitly.
    pub dimension: usize,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        let model = "text-embedding-3-small".to_string();
        let dimension = default_dimension_for_model(&model);
        Self {
            enabled: false,
            provider: "openai".to_string(),
            openai_api_key: None,
            model,
            ollama_base_url: "http://localhost:11434".to_string(),
            dimension,
        }
    }
}

/// Infer the embedding dimension from a well-known model name.
///
/// Falls back to 1536 (OpenAI text-embedding-3-small default) for unknown models.
fn default_dimension_for_model(model: &str) -> usize {
    match model {
        "text-embedding-3-small" => 1536,
        "text-embedding-3-large" => 3072,
        "text-embedding-ada-002" => 1536,
        "amazon.titan-embed-text-v2:0" => 1024,
        "nomic-embed-text" => 768,
        "mxbai-embed-large" => 1024,
        "all-minilm" => 384,
        _ => 1536,
    }
}

impl EmbeddingsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let openai_api_key = optional_env("OPENAI_API_KEY")?.map(SecretString::from);

        let provider = optional_env("EMBEDDING_PROVIDER")?
            .unwrap_or_else(|| settings.embeddings.provider.clone());

        let model = if provider == "bedrock" {
            optional_env("EMBEDDING_MODEL")?
                .unwrap_or_else(|| "amazon.titan-embed-text-v2:0".to_string())
        } else {
            optional_env("EMBEDDING_MODEL")?.unwrap_or_else(|| settings.embeddings.model.clone())
        };

        let ollama_base_url = optional_env("OLLAMA_BASE_URL")?
            .or_else(|| settings.ollama_base_url.clone())
            .unwrap_or_else(|| "http://localhost:11434".to_string());

        let dimension =
            parse_optional_env("EMBEDDING_DIMENSION", default_dimension_for_model(&model))?;

        // Bedrock Titan v2 only supports specific dimensions.
        if provider == "bedrock" && !matches!(dimension, 256 | 512 | 1024) {
            return Err(ConfigError::InvalidValue {
                key: "EMBEDDING_DIMENSION".to_string(),
                message: "Bedrock Titan v2 embeddings support only 256, 512, or 1024 dimensions"
                    .to_string(),
            });
        }

        let enabled = parse_bool_env("EMBEDDING_ENABLED", settings.embeddings.enabled)?;

        Ok(Self {
            enabled,
            provider,
            openai_api_key,
            model,
            ollama_base_url,
            dimension,
        })
    }

    /// Get the OpenAI API key if configured.
    pub fn openai_api_key(&self) -> Option<&str> {
        self.openai_api_key.as_ref().map(|s| s.expose_secret())
    }

    /// Create the appropriate embedding provider based on configuration.
    ///
    /// Returns `None` if embeddings are disabled or the required credentials
    /// are missing.
    pub async fn create_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        if !self.enabled {
            tracing::info!("Embeddings disabled (set EMBEDDING_ENABLED=true to enable)");
            return None;
        }

        match self.provider.as_str() {
            "bedrock" => {
                #[cfg(feature = "bedrock")]
                {
                    let region =
                        std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
                    let profile = std::env::var("AWS_PROFILE").ok();
                    tracing::info!(
                        "Embeddings enabled via Bedrock (model: {}, region: {}, dim: {})",
                        self.model,
                        region,
                        self.dimension,
                    );
                    let provider = crate::workspace::BedrockEmbeddings::new(
                        region,
                        profile.as_deref(),
                        &self.model,
                        self.dimension,
                    )
                    .await;
                    Some(Arc::new(provider) as Arc<dyn EmbeddingProvider>)
                }
                #[cfg(not(feature = "bedrock"))]
                {
                    tracing::warn!(
                        "Embeddings configured for Bedrock but the `bedrock` feature is disabled. \
                         Rebuild with --features bedrock to enable."
                    );
                    None
                }
            }
            "ollama" => {
                tracing::info!(
                    "Embeddings enabled via Ollama (model: {}, url: {}, dim: {})",
                    self.model,
                    self.ollama_base_url,
                    self.dimension,
                );
                Some(Arc::new(
                    crate::workspace::OllamaEmbeddings::new(&self.ollama_base_url)
                        .with_model(&self.model, self.dimension),
                ))
            }
            _ => {
                if let Some(api_key) = self.openai_api_key() {
                    tracing::info!(
                        "Embeddings enabled via OpenAI (model: {}, dim: {})",
                        self.model,
                        self.dimension,
                    );
                    Some(Arc::new(crate::workspace::OpenAiEmbeddings::with_model(
                        api_key,
                        &self.model,
                        self.dimension,
                    )))
                } else {
                    tracing::warn!("Embeddings configured but OPENAI_API_KEY not set");
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;
    use crate::settings::{EmbeddingsSettings, Settings};

    /// Clear all embedding-related env vars.
    fn clear_embedding_env() {
        // SAFETY: Only called under ENV_MUTEX in tests.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_MODEL");
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn embeddings_disabled_not_overridden_by_openai_key() {
        let _guard = lock_env();

        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX, no concurrent env access.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "sk-test-key-for-issue-129");
        }

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            !config.enabled,
            "embeddings should remain disabled when settings.embeddings.enabled=false, \
             even when OPENAI_API_KEY is set (issue #129)"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[test]
    fn embeddings_enabled_from_settings() {
        let _guard = lock_env();
        clear_embedding_env();

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "embeddings should be enabled when settings say so"
        );
    }

    #[test]
    fn embeddings_env_override_takes_precedence() {
        let _guard = lock_env();

        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
        }

        let settings = Settings {
            embeddings: EmbeddingsSettings {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };

        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert!(
            config.enabled,
            "EMBEDDING_ENABLED=true env var should override settings"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
        }
    }

    #[test]
    fn bedrock_provider_defaults_to_titan_v2() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
            std::env::set_var("EMBEDDING_PROVIDER", "bedrock");
        }

        let settings = Settings::default();
        let config = EmbeddingsConfig::resolve(&settings).expect("resolve should succeed");
        assert_eq!(config.provider, "bedrock");
        assert_eq!(config.model, "amazon.titan-embed-text-v2:0");
        assert_eq!(config.dimension, 1024);

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
        }
    }

    #[test]
    fn bedrock_dimension_validation_rejects_unsupported_values() {
        let _guard = lock_env();
        clear_embedding_env();
        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::set_var("EMBEDDING_ENABLED", "true");
            std::env::set_var("EMBEDDING_PROVIDER", "bedrock");
            std::env::set_var("EMBEDDING_DIMENSION", "1536");
        }

        let settings = Settings::default();
        let result = EmbeddingsConfig::resolve(&settings);
        assert!(
            result.is_err(),
            "unsupported bedrock dimensions should fail"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("256, 512, or 1024"),
            "error should mention valid dimensions: {err}"
        );

        // SAFETY: Under ENV_MUTEX.
        unsafe {
            std::env::remove_var("EMBEDDING_ENABLED");
            std::env::remove_var("EMBEDDING_PROVIDER");
            std::env::remove_var("EMBEDDING_DIMENSION");
        }
    }
}
