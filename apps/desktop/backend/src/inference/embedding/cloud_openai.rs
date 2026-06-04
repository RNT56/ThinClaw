//! OpenAI embedding backend.
//!
//! Supports `text-embedding-3-small` (1536 dims) and `text-embedding-3-large` (3072 dims).

use crate::inference::embedding::EmbeddingBackend;
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct OpenAiEmbeddingBackend {
    pub api_key: String,
    pub model: String,
    pub dims: usize,
}

impl OpenAiEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        let model = model_override.unwrap_or_else(|| "text-embedding-3-small".to_string());
        let dims = if model.contains("large") { 3072 } else { 1536 };
        Self {
            api_key,
            model,
            dims,
        }
    }

    pub fn small(api_key: String) -> Self {
        Self::new(api_key, None)
    }

    pub fn large(api_key: String) -> Self {
        Self::new(api_key, Some("text-embedding-3-large".to_string()))
    }
}

#[async_trait]
impl EmbeddingBackend for OpenAiEmbeddingBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "openai".to_string(),
            display_name: format!("OpenAI ({})", self.model),
            is_local: false,
            model_id: Some(self.model.clone()),
            available: true,
        }
    }

    async fn embed_batch(&self, texts: Vec<String>) -> InferenceResult<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();

        let response = client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "input": texts,
                "model": self.model,
                "dimensions": self.dims
            }))
            .send()
            .await
            .map_err(|e| {
                InferenceError::network(format!("OpenAI embedding request failed: {}", e))
            })?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid OpenAI API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "OpenAI embedding error ({}): {}",
                status, text
            )));
        }

        #[derive(serde::Deserialize)]
        struct Response {
            data: Vec<Data>,
        }
        #[derive(serde::Deserialize)]
        struct Data {
            embedding: Vec<f32>,
        }

        let result: Response = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Failed to parse: {}", e)))?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
