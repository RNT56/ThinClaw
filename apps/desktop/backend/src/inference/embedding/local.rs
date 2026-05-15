//! Local embedding backend — wraps existing llama-server / MLX embed server.

use crate::inference::embedding::EmbeddingBackend;
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
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/v1/embeddings", self.port);

        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .json(&serde_json::json!({
                "input": texts,
                "model": "default"
            }))
            .send()
            .await
            .map_err(|e| InferenceError::network(format!("Embedding request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Embedding server error ({}): {}",
                status, text
            )));
        }

        #[derive(serde::Deserialize)]
        struct EmbeddingResponse {
            data: Vec<EmbeddingData>,
        }
        #[derive(serde::Deserialize)]
        struct EmbeddingData {
            embedding: Vec<f32>,
        }

        let result: EmbeddingResponse = response
            .json()
            .await
            .map_err(|e| InferenceError::provider(format!("Failed to parse embedding: {}", e)))?;

        Ok(result.data.into_iter().map(|d| d.embedding).collect())
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }
}
