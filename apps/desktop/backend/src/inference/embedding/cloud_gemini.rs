//! Gemini embedding backend.
//!
//! Uses `text-embedding-004` (768 dims). Free within quota.

use crate::inference::embedding::EmbeddingBackend;
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct GeminiEmbeddingBackend {
    pub api_key: String,
    pub model: String,
}

impl GeminiEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        Self {
            api_key,
            model: model_override.unwrap_or_else(|| "text-embedding-004".to_string()),
        }
    }
}

#[async_trait]
impl EmbeddingBackend for GeminiEmbeddingBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "gemini".to_string(),
            display_name: "Gemini Embeddings".to_string(),
            is_local: false,
            model_id: Some(self.model.clone()),
            available: true,
        }
    }

    async fn embed_batch(&self, texts: Vec<String>) -> InferenceResult<Vec<Vec<f32>>> {
        let client = reqwest::Client::new();
        let mut all_embeddings = Vec::with_capacity(texts.len());

        // Gemini embedding API doesn't support batching in the same way as OpenAI.
        // We need to call embedContent per text, or use batchEmbedContents.
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:batchEmbedContents?key={}",
            self.model, self.api_key
        );

        let requests: Vec<serde_json::Value> = texts
            .iter()
            .map(|text| {
                serde_json::json!({
                    "model": format!("models/{}", self.model),
                    "content": { "parts": [{ "text": text }] }
                })
            })
            .collect();

        let response = client
            .post(&url)
            .json(&serde_json::json!({ "requests": requests }))
            .send()
            .await
            .map_err(|e| {
                InferenceError::network(format!("Gemini embedding request failed: {}", e))
            })?;

        if response.status() == 401 || response.status() == 403 {
            return Err(InferenceError::auth("Invalid Gemini API key"));
        }

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Gemini embedding error ({}): {}",
                status, text
            )));
        }

        #[derive(serde::Deserialize)]
        struct BatchResponse {
            embeddings: Vec<EmbeddingObj>,
        }
        #[derive(serde::Deserialize)]
        struct EmbeddingObj {
            values: Vec<f32>,
        }

        let result: BatchResponse = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Failed to parse: {}", e)))?;

        for emb in result.embeddings {
            all_embeddings.push(emb.values);
        }

        Ok(all_embeddings)
    }

    fn dimensions(&self) -> usize {
        768
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
