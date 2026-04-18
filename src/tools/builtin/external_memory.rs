use std::sync::Arc;

use async_trait::async_trait;

use crate::agent::learning::{LearningOrchestrator, ProviderHealthStatus, ProviderReadiness};
use crate::context::JobContext;
use crate::settings::ActiveLearningProvider;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

fn active_provider_status(
    active: ActiveLearningProvider,
    statuses: &[ProviderHealthStatus],
) -> Result<&ProviderHealthStatus, ToolError> {
    let active_name = match active {
        ActiveLearningProvider::None => {
            return Err(ToolError::ExecutionFailed(
                "No external memory provider is active. Enable learning.providers.active to use this tool."
                    .to_string(),
            ));
        }
        _ => active.as_str(),
    };

    statuses
        .iter()
        .find(|status| status.provider == active_name)
        .ok_or_else(|| {
            ToolError::ExecutionFailed(format!(
                "Active external memory provider '{}' is not registered.",
                active_name
            ))
        })
}

fn ensure_active_provider_healthy(status: &ProviderHealthStatus) -> Result<(), ToolError> {
    if status.readiness == ProviderReadiness::Ready {
        return Ok(());
    }

    let reason = status
        .error
        .clone()
        .unwrap_or_else(|| format!("provider state is {}", status.readiness.as_str()));
    Err(ToolError::ExecutionFailed(format!(
        "Active external memory provider '{}' is unavailable: {}",
        status.provider, reason
    )))
}

pub struct ExternalMemoryRecallTool {
    orchestrator: Arc<LearningOrchestrator>,
}

impl ExternalMemoryRecallTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait]
impl Tool for ExternalMemoryRecallTool {
    fn name(&self) -> &str {
        "external_memory_recall"
    }

    fn description(&self) -> &str {
        "Recall relevant context from the active external memory provider. Use this \
         when the task may depend on long-term memory stored outside the local workspace."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language recall query to send to the active external memory provider."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 10,
                    "description": "Maximum number of recall hits to return."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let query = require_str(&params, "query")?;
        let limit = params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(5)
            .clamp(1, 10) as usize;

        let settings = self.orchestrator.load_settings_for_user(&ctx.user_id).await;
        let statuses = self.orchestrator.provider_health(&ctx.user_id).await;
        let active_status = active_provider_status(settings.providers.active, &statuses)?;
        ensure_active_provider_healthy(active_status)?;

        let hits = self
            .orchestrator
            .provider_recall(&ctx.user_id, query, limit)
            .await;

        Ok(ToolOutput::success(
            serde_json::json!({
                "provider": active_status.provider,
                "query": query,
                "limit": limit,
                "count": hits.len(),
                "hits": hits,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ExternalMemoryStatusTool {
    orchestrator: Arc<LearningOrchestrator>,
}

impl ExternalMemoryStatusTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>) -> Self {
        Self { orchestrator }
    }
}

#[async_trait]
impl Tool for ExternalMemoryStatusTool {
    fn name(&self) -> &str {
        "external_memory_status"
    }

    fn description(&self) -> &str {
        "Inspect which external memory provider is active and whether it is healthy. \
         Use this when recall fails or you need to understand external-memory availability."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let settings = self.orchestrator.load_settings_for_user(&ctx.user_id).await;
        let statuses = self.orchestrator.provider_health(&ctx.user_id).await;
        let active_provider = settings.providers.active.as_str();
        let active_status = statuses
            .iter()
            .find(|status| status.provider == active_provider);
        let tool_extensions = self
            .orchestrator
            .provider_tool_extensions(&ctx.user_id)
            .await;

        Ok(ToolOutput::success(
            serde_json::json!({
                "active_provider": active_provider,
                "active": active_provider != "none",
                "active_status": active_status,
                "providers": statuses,
                "tool_extensions": tool_extensions,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
