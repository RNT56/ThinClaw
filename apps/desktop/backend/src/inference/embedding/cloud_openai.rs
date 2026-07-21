//! OpenAI embedding backend.
//!
//! Supports `text-embedding-3-small` (1536 dims) and `text-embedding-3-large` (3072 dims).

use crate::inference::embedding::{
    bounded_embedding_json, cloud_embedding_dimensions, embedding_http_client,
    embedding_status_error, normalize_embedding_response, validate_cloud_embedding_config,
    validate_embedding_request, EmbeddingBackend,
};
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
        let dims = cloud_embedding_dimensions("openai", &model).unwrap_or(0);
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
        validate_embedding_request(&texts)?;
        validate_cloud_embedding_config(&self.api_key, &self.model)?;
        if self.dims == 0 {
            return Err(InferenceError::config(
                "The configured OpenAI embedding model is not supported",
            ));
        }
        let input_count = texts.len();
        let client = embedding_http_client(false)?;
        let body = if self.model == "text-embedding-ada-002" {
            serde_json::json!({
                "input": texts,
                "model": self.model,
            })
        } else {
            serde_json::json!({
                "input": texts,
                "model": self.model,
                "dimensions": self.dims,
                "encoding_format": "float",
            })
        };

        let response = client
            .post("https://api.openai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("OpenAI embedding request failed: {error}"))
            })?;

        if !response.status().is_success() {
            return Err(embedding_status_error("OpenAI", response.status()));
        }

        #[derive(serde::Deserialize)]
        struct Response {
            data: Vec<Data>,
        }
        #[derive(serde::Deserialize)]
        struct Data {
            index: Option<usize>,
            embedding: Vec<f32>,
        }

        let result: Response = bounded_embedding_json(response).await?;
        normalize_embedding_response(
            result
                .data
                .into_iter()
                .map(|item| (item.index, item.embedding))
                .collect(),
            input_count,
            self.dims,
        )
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
    fn maps_only_supported_model_dimensions() {
        assert_eq!(
            OpenAiEmbeddingBackend::small("key".into()).dimensions(),
            1536
        );
        assert_eq!(
            OpenAiEmbeddingBackend::large("key".into()).dimensions(),
            3072
        );
        assert_eq!(
            OpenAiEmbeddingBackend::new("key".into(), Some("unknown".into())).dimensions(),
            0
        );
    }
}
