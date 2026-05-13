use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use thinclaw_tools_core::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};
use thinclaw_types::JobContext;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalMemoryProviderConfig {
    pub enabled: bool,
    pub config: std::collections::HashMap<String, String>,
    pub cadence: Option<u32>,
    pub depth: Option<u32>,
    pub user_modeling_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalMemoryProviderStatus {
    pub provider: String,
    #[serde(default)]
    pub active: bool,
    pub enabled: bool,
    pub healthy: bool,
    pub readiness: String,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait ExternalMemoryPort: Send + Sync {
    async fn active_provider_name(&self, user_id: &str) -> Option<String>;
    async fn provider_health(&self, user_id: &str) -> Vec<ExternalMemoryProviderStatus>;
    async fn provider_tool_extensions(&self, user_id: &str) -> Vec<String>;
    async fn provider_recall(
        &self,
        user_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<serde_json::Value>;
    async fn configure_memory_provider(
        &self,
        user_id: &str,
        provider: &str,
        settings: ExternalMemoryProviderConfig,
        activate: bool,
    ) -> Result<Vec<ExternalMemoryProviderStatus>, String>;
    async fn export_provider_payload(
        &self,
        user_id: &str,
        payload: &serde_json::Value,
    ) -> Result<String, String>;
    async fn disable_active_memory_provider(
        &self,
        user_id: &str,
    ) -> Result<Vec<ExternalMemoryProviderStatus>, String>;
}

fn config_value_to_string(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn active_provider_status<'a>(
    active_provider: Option<&str>,
    statuses: &'a [ExternalMemoryProviderStatus],
) -> Result<&'a ExternalMemoryProviderStatus, ToolError> {
    let active_name = active_provider.filter(|value| !value.trim().is_empty()).ok_or_else(|| {
        ToolError::ExecutionFailed(
            "No external memory provider is active. Enable learning.providers.active to use this tool."
                .to_string(),
        )
    })?;

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

fn ensure_active_provider_healthy(status: &ExternalMemoryProviderStatus) -> Result<(), ToolError> {
    if status.readiness == "ready" {
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
    port: Arc<dyn ExternalMemoryPort>,
}

impl ExternalMemoryRecallTool {
    pub fn new(port: Arc<dyn ExternalMemoryPort>) -> Self {
        Self { port }
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

        let active_provider = self.port.active_provider_name(&ctx.user_id).await;
        let statuses = self.port.provider_health(&ctx.user_id).await;
        let active_status = active_provider_status(active_provider.as_deref(), &statuses)?;
        ensure_active_provider_healthy(active_status)?;

        let hits = self.port.provider_recall(&ctx.user_id, query, limit).await;

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
    port: Arc<dyn ExternalMemoryPort>,
}

impl ExternalMemoryStatusTool {
    pub fn new(port: Arc<dyn ExternalMemoryPort>) -> Self {
        Self { port }
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
        let statuses = self.port.provider_health(&ctx.user_id).await;
        let active_provider = self
            .port
            .active_provider_name(&ctx.user_id)
            .await
            .unwrap_or_else(|| "none".to_string());
        let active_status = statuses
            .iter()
            .find(|status| status.provider == active_provider);
        let tool_extensions = self.port.provider_tool_extensions(&ctx.user_id).await;

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

pub struct ExternalMemorySetupTool {
    port: Arc<dyn ExternalMemoryPort>,
}

impl ExternalMemorySetupTool {
    pub fn new(port: Arc<dyn ExternalMemoryPort>) -> Self {
        Self { port }
    }
}

#[async_trait]
impl Tool for ExternalMemorySetupTool {
    fn name(&self) -> &str {
        "external_memory_setup"
    }

    fn description(&self) -> &str {
        "Configure and optionally activate an external memory provider such as Honcho, Zep, \
         Mem0, OpenMemory, Letta, Chroma, Qdrant, or custom_http."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "provider": {
                    "type": "string",
                    "description": "Provider slug: honcho, zep, mem0, openmemory, letta, chroma, qdrant, or custom_http."
                },
                "base_url": {
                    "type": "string",
                    "description": "Base URL for the provider API."
                },
                "api_key": {
                    "type": "string",
                    "description": "Optional inline API key."
                },
                "api_key_env": {
                    "type": "string",
                    "description": "Optional environment variable name holding the API key."
                },
                "embedding_url": {
                    "type": "string",
                    "description": "Embedding endpoint for vector stores such as Chroma or Qdrant."
                },
                "collection": {
                    "type": "string",
                    "description": "Qdrant collection name."
                },
                "collection_id": {
                    "type": "string",
                    "description": "Chroma collection UUID."
                },
                "agent_id": {
                    "type": "string",
                    "description": "Provider-side agent identifier for Mem0 or Letta."
                },
                "user_id": {
                    "type": "string",
                    "description": "Optional provider-side user identifier; defaults to the ThinClaw principal."
                },
                "enabled": {
                    "type": "boolean",
                    "default": true
                },
                "activate": {
                    "type": "boolean",
                    "default": true
                },
                "cadence": {
                    "type": "integer",
                    "minimum": 1
                },
                "depth": {
                    "type": "integer",
                    "minimum": 1
                },
                "user_modeling_enabled": {
                    "type": "boolean",
                    "default": false
                },
                "config": {
                    "type": "object",
                    "description": "Additional provider-specific config key/value pairs."
                }
            },
            "required": ["provider"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let provider = require_str(&params, "provider")?
            .trim()
            .to_ascii_lowercase();
        let enabled = params
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let activate = params
            .get("activate")
            .and_then(|value| value.as_bool())
            .unwrap_or(true);
        let mut config = params
            .get("config")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(key, value)| config_value_to_string(value).map(|value| (key, value)))
            .collect::<std::collections::HashMap<_, _>>();
        if let Some(base_url) = params.get("base_url").and_then(|value| value.as_str()) {
            config.insert("base_url".to_string(), base_url.to_string());
        }
        if let Some(api_key) = params.get("api_key").and_then(|value| value.as_str()) {
            config.insert("api_key".to_string(), api_key.to_string());
        }
        if let Some(api_key_env) = params.get("api_key_env").and_then(|value| value.as_str()) {
            config.insert("api_key_env".to_string(), api_key_env.to_string());
        }
        for key in [
            "embedding_url",
            "collection",
            "collection_id",
            "agent_id",
            "user_id",
        ] {
            if let Some(value) = params.get(key).and_then(|value| value.as_str()) {
                config.insert(key.to_string(), value.to_string());
            }
        }
        let provider_settings = ExternalMemoryProviderConfig {
            enabled,
            config,
            cadence: params
                .get("cadence")
                .and_then(|value| value.as_u64())
                .map(|value| value as u32),
            depth: params
                .get("depth")
                .and_then(|value| value.as_u64())
                .map(|value| value as u32),
            user_modeling_enabled: params
                .get("user_modeling_enabled")
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
        };
        let statuses = self
            .port
            .configure_memory_provider(&ctx.user_id, &provider, provider_settings, activate)
            .await
            .map_err(ToolError::ExecutionFailed)?;
        let active_status = statuses.iter().find(|status| status.provider == provider);

        Ok(ToolOutput::success(
            serde_json::json!({
                "provider": provider,
                "active": activate,
                "active_status": active_status,
                "providers": statuses,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ExternalMemoryExportTool {
    port: Arc<dyn ExternalMemoryPort>,
}

impl ExternalMemoryExportTool {
    pub fn new(port: Arc<dyn ExternalMemoryPort>) -> Self {
        Self { port }
    }
}

#[async_trait]
impl Tool for ExternalMemoryExportTool {
    fn name(&self) -> &str {
        "external_memory_export"
    }

    fn description(&self) -> &str {
        "Export an explicit memory payload to the active external memory provider. Use this \
         for durable facts, user preferences, or important session summaries that should be \
         mirrored outside local workspace memory."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Memory content to export."
                },
                "payload": {
                    "type": "object",
                    "description": "Structured memory payload. Used when content is omitted."
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let payload = params
            .get("payload")
            .cloned()
            .or_else(|| {
                params
                    .get("content")
                    .and_then(|value| value.as_str())
                    .map(|content| serde_json::json!({"content": content}))
            })
            .ok_or_else(|| {
                ToolError::InvalidParameters(
                    "external_memory_export requires content or payload".to_string(),
                )
            })?;

        let active_provider = self.port.active_provider_name(&ctx.user_id).await;
        let statuses = self.port.provider_health(&ctx.user_id).await;
        let active_status = active_provider_status(active_provider.as_deref(), &statuses)?;
        ensure_active_provider_healthy(active_status)?;

        let provider = self
            .port
            .export_provider_payload(&ctx.user_id, &payload)
            .await
            .map_err(ToolError::ExecutionFailed)?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "provider": provider,
                "status": "exported",
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

pub struct ExternalMemoryOffTool {
    port: Arc<dyn ExternalMemoryPort>,
}

impl ExternalMemoryOffTool {
    pub fn new(port: Arc<dyn ExternalMemoryPort>) -> Self {
        Self { port }
    }
}

#[async_trait]
impl Tool for ExternalMemoryOffTool {
    fn name(&self) -> &str {
        "external_memory_off"
    }

    fn description(&self) -> &str {
        "Disable the active external memory provider and shut it down cleanly."
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
        let statuses = self
            .port
            .disable_active_memory_provider(&ctx.user_id)
            .await
            .map_err(ToolError::ExecutionFailed)?;

        Ok(ToolOutput::success(
            serde_json::json!({
                "active": false,
                "providers": statuses,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
