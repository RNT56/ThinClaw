//! Embedding backend trait and types.

pub mod cloud_cohere;
pub mod cloud_gemini;
pub mod cloud_openai;
pub mod cloud_voyage;
pub mod local;

use super::{BackendInfo, InferenceResult};
use async_trait::async_trait;

pub(crate) const MAX_EMBEDDING_DIMENSIONS: usize = 65_536;

/// Validate provider output before it can reach SQLite or a vector index.
pub(crate) fn validate_embedding_batch(
    embeddings: Vec<Vec<f32>>,
    expected_count: usize,
    expected_dimensions: usize,
    provider: &str,
) -> InferenceResult<Vec<Vec<f32>>> {
    if !(1..=MAX_EMBEDDING_DIMENSIONS).contains(&expected_dimensions) {
        return Err(super::InferenceError::provider(format!(
            "{provider} declared an invalid embedding dimension: {expected_dimensions}"
        )));
    }
    if embeddings.len() != expected_count {
        return Err(super::InferenceError::provider(format!(
            "{provider} returned {} embeddings for {expected_count} inputs",
            embeddings.len()
        )));
    }
    for (index, embedding) in embeddings.iter().enumerate() {
        if embedding.len() != expected_dimensions {
            return Err(super::InferenceError::provider(format!(
                "{provider} embedding {index} has {} dimensions; expected {expected_dimensions}",
                embedding.len()
            )));
        }
        if embedding.iter().any(|value| !value.is_finite()) {
            return Err(super::InferenceError::provider(format!(
                "{provider} embedding {index} contains a non-finite value"
            )));
        }
    }
    Ok(embeddings)
}

/// Embedding backend — local or cloud.
#[async_trait]
pub trait EmbeddingBackend: Send + Sync {
    /// Information about this backend.
    fn info(&self) -> BackendInfo;

    /// Embed a batch of texts.  Returns one embedding vector per input.
    async fn embed_batch(&self, texts: Vec<String>) -> InferenceResult<Vec<Vec<f32>>>;

    /// Embed a single text.  Default: calls `embed_batch` with one input.
    async fn embed(&self, text: String) -> InferenceResult<Vec<f32>> {
        let mut results = self.embed_batch(vec![text]).await?;
        results
            .pop()
            .ok_or_else(|| super::InferenceError::provider("Empty embedding response"))
    }

    /// Embed a retrieval query. Providers with asymmetric retrieval models
    /// override this to use their query-specific task/input type.
    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        self.embed(text).await
    }

    /// The output dimensionality of embeddings from this backend.
    fn dimensions(&self) -> usize;

    /// The model identifier used by this backend.
    fn model_name(&self) -> &str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_embedding_shape_and_values() {
        assert!(validate_embedding_batch(vec![vec![0.0, 1.0]], 1, 2, "test").is_ok());
        assert!(validate_embedding_batch(vec![], 1, 2, "test").is_err());
        assert!(validate_embedding_batch(vec![vec![0.0]], 1, 2, "test").is_err());
        assert!(validate_embedding_batch(vec![vec![f32::NAN, 0.0]], 1, 2, "test").is_err());
    }
}
