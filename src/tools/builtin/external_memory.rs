//! Compatibility adapter for extracted external-memory tools.

use async_trait::async_trait;

use crate::agent::learning::{LearningOrchestrator, ProviderHealthStatus};
use crate::settings::LearningProviderSettings;

pub use thinclaw_tools::builtin::external_memory::{
    ExternalMemoryExportTool, ExternalMemoryOffTool, ExternalMemoryPort,
    ExternalMemoryProviderConfig, ExternalMemoryProviderStatus, ExternalMemoryRecallTool,
    ExternalMemorySetupTool, ExternalMemoryStatusTool,
};

fn provider_status_to_tool_status(status: ProviderHealthStatus) -> ExternalMemoryProviderStatus {
    ExternalMemoryProviderStatus {
        provider: status.provider,
        active: status.active,
        enabled: status.enabled,
        healthy: status.healthy,
        readiness: status.readiness.as_str().to_string(),
        latency_ms: status.latency_ms,
        error: status.error,
        capabilities: status.capabilities,
        metadata: status.metadata,
    }
}

fn provider_config_to_settings(value: ExternalMemoryProviderConfig) -> LearningProviderSettings {
    LearningProviderSettings {
        enabled: value.enabled,
        config: value.config,
        cadence: value.cadence,
        depth: value.depth,
        user_modeling_enabled: value.user_modeling_enabled,
    }
}

#[async_trait]
impl ExternalMemoryPort for LearningOrchestrator {
    async fn active_provider_name(&self, user_id: &str) -> Option<String> {
        self.load_settings_for_user(user_id)
            .await
            .providers
            .active_provider_name()
    }

    async fn provider_health(&self, user_id: &str) -> Vec<ExternalMemoryProviderStatus> {
        LearningOrchestrator::provider_health(self, user_id)
            .await
            .into_iter()
            .map(provider_status_to_tool_status)
            .collect()
    }

    async fn provider_tool_extensions(&self, user_id: &str) -> Vec<String> {
        LearningOrchestrator::provider_tool_extensions(self, user_id).await
    }

    async fn provider_recall(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        LearningOrchestrator::provider_recall(self, user_id, query, limit)
            .await
            .into_iter()
            .filter_map(|hit| serde_json::to_value(hit).ok())
            .collect()
    }

    async fn configure_memory_provider(
        &self,
        user_id: &str,
        provider: &str,
        settings: ExternalMemoryProviderConfig,
        activate: bool,
    ) -> Result<Vec<ExternalMemoryProviderStatus>, String> {
        LearningOrchestrator::configure_memory_provider(
            self,
            user_id,
            provider,
            provider_config_to_settings(settings),
            activate,
        )
        .await
        .map(|statuses| {
            statuses
                .into_iter()
                .map(provider_status_to_tool_status)
                .collect()
        })
    }

    async fn export_provider_payload(
        &self,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<String, String> {
        LearningOrchestrator::export_provider_payload(self, user_id, payload).await
    }

    async fn disable_active_memory_provider(
        &self,
        user_id: &str,
    ) -> Result<Vec<ExternalMemoryProviderStatus>, String> {
        LearningOrchestrator::disable_active_memory_provider(self, user_id)
            .await
            .map(|statuses| {
                statuses
                    .into_iter()
                    .map(provider_status_to_tool_status)
                    .collect()
            })
    }
}
