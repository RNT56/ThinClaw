use super::*;

pub use thinclaw_agent::learning_provider_types::{
    LearningOutcome, ProviderHealthStatus, ProviderMemoryHit, ProviderPrefetchContext,
    ProviderReadiness, decorate_provider_status, extract_embedding, parse_custom_http_hits,
    parse_provider_hits, provider_configured_skipped_health_status, provider_context_refs,
    provider_disabled_status, provider_http_client_error_status,
    provider_http_request_error_status, provider_http_response_status,
    provider_missing_base_url_status, provider_required_status, render_provider_prompt_context,
};

#[async_trait]
pub trait MemoryProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn health(&self, settings: &LearningSettings) -> ProviderHealthStatus;
    async fn system_prompt_block(
        &self,
        _settings: &LearningSettings,
        _user_id: &str,
    ) -> Option<String> {
        None
    }
    async fn prefetch(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String> {
        self.recall(settings, user_id, query, limit).await
    }
    async fn recall(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<ProviderMemoryHit>, String>;
    async fn export_turn(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String>;
    fn render_prompt_context(&self, hits: &[ProviderMemoryHit]) -> Option<String> {
        render_provider_prompt_context(self.name(), hits)
    }
    async fn prefetch_session_context(
        &self,
        settings: &LearningSettings,
        user_id: &str,
    ) -> Option<String> {
        self.system_prompt_block(settings, user_id).await
    }
    async fn after_turn_sync(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.export_turn(settings, user_id, payload).await
    }
    async fn session_end_extract(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.export_turn(settings, user_id, payload).await
    }
    async fn mirror_workspace_write(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.export_turn(settings, user_id, payload).await
    }
    async fn pre_compress_hook(
        &self,
        settings: &LearningSettings,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        self.export_turn(settings, user_id, payload).await
    }
    async fn shutdown(&self, _settings: &LearningSettings) -> Result<(), String> {
        Ok(())
    }
    fn tool_extensions(&self) -> Vec<String> {
        vec![
            "external_memory_recall".to_string(),
            "external_memory_export".to_string(),
            "external_memory_status".to_string(),
        ]
    }
}

#[derive(Default)]
pub struct HonchoProvider;

#[derive(Default)]
pub struct ZepProvider;

#[derive(Default)]
pub struct CustomHttpProvider;

#[derive(Default)]
pub struct Mem0Provider;

#[derive(Default)]
pub struct OpenMemoryProvider;

#[derive(Default)]
pub struct LettaProvider;

#[derive(Default)]
pub struct ChromaProvider;

#[derive(Default)]
pub struct QdrantProvider;
