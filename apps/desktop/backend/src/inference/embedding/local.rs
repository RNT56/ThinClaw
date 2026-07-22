//! Local embedding backend — wraps existing llama-server / MLX embed server.

use crate::inference::embedding::{
    bounded_embedding_json, embedding_http_client, normalize_embedding_response,
    validate_embedding_request, EmbeddingBackend,
};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

/// Local embedding backend using the llama-server or MLX embedding sidecar.
///
/// Delegates to `POST http://127.0.0.1:{port}/v1/embeddings`.
pub struct LocalEmbeddingBackend {
    /// Port of the running embedding server.
    pub port: u16,
    /// Auth token.
    pub token: String,
    /// Model name.
    pub model_name: String,
    /// Output dimensions.
    pub dimensions: usize,
    /// Opaque fingerprint of the selected local model artifact.
    pub profile_id: String,
}

#[async_trait]
impl EmbeddingBackend for LocalEmbeddingBackend {
    fn info(&self) -> BackendInfo {
        BackendInfo {
            id: "local".to_string(),
            display_name: "Local Embedding Server".to_string(),
            is_local: true,
            model_id: Some(self.model_name.clone()),
            available: true,
        }
    }

    async fn embed_batch(&self, texts: Vec<String>) -> InferenceResult<Vec<Vec<f32>>> {
        validate_embedding_request(&texts)?;
        let input_count = texts.len();
        let client = embedding_http_client(true)?;
        let url = format!("http://127.0.0.1:{}/v1/embeddings", self.port);

        let response = client
            .post(&url)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({
                "input": texts,
                "model": "thinclaw-embedding"
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Embedding request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(InferenceError::provider(format!(
                "Embedding server returned HTTP {status}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingData>,
        }
        #[derive(serde::Deserialize)]
        struct EmbeddingData {
            #[serde(default)]
            index: Option<usize>,
            embedding: Vec<f32>,
        }

        let result: EmbeddingResponse = bounded_embedding_json(response).await?;

        normalize_embedding_response(
            result
                .data
                .into_iter()
                .map(|data| (data.index, data.embedding))
                .collect(),
            input_count,
            self.dimensions,
        )
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn profile_id(&self) -> String {
        format!("local:{}:{}", self.profile_id, self.dimensions)
    }
}
