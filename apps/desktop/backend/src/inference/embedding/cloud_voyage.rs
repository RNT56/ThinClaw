//! Voyage AI embedding backend.
//!
//! Best-in-class embeddings for RAG use cases.
//! `voyage-3` = 1024 dims, `voyage-3-large` = 1024 dims.

use crate::inference::embedding::EmbeddingBackend;
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct VoyageEmbeddingBackend {
    pub api_key: String,
    pub model: String,
    pub dims: usize,
}

impl VoyageEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        let model = model_override.unwrap_or_else(|| "voyage-3".to_string());
        let dims = 1024; // Voyage models all use 1024 dims
        Self {
            api_key,
            model,
            dims,
        }
    }

    pub fn large(api_key: String) -> Self {
        Self::new(api_key, Some("voyage-3-large".to_string()))
    }
}

#[async_trait]
impl EmbeddingBackend for VoyageEmbeddingBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "voyage".to_string(),
            display_name: format!("Voyage AI ({})", self.model),
            is_local: false,
            model_id: Some(self.model.clone()),
            available: true,
        }
    }

    async fn embed_batch(&self, texts: Vec<String>) -> InferenceResult<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();

        let response = client
            .post("https://api.voyageai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "input": texts,
                "model": self.model,
                "input_type": "document"
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Voyage embedding failed: {}", e)))?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid Voyage AI API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Voyage embedding error ({}): {}",
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
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
