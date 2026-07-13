//! Cohere embedding backend.
//!
//! `embed-multilingual-v3.0` = 1024 dims.

use crate::inference::embedding::{validate_embedding_batch, EmbeddingBackend};
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

    async fn embed_with_input_type(
        &self,
        texts: Vec<String>,
        input_type: &str,
    ) -> InferenceResult<Vec<Vec<f32>>> {
        let expected_count = texts.len();
        let response = reqwest::Client::new()
            .post("https://api.cohere.com/v1/embed")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "texts": texts,
                "model": self.model,
                "input_type": input_type,
                "embedding_types": ["float"]
            }))
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Cohere embedding failed: {error}"))
            })?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid Cohere API key"));
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Cohere embedding error ({status}): {text}"
            )));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|error| InferenceError::provider(format!("Parse error: {error}")))?;
        let embeddings = result
            .get("embeddings")
            .and_then(|embeddings| embeddings.get("float"))
            .and_then(|float| float.as_array())
            .ok_or_else(|| InferenceError::provider("Unexpected Cohere response format"))?;
        let results = embeddings
            .iter()
            .map(|embedding| {
                embedding
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value.as_f64().map(|value| value as f32))
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .collect();
        validate_embedding_batch(results, expected_count, self.dimensions(), "Cohere")
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
        self.embed_with_input_type(texts, "search_document").await
    }

    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        self.embed_with_input_type(vec![text], "search_query")
            .await?
            .pop()
            .ok_or_else(|| InferenceError::provider("Cohere returned no query embedding"))
    }

    fn dimensions(&self) -> usize {
        1024
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
