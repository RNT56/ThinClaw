use std::sync::Arc;

use async_trait::async_trait;

pub use thinclaw_config::embeddings::EmbeddingsConfig;

use crate::workspace::EmbeddingProvider;

#[async_trait]
pub trait EmbeddingsConfigProviderExt {
    async fn create_provider(&self) -> Option<Arc<dyn EmbeddingProvider>>;
}

#[async_trait]
impl EmbeddingsConfigProviderExt for EmbeddingsConfig {
    async fn create_provider(&self) -> Option<Arc<dyn EmbeddingProvider>> {
        if !self.enabled {
            tracing::info!("Embeddings disabled (set EMBEDDING_ENABLED=true to enable)");
            return None;
        }

        match self.provider.as_str() {
            "bedrock" => {
                #[cfg(feature = "bedrock")]
                {
                    let region =
                        std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
                    let profile = std::env::var("AWS_PROFILE").ok();
                    tracing::info!(
                        "Embeddings enabled via Bedrock (model: {}, region: {}, dim: {})",
                        self.model,
                        region,
                        self.dimension,
                    );
                    let provider = crate::workspace::BedrockEmbeddings::new(
                        region,
                        profile.as_deref(),
                        &self.model,
                        self.dimension,
                    )
                    .await;
                    Some(Arc::new(provider) as Arc<dyn EmbeddingProvider>)
                }
                #[cfg(not(feature = "bedrock"))]
                {
                    tracing::warn!(
                        "Embeddings configured for Bedrock but the `bedrock` feature is disabled. \
                         Rebuild with --features bedrock to enable."
                    );
                    None
                }
            }
            "ollama" => {
                tracing::info!(
                    "Embeddings enabled via Ollama (model: {}, url: {}, dim: {})",
                    self.model,
                    self.ollama_base_url,
                    self.dimension,
                );
                Some(Arc::new(
                    crate::workspace::OllamaEmbeddings::new(&self.ollama_base_url)
                        .with_model(&self.model, self.dimension),
                ))
            }
            _ => {
                if let Some(api_key) = self.openai_api_key() {
                    tracing::info!(
                        "Embeddings enabled via OpenAI (model: {}, dim: {})",
                        self.model,
                        self.dimension,
                    );
                    Some(Arc::new(crate::workspace::OpenAiEmbeddings::with_model(
                        api_key,
                        &self.model,
                        self.dimension,
                    )))
                } else {
                    tracing::warn!("Embeddings configured but OPENAI_API_KEY not set");
                    None
                }
            }
        }
    }
}
