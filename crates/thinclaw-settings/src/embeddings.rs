use super::*;

/// Source for the secrets master key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// Auto-generated key stored in OS keychain.
    Keychain,
    /// User provides via SECRETS_MASTER_KEY env var.
    Env,
    /// Not configured (secrets features disabled).
    #[default]
    None,
}

/// Embeddings configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingsSettings {
    /// Whether embeddings are enabled.
    #[serde(default)]
    pub enabled: bool,

    /// Provider to use: "openai" or "ollama".
    #[serde(default = "default_embeddings_provider")]
    pub provider: String,

    /// Model to use for embeddings.
    #[serde(default = "default_embeddings_model")]
    pub model: String,
}

fn default_embeddings_provider() -> String {
    "openai".to_string()
}

fn default_embeddings_model() -> String {
    "text-embedding-3-small".to_string()
}

impl Default for EmbeddingsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: default_embeddings_provider(),
            model: default_embeddings_model(),
        }
    }
}
