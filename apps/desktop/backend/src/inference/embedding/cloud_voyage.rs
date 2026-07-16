//! Voyage AI embedding backend.
//!
//! Best-in-class embeddings for RAG use cases.
//! `voyage-4` = 1024 dims by default.

use crate::inference::embedding::{validate_embedding_batch, EmbeddingBackend};
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
        let dims = 1024;
        Self {
            api_key,
            model,
            dims,
        }
    }

    pub fn large(api_key: String) -> Self {
        Self::new(api_key, Some("voyage-4-large".to_string()))
    }

    async fn embed_with_input_type(
        &self,
        texts: Vec<String>,
        input_type: &str,
    ) -> InferenceResult<Vec<Vec<f32>>> {
        let expected_count = texts.len();
        let response = reqwest::Client::new()
            .post("https://api.voyageai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "input": texts,
                "model": self.model,
                "input_type": input_type,
                "output_dimension": self.dims
            }))
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Voyage embedding failed: {error}"))
            })?;

        if response.status() == 401 {
            return Err(InferenceError::auth("Invalid Voyage AI API key"));
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Voyage embedding error ({status}): {text}"
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
            .map_err(|error| InferenceError::provider(format!("Parse error: {error}")))?;
        validate_embedding_batch(
            result.data.into_iter().map(|data| data.embedding).collect(),
            expected_count,
            self.dims,
            "Voyage",
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
        self.embed_with_input_type(texts, "document").await
    }

    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        self.embed_with_input_type(vec![text], "query")
            .await?
            .pop()
            .ok_or_else(|| InferenceError::provider("Voyage returned no query embedding"))
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}
