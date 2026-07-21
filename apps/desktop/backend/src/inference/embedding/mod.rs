//! Embedding backend trait and types.

pub mod cloud_cohere;
pub mod cloud_gemini;
pub mod cloud_openai;
pub mod cloud_voyage;
pub mod local;

use super::{BackendInfo, InferenceResult};
use async_trait::async_trait;

// Cohere's current synchronous endpoint accepts at most 96 text inputs. Keeping
// the shared ceiling at the strictest supported provider makes every backend
// interchangeable without provider-specific partial requests.
pub const MAX_EMBEDDING_BATCH: usize = 96;
pub const MAX_EMBEDDING_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_EMBEDDING_INPUT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_EMBEDDING_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_EMBEDDING_DIMENSIONS: usize = 65_536;
pub const EMBEDDING_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2 * 60);

pub fn validate_embedding_request(texts: &[String]) -> InferenceResult<()> {
    if texts.is_empty() {
        return Err(super::InferenceError::config(
            "Embedding input batch is empty",
        ));
    }
    if texts.len() > MAX_EMBEDDING_BATCH {
        return Err(super::InferenceError::config(format!(
            "Embedding batch exceeds the {MAX_EMBEDDING_BATCH}-item limit"
        )));
    }
    let mut total = 0_usize;
    for text in texts {
        if text.len() > MAX_EMBEDDING_TEXT_BYTES {
            return Err(super::InferenceError::config(format!(
                "Embedding input exceeds the {MAX_EMBEDDING_TEXT_BYTES}-byte per-item limit"
            )));
        }
        total = total.saturating_add(text.len());
        if total > MAX_EMBEDDING_INPUT_BYTES {
            return Err(super::InferenceError::config(format!(
                "Embedding batch exceeds the {MAX_EMBEDDING_INPUT_BYTES}-byte limit"
            )));
        }
    }
    Ok(())
}

pub fn embedding_http_client(local_only: bool) -> InferenceResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(EMBEDDING_REQUEST_TIMEOUT);
    if local_only {
        builder = builder.no_proxy();
    }
    builder.build().map_err(|error| {
        super::InferenceError::network(format!("Could not build embedding client: {error}"))
    })
}

pub fn validate_cloud_embedding_config(api_key: &str, model: &str) -> InferenceResult<()> {
    if api_key.is_empty()
        || api_key.len() > 4096
        || api_key
            .chars()
            .any(|character| character.is_control() || character == '\r' || character == '\n')
    {
        return Err(super::InferenceError::auth(
            "The embedding provider credential is missing or invalid",
        ));
    }
    if model.is_empty()
        || model.len() > 128
        || !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(super::InferenceError::config(
            "The embedding model identifier is invalid",
        ));
    }
    Ok(())
}

/// Return the documented output size for cloud models that this build knows
/// how to call. Unknown names are deliberately rejected instead of guessing a
/// dimension and corrupting a vector index.
pub fn cloud_embedding_dimensions(provider: &str, model: &str) -> Option<usize> {
    match (provider, model) {
        ("openai", "text-embedding-3-small" | "text-embedding-ada-002") => Some(1536),
        ("openai", "text-embedding-3-large") => Some(3072),
        ("gemini", "gemini-embedding-001") => Some(768),
        ("cohere", "embed-v4.0") => Some(1536),
        ("cohere", "embed-multilingual-v3.0" | "embed-english-v3.0") => Some(1024),
        (
            "voyage",
            "voyage-4-large"
            | "voyage-4"
            | "voyage-4-lite"
            | "voyage-code-3"
            | "voyage-finance-2"
            | "voyage-law-2"
            | "voyage-3-large"
            | "voyage-3.5"
            | "voyage-3.5-lite"
            | "voyage-3"
            | "voyage-multilingual-2"
            | "voyage-large-2-instruct"
            | "voyage-2",
        ) => Some(1024),
        ("voyage", "voyage-code-2" | "voyage-large-2") => Some(1536),
        ("voyage", "voyage-3-lite") => Some(512),
        _ => None,
    }
}

pub fn embedding_status_error(
    provider: &str,
    status: reqwest::StatusCode,
) -> super::InferenceError {
    match status.as_u16() {
        401 | 403 | 498 => super::InferenceError::auth(format!(
            "{provider} rejected the embedding provider credential"
        )),
        404 => super::InferenceError::model_not_found(format!(
            "{provider} could not find the configured embedding model"
        )),
        429 => super::InferenceError::rate_limited(format!(
            "{provider} rate-limited the embedding request"
        )),
        _ => super::InferenceError::provider(format!(
            "{provider} embedding request failed with HTTP status {status}"
        )),
    }
}

