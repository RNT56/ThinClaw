//! Cohere embedding backend.
//!
//! `embed-multilingual-v3.0` = 1024 dims.

use crate::inference::embedding::EmbeddingBackend;
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct CohereEmbeddingBackend {
    pub api_key: String,
    pub model: String,
}

impl CohereEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        Self {
            api_key,
            model: model_override.unwrap_or_else(|| "embed-multilingual-v3.0".to_string()),
        }
    }
}

#[async_trait]
impl EmbeddingBackend for CohereEmbeddingBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "cohere".to_string(),
            display_name: format!("Cohere ({})", self.model),
            is_local: false,
            model_id: Some(self.model.clone()),
            available: true,
        }
    }

    async fn embed_batch(&self, texts: Vec<String>) -> InferenceResult<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();

        let response = client
            .post("https://api.cohere.com/v1/embed")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "texts": texts,
                "model": self.model,
                "input_type": "search_document",
                "embedding_types": ["float"]
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Cohere embedding failed: {}", e)))?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid Cohere API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Cohere embedding error ({}): {}",
                status, text
            )));
        }

        // Cohere returns: { embeddings: { float: [[f32]] } }
        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Parse error: {}", e)))?;

        let embeddings = result
            .get("embeddings")
            .and_then(|e| e.get("float"))
            .and_then(|f| f.as_array())
            .ok_or_else(|| InferenceError::provider("Unexpected Cohere response format"))?;

        let mut results = Vec::with_capacity(embeddings.len());
        for emb in embeddings {
            let vec: Vec<f32> = emb
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect()
                })
                .unwrap_or_default();
            results.push(vec);
        }

        Ok(results)
    }

    fn dimensions(&self) -> usize {
        1024
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
