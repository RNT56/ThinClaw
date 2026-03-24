//! Embedding support: Gemini and local embeddings.
//!
//! Provides embedding generation via Google Gemini API and local
//! models (e.g., sentence-transformers via ONNX or local API).

use serde::{Deserialize, Serialize};

/// Configuration for an embedding provider.
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Provider type.
    pub provider: EmbeddingProvider,
    /// Model name/ID.
    pub model: String,
    /// Embedding dimensions (output vector size).
    pub dimensions: Option<u32>,
    /// API key (for remote providers).
    pub api_key: Option<String>,
    /// Base URL override.
    pub base_url: Option<String>,
    /// Max tokens per request.
    pub max_tokens: u32,
}

/// Supported embedding providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EmbeddingProvider {
    /// Google Gemini text-embedding API.
    Gemini,
    /// OpenAI embeddings endpoint.
    OpenAI,
    /// Local model via HTTP API (e.g., TEI, Ollama).
    Local,
    /// Ollama embeddings endpoint.
    Ollama,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProvider::OpenAI,
            model: "text-embedding-3-small".to_string(),
            dimensions: Some(1536),
            api_key: None,
            base_url: None,
            max_tokens: 8191,
        }
    }
}

impl EmbeddingConfig {
    /// Create a Gemini embedding config.
    pub fn gemini() -> Self {
        Self {
            provider: EmbeddingProvider::Gemini,
            model: "text-embedding-004".to_string(),
            dimensions: Some(768),
            api_key: std::env::var("GOOGLE_AI_API_KEY").ok(),
            base_url: Some("https://generativelanguage.googleapis.com/v1beta".to_string()),
            max_tokens: 2048,
        }
    }

    /// Create a local embedding config.
    pub fn local() -> Self {
        Self {
            provider: EmbeddingProvider::Local,
            model: "all-MiniLM-L6-v2".to_string(),
            dimensions: Some(384),
            api_key: None,
            base_url: Some(
                std::env::var("LOCAL_EMBEDDING_URL")
                    .unwrap_or_else(|_| "http://localhost:8080".to_string()),
            ),
            max_tokens: 512,
        }
    }

    /// Create an Ollama embedding config.
    pub fn ollama() -> Self {
        Self {
            provider: EmbeddingProvider::Ollama,
            model: "nomic-embed-text".to_string(),
            dimensions: Some(768),
            api_key: None,
            base_url: Some(
                std::env::var("OLLAMA_HOST")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string()),
            ),
            max_tokens: 8192,
        }
    }

    /// Create from environment.
    pub fn from_env() -> Self {
        let provider = match std::env::var("EMBEDDING_PROVIDER")
            .unwrap_or_default()
            .to_lowercase()
            .as_str()
        {
            "gemini" => return Self::gemini(),
            "local" => return Self::local(),
            "ollama" => return Self::ollama(),
            _ => EmbeddingProvider::OpenAI,
        };

        let mut config = Self {
            provider,
            ..Self::default()
        };
        if let Ok(model) = std::env::var("EMBEDDING_MODEL") {
            config.model = model;
        }
        if let Ok(dims) = std::env::var("EMBEDDING_DIMENSIONS") {
            config.dimensions = dims.parse().ok();
        }
        config
    }

    /// Get the API endpoint for embeddings.
    pub fn endpoint(&self) -> String {
        match self.provider {
            EmbeddingProvider::Gemini => {
                let base = self
                    .base_url
                    .as_deref()
                    .unwrap_or("https://generativelanguage.googleapis.com/v1beta");
                format!("{}/models/{}:embedContent", base, self.model)
            }
            EmbeddingProvider::OpenAI => {
                let base = self
                    .base_url
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1");
                format!("{}/embeddings", base)
            }
            EmbeddingProvider::Local => {
                let base = self.base_url.as_deref().unwrap_or("http://localhost:8080");
                format!("{}/embed", base)
            }
            EmbeddingProvider::Ollama => {
                let base = self.base_url.as_deref().unwrap_or("http://localhost:11434");
                format!("{}/api/embeddings", base)
            }
        }
    }
}

/// Gemini embedding request.
#[derive(Debug, Serialize)]
pub struct GeminiEmbedRequest {
    pub model: String,
    pub content: GeminiEmbedContent,
}

#[derive(Debug, Serialize)]
pub struct GeminiEmbedContent {
    pub parts: Vec<GeminiEmbedPart>,
}

#[derive(Debug, Serialize)]
pub struct GeminiEmbedPart {
    pub text: String,
}

/// Gemini embedding response.
#[derive(Debug, Deserialize)]
pub struct GeminiEmbedResponse {
    pub embedding: Option<GeminiEmbeddingValues>,
}

#[derive(Debug, Deserialize)]
pub struct GeminiEmbeddingValues {
    pub values: Vec<f32>,
}

/// Build a Gemini embed request.
pub fn build_gemini_request(text: &str, model: &str) -> GeminiEmbedRequest {
    GeminiEmbedRequest {
        model: format!("models/{}", model),
        content: GeminiEmbedContent {
            parts: vec![GeminiEmbedPart {
                text: text.to_string(),
            }],
        },
    }
}

/// Ollama embedding request.
#[derive(Debug, Serialize)]
pub struct OllamaEmbedRequest {
    pub model: String,
    pub prompt: String,
}

/// Ollama embedding response.
#[derive(Debug, Deserialize)]
pub struct OllamaEmbedResponse {
    pub embedding: Vec<f32>,
}

/// Embedding result.
#[derive(Debug, Clone)]
pub struct EmbeddingResult {
    pub vector: Vec<f32>,
    pub model: String,
    pub dimensions: usize,
    pub token_count: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.provider, EmbeddingProvider::OpenAI);
        assert_eq!(config.dimensions, Some(1536));
    }

    #[test]
    fn test_gemini_config() {
        let config = EmbeddingConfig::gemini();
        assert_eq!(config.provider, EmbeddingProvider::Gemini);
        assert_eq!(config.model, "text-embedding-004");
        assert_eq!(config.dimensions, Some(768));
    }

    #[test]
    fn test_local_config() {
        let config = EmbeddingConfig::local();
        assert_eq!(config.provider, EmbeddingProvider::Local);
        assert_eq!(config.dimensions, Some(384));
    }

    #[test]
    fn test_ollama_config() {
        let config = EmbeddingConfig::ollama();
        assert_eq!(config.provider, EmbeddingProvider::Ollama);
        assert_eq!(config.model, "nomic-embed-text");
    }

    #[test]
    fn test_gemini_endpoint() {
        let config = EmbeddingConfig::gemini();
        assert!(config.endpoint().contains("embedContent"));
        assert!(config.endpoint().contains("text-embedding-004"));
    }

    #[test]
    fn test_openai_endpoint() {
        let config = EmbeddingConfig::default();
        assert_eq!(config.endpoint(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn test_ollama_endpoint() {
        let config = EmbeddingConfig::ollama();
        assert!(config.endpoint().contains("/api/embeddings"));
    }

    #[test]
    fn test_build_gemini_request() {
        let req = build_gemini_request("hello world", "text-embedding-004");
        assert_eq!(req.content.parts[0].text, "hello world");
        assert!(req.model.contains("text-embedding-004"));
    }
}