pub async fn bounded_embedding_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> InferenceResult<T> {
    thinclaw_core::http_response::bounded_json(response, MAX_EMBEDDING_RESPONSE_BYTES)
        .await
        .map_err(|error| {
            super::InferenceError::provider(format!("Invalid embedding response: {error}"))
        })
}

pub fn normalize_embedding_response(
    data: Vec<(Option<usize>, Vec<f32>)>,
    input_count: usize,
    expected_dimensions: usize,
) -> InferenceResult<Vec<Vec<f32>>> {
    if data.len() != input_count {
        return Err(super::InferenceError::provider(format!(
            "Embedding response contained {} vectors for {input_count} inputs",
            data.len()
        )));
    }

    let indexed = data.iter().any(|(index, _)| index.is_some());
    let vectors = if indexed {
        if data.iter().any(|(index, _)| index.is_none()) {
            return Err(super::InferenceError::provider(
                "Embedding response mixed indexed and unindexed vectors",
            ));
        }
        let mut ordered = vec![None; input_count];
        for (index, vector) in data {
            let index = index.expect("indices were validated above");
            if index >= input_count || ordered[index].is_some() {
                return Err(super::InferenceError::provider(
                    "Embedding response contained an invalid or duplicate index",
                ));
            }
            ordered[index] = Some(vector);
        }
        ordered
            .into_iter()
            .map(|vector| {
                vector.ok_or_else(|| {
                    super::InferenceError::provider("Embedding response omitted an input index")
                })
            })
            .collect::<InferenceResult<Vec<_>>>()?
    } else {
        data.into_iter().map(|(_, vector)| vector).collect()
    };

    for vector in &vectors {
        if vector.is_empty() || vector.len() > MAX_EMBEDDING_DIMENSIONS {
            return Err(super::InferenceError::provider(
                "Embedding response contained an invalid vector dimension",
            ));
        }
        if expected_dimensions > 0 && vector.len() != expected_dimensions {
            return Err(super::InferenceError::provider(format!(
                "Embedding response dimension {} did not match the expected {expected_dimensions}",
                vector.len()
            )));
        }
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(super::InferenceError::provider(
                "Embedding response contained a non-finite value",
            ));
        }
    }
    Ok(vectors)
}

/// Retrieval role for providers that tune query and document vectors
/// differently. Local/OpenAI-compatible backends can ignore this distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingPurpose {
    Document,
    Query,
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

    /// Embed corpus content for storage in a retrieval index.
    async fn embed_document(&self, text: String) -> InferenceResult<Vec<f32>> {
        self.embed(text).await
    }

    /// Embed a retrieval query. Providers with query/document task modes should
    /// override this method so that query vectors match their documented usage.
    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        self.embed(text).await
    }

    /// The output dimensionality of embeddings from this backend.
    fn dimensions(&self) -> usize;

    /// The model identifier used by this backend.
    fn model_name(&self) -> &str;

    /// Stable identity used to invalidate derived vector indices when the
    /// provider/model changes, including changes that keep the same dimension.
    fn profile_id(&self) -> String {
        format!(
            "{}:{}:{}",
            self.info().id,
            self.model_name(),
            self.dimensions()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_oversized_embedding_inputs() {
        assert!(validate_embedding_request(&[]).is_err());
        assert!(validate_embedding_request(&vec![String::new(); MAX_EMBEDDING_BATCH + 1]).is_err());
        assert!(validate_embedding_request(&["x".repeat(MAX_EMBEDDING_TEXT_BYTES + 1)]).is_err());
    }

    #[test]
    fn rejects_header_injection_and_unsafe_model_ids() {
        assert!(validate_cloud_embedding_config("key\nvalue", "model").is_err());
        assert!(validate_cloud_embedding_config("key", "https://example.test/model").is_err());
        assert!(validate_cloud_embedding_config("key", "model-name_1.0").is_ok());
    }

    #[test]
    fn never_guesses_unknown_cloud_dimensions() {
        assert_eq!(
            cloud_embedding_dimensions("openai", "text-embedding-3-large"),
            Some(3072)
        );
        assert_eq!(
            cloud_embedding_dimensions("voyage", "voyage-3-lite"),
            Some(512)
        );
        assert_eq!(cloud_embedding_dimensions("openai", "future-model"), None);
    }

    #[test]
    fn validates_and_reorders_indexed_vectors() {
        let vectors = normalize_embedding_response(
            vec![(Some(1), vec![3.0, 4.0]), (Some(0), vec![1.0, 2.0])],
            2,
            2,
        )
        .unwrap();
        assert_eq!(vectors, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
        assert!(normalize_embedding_response(vec![(None, vec![f32::NAN])], 1, 1).is_err());
    }
}
