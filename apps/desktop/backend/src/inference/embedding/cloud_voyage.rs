//! Voyage AI embedding backend.
//!
//! Best-in-class embeddings for RAG use cases.
//! Defaults to the current `voyage-4` model (1024 dimensions).

use crate::inference::embedding::{
    bounded_embedding_json, cloud_embedding_dimensions, embedding_http_client,
    embedding_status_error, normalize_embedding_response, validate_cloud_embedding_config,
    validate_embedding_request, EmbeddingBackend, EmbeddingPurpose,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub struct VoyageEmbeddingBackend {
    pub api_key: String,
    pub model: String,
    pub dims: usize,
}

impl VoyageEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        let model = model_override.unwrap_or_else(|| "voyage-4".to_string());
        let dims = cloud_embedding_dimensions("voyage", &model).unwrap_or(0);
        Self {
            api_key,
            model,
            dims,
        }
    }

    pub fn large(api_key: String) -> Self {
        Self::new(api_key, Some("voyage-4-large".to_string()))
    }

    fn supports_flexible_dimensions(&self) -> bool {
        matches!(
            self.model.as_str(),
            "voyage-4-large"
                | "voyage-4"
                | "voyage-4-lite"
                | "voyage-code-3"
                | "voyage-3-large"
                | "voyage-3.5"
                | "voyage-3.5-lite"
        )
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
                "The configured Voyage embedding model is not supported",
            ));
        }
        let input_count = texts.len();
        let input_type = match purpose {
            EmbeddingPurpose::Document => "document",
            EmbeddingPurpose::Query => "query",
        };
        let mut body = serde_json::json!({
            "input": texts,
            "model": self.model,
            "input_type": input_type,
            "truncation": false,
            "output_dtype": "float",
        });
        if self.supports_flexible_dimensions() {
            body["output_dimension"] = serde_json::json!(self.dims);
        }

        let response = embedding_http_client(false)?
            .post("https://api.voyageai.com/v1/embeddings")
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Voyage embedding request failed: {error}"))
            })?;

        if !response.status().is_success() {
            return Err(embedding_status_error("Voyage AI", response.status()));
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
        self.embed_with_purpose(texts, EmbeddingPurpose::Document)
            .await
    }

    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        let mut result = self
            .embed_with_purpose(vec![text], EmbeddingPurpose::Query)
            .await?;
        result
            .pop()
            .ok_or_else(|| InferenceError::provider("Empty Voyage embedding response"))
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
    fn maps_current_and_fixed_dimension_models() {
        let default = VoyageEmbeddingBackend::new("key".into(), None);
        assert_eq!(default.model_name(), "voyage-4");
        assert_eq!(default.dimensions(), 1024);
        assert_eq!(
            VoyageEmbeddingBackend::large("key".into()).dimensions(),
            1024
        );
        assert_eq!(
            VoyageEmbeddingBackend::new("key".into(), Some("voyage-code-2".into())).dimensions(),
            1536
        );
        assert_eq!(
            VoyageEmbeddingBackend::new("key".into(), Some("voyage-3-lite".into())).dimensions(),
            512
        );
        assert_eq!(
            VoyageEmbeddingBackend::new("key".into(), Some("unknown".into())).dimensions(),
            0
        );
    }
}
