//! Gemini embedding backend.
//!
//! Uses `gemini-embedding-001` with an explicit 768-dimensional output.

use crate::inference::embedding::{
    bounded_embedding_json, cloud_embedding_dimensions, embedding_http_client,
    embedding_status_error, normalize_embedding_response, validate_cloud_embedding_config,
    validate_embedding_request, EmbeddingBackend, EmbeddingPurpose,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct GeminiEmbeddingBackend {
    pub api_key: String,
    pub model: String,
    pub dims: usize,
}

impl GeminiEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        let model = model_override.unwrap_or_else(|| "gemini-embedding-001".to_string());
        let dims = cloud_embedding_dimensions("gemini", &model).unwrap_or(0);
        Self {
            api_key,
            model,
            dims,
        }
    }

    async fn embed_with_purpose(
        &self,
        texts: Vec<String>,
        purpose: EmbeddingPurpose,
    ) -> InferenceResult<Vec<Vec<f32>>> {
        validate_embedding_request(&texts)?;
        validate_cloud_embedding_config(&self.api_key, &self.model)?;
        if self.dims == 0 {
            return Err(InferenceError::config(
                "The configured Gemini embedding model is not supported",
            ));
        }
        let input_count = texts.len();
        let client = embedding_http_client(false)?;
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:batchEmbedContents",
            self.model
        );
        let task_type = match purpose {
            EmbeddingPurpose::Document => "RETRIEVAL_DOCUMENT",
            EmbeddingPurpose::Query => "RETRIEVAL_QUERY",
        };
        let requests: Vec<serde_json::Value> = texts
            .iter()
            .map(|text| {
                serde_json::json!({
                    "model": format!("models/{}", self.model),
                    "content": { "parts": [{ "text": text }] },
                    "embedContentConfig": {
                        "taskType": task_type,
                        "autoTruncate": false,
                        "outputDimensionality": self.dims,
                    },
                })
            })
            .collect();

        let response = client
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(&serde_json::json!({ "requests": requests }))
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Gemini embedding request failed: {error}"))
            })?;

        if !response.status().is_success() {
            return Err(embedding_status_error("Gemini", response.status()));
        }

        #[derive(serde::Deserialize)]
        struct BatchResponse {
            embeddings: Vec<EmbeddingObject>,
        }
        #[derive(serde::Deserialize)]
        struct EmbeddingObject {
            values: Vec<f32>,
        }

        let result: BatchResponse = bounded_embedding_json(response).await?;
        normalize_embedding_response(
            result
                .embeddings
                .into_iter()
                .map(|embedding| (None, embedding.values))
                .collect(),
            input_count,
            self.dims,
        )
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
        self.embed_with_purpose(texts, EmbeddingPurpose::Document)
            .await
    }

    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        let mut result = self
            .embed_with_purpose(vec![text], EmbeddingPurpose::Query)
            .await?;
        result
            .pop()
            .ok_or_else(|| InferenceError::provider("Empty Gemini embedding response"))
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_current_supported_model() {
        let backend = GeminiEmbeddingBackend::new("key".into(), None);
        assert_eq!(backend.model_name(), "gemini-embedding-001");
        assert_eq!(backend.dimensions(), 768);
        assert_eq!(
            GeminiEmbeddingBackend::new("key".into(), Some("unknown".into())).dimensions(),
            0
        );
    }
}
