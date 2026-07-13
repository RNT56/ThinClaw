//! Gemini embedding backend.
//!
//! Uses `gemini-embedding-2` at 768 dimensions for a stable local index size.

use crate::inference::embedding::{validate_embedding_batch, EmbeddingBackend};
use crate::inference::{BackendInfo, InferenceError, InferenceResult};
use async_trait::async_trait;

pub(crate) fn is_retired_embedding_model(model: &str) -> bool {
    matches!(
        model,
        "text-embedding-004" | "embedding-001" | "gemini-embedding-001"
    )
}

pub struct GeminiEmbeddingBackend {
    pub api_key: String,
    pub model: String,
}

impl GeminiEmbeddingBackend {
    pub fn new(api_key: String, model_override: Option<String>) -> Self {
        let model = match model_override.as_deref() {
            None => "gemini-embedding-2".to_string(),
            Some(model) if is_retired_embedding_model(model) => "gemini-embedding-2".to_string(),
            Some(model) => model.to_string(),
        };
        Self { api_key, model }
    }

    async fn embed_for_retrieval(
        &self,
        texts: Vec<String>,
        is_query: bool,
    ) -> InferenceResult<Vec<Vec<f32>>> {
        let expected_count = texts.len();
        let client = reqwest::Client::new();
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:batchEmbedContents",
            self.model
        );
        let uses_instruction_prefix = self.model.contains("gemini-embedding-2");
        let requests: Vec<serde_json::Value> = texts
            .iter()
            .map(|text| {
                let prepared_text = if uses_instruction_prefix {
                    if is_query {
                        format!("task: search result | query: {text}")
                    } else {
                        format!("title: none | text: {text}")
                    }
                } else {
                    text.clone()
                };
                let mut request = serde_json::json!({
                    "model": format!("models/{}", self.model),
                    "content": { "parts": [{ "text": prepared_text }] },
                    "outputDimensionality": self.dimensions()
                });
                if !uses_instruction_prefix {
                    request["taskType"] = serde_json::Value::String(
                        if is_query {
                            "RETRIEVAL_QUERY"
                        } else {
                            "RETRIEVAL_DOCUMENT"
                        }
                        .to_string(),
                    );
                }
                request
            })
            .collect();
        let response = client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
            .json(&serde_json::json!({ "requests": requests }))
            .send()
            .await
            .map_err(|error| {
                InferenceError::network(format!("Gemini embedding request failed: {error}"))
            })?;

        if response.status() == 401 || response.status() == 403 {
            return Err(InferenceError::auth("Invalid Gemini API key"));
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(InferenceError::provider(format!(
                "Gemini embedding error ({status}): {text}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct BatchResponse {
            embeddings: Vec<EmbeddingObj>,
        }
        #[derive(serde::Deserialize)]
        struct EmbeddingObj {
            values: Vec<f32>,
        }
        let result: BatchResponse = response
            .json()
            .await
            .map_err(|error| InferenceError::provider(format!("Failed to parse: {error}")))?;
        validate_embedding_batch(
            result
                .embeddings
                .into_iter()
                .map(|embedding| embedding.values)
                .collect(),
            expected_count,
            self.dimensions(),
            "Gemini",
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
        self.embed_for_retrieval(texts, false).await
    }

    async fn embed_query(&self, text: String) -> InferenceResult<Vec<f32>> {
        self.embed_for_retrieval(vec![text], true)
            .await?
            .pop()
            .ok_or_else(|| InferenceError::provider("Gemini returned no query embedding"))
    }

    fn dimensions(&self) -> usize {
        768
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_embedding_models_migrate_to_current_default() {
        for retired in [
            "text-embedding-004",
            "embedding-001",
            "gemini-embedding-001",
        ] {
            let backend = GeminiEmbeddingBackend::new("key".into(), Some(retired.into()));
            assert_eq!(backend.model, "gemini-embedding-2");
        }
        let custom = GeminiEmbeddingBackend::new("key".into(), Some("custom-model".into()));
        assert_eq!(custom.model, "custom-model");
    }
}
