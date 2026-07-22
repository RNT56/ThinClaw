//! Cohere embedding backend.
//!
//! Uses Cohere's current v2 Embed API.

use crate::inference::embedding::{
    bounded_embedding_json, cloud_embedding_dimensions, embedding_http_client,
    embedding_status_error, normalize_embedding_response, validate_cloud_embedding_config,
    validate_embedding_request, EmbeddingBackend, EmbeddingPurpose,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct CohereEmbeddingBackend {
    pub api_key: String,
    pub model: String,
    pub dims: usize,
}

impl CohereEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        let model = model_override.unwrap_or_else(|| "embed-v4.0".to_string());
        let dims = cloud_embedding_dimensions("cohere", &model).unwrap_or(0);
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
                "The configured Cohere embedding model is not supported",
            ));
        }
        let input_count = texts.len();
        let input_type = match purpose {
            EmbeddingPurpose::Document => "search_document",
            EmbeddingPurpose::Query => "search_query",
        };
        let mut body = serde_json::json!({
            "texts": texts,
            "model": self.model,
            "input_type": input_type,
            "embedding_types": ["float"],
            "truncate": "NONE",
        });
        if self.model == "embed-v4.0" {
            body["output_dimension"] = serde_json::json!(self.dims);
        }

        let response = embedding_http_client(false)?
            .post("https://api.cohere.com/v2/embed")
            .bearer_auth(&self.api_key)
            .header("X-Client-Name", "ThinClaw")
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Cohere embedding request failed: {error}"))
            })?;

        if !response.status().is_success() {
            return Err(embedding_status_error("Cohere", response.status()));
        }

        #[derive(serde::Deserialize)]
        struct Response {
            embeddings: Embeddings,
        }
        #[derive(serde::Deserialize)]
        struct Embeddings {
            float: Vec<Vec<f32>>,
        }

        let result: Response = bounded_embedding_json(response).await?;
        normalize_embedding_response(
            result
                .embeddings
                .float
                .into_iter()
                .map(|embedding| (None, embedding))
                .collect(),
            input_count,
            self.dims,
        )
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
        self.embed_with_purpose(texts, EmbeddingPurpose::Document)
            .await
    }

    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        let mut result = self
            .embed_with_purpose(vec![text], EmbeddingPurpose::Query)
            .await?;
        result
            .pop()
            .ok_or_else(|| InferenceError::provider("Empty Cohere embedding response"))
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
    fn maps_current_and_legacy_supported_dimensions() {
        assert_eq!(
            CohereEmbeddingBackend::new("key".into(), None).dimensions(),
            1536
        );
        assert_eq!(
            CohereEmbeddingBackend::new("key".into(), Some("embed-multilingual-v3.0".into()))
                .dimensions(),
            1024
        );
        assert_eq!(
            CohereEmbeddingBackend::new("key".into(), Some("unknown".into())).dimensions(),
            0
        );
    }
}
