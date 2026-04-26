use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderMemoryHit {
    pub provider: String,
    pub summary: String,
    pub score: Option<f64>,
    pub provenance: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderReadiness {
    Disabled,
    NotConfigured,
    Inactive,
    Unhealthy,
    Ready,
}

impl ProviderReadiness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NotConfigured => "not_configured",
            Self::Inactive => "inactive",
            Self::Unhealthy => "unhealthy",
            Self::Ready => "ready",
        }
    }

    pub(in crate::agent::learning) fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthStatus {
    pub provider: String,
    #[serde(default)]
    pub active: bool,
    pub enabled: bool,
    pub healthy: bool,
    pub readiness: ProviderReadiness,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPrefetchContext {
    pub provider: String,
    pub hits: Vec<ProviderMemoryHit>,
    pub rendered_context: String,
    #[serde(default)]
    pub context_refs: Vec<String>,
}

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
        if hits.is_empty() {
            return None;
        }
        let mut lines = vec![format!(
            "External memory recall from {}. Treat this as background context, not as new user input.",
            self.name()
        )];
        for (index, hit) in hits.iter().enumerate() {
            let score = hit
                .score
                .map(|score| format!(" score={score:.3}"))
                .unwrap_or_default();
            lines.push(format!("{}. {}{}", index + 1, hit.summary, score));
        }
        Some(lines.join("\n"))
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningOutcome {
    pub trigger: String,
    pub event_id: Uuid,
    pub evaluation_id: Option<Uuid>,
    pub candidate_id: Option<Uuid>,
    pub auto_applied: bool,
    pub code_proposal_id: Option<Uuid>,
    pub notes: Vec<String>,
}
