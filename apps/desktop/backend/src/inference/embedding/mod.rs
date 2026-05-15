//! Embedding backend trait and types.

pub mod cloud_cohere;
pub mod cloud_gemini;
pub mod cloud_openai;
pub mod cloud_voyage;
pub mod local;

use super::{BackendInfo, InferenceResult};
use async_trait::async_trait;

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

    /// The output dimensionality of embeddings from this backend.
    fn dimensions(&self) -> usize;

    /// The model identifier used by this backend.
    fn model_name(&self) -> &str;
}
