use std::sync::Arc;

use async_trait::async_trait;

use crate::agent::learning::{LearningOrchestrator, ProviderHealthStatus, ProviderReadiness};
use crate::context::JobContext;
use crate::settings::LearningProviderSettings;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

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
    statuses: &'a [ProviderHealthStatus],
) -> Result<&'a ProviderHealthStatus, ToolError> {
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
        let active_status = active_provider_status(
            settings.providers.active_provider_name().as_deref(),
            &statuses,
        )?;
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
        let active_provider = settings
            .providers
            .active_provider_name()
            .unwrap_or_else(|| settings.providers.active.as_str().to_string());
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

pub struct ExternalMemorySetupTool {
    orchestrator: Arc<LearningOrchestrator>,
}

impl ExternalMemorySetupTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>) -> Self {
        Self { orchestrator }
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
        let provider_settings = LearningProviderSettings {
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
            .orchestrator
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
    orchestrator: Arc<LearningOrchestrator>,
}

impl ExternalMemoryExportTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>) -> Self {
        Self { orchestrator }
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

        let settings = self.orchestrator.load_settings_for_user(&ctx.user_id).await;
        let statuses = self.orchestrator.provider_health(&ctx.user_id).await;
        let active_status = active_provider_status(
            settings.providers.active_provider_name().as_deref(),
            &statuses,
        )?;
        ensure_active_provider_healthy(active_status)?;

        let provider = self
            .orchestrator
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
    orchestrator: Arc<LearningOrchestrator>,
}

impl ExternalMemoryOffTool {
    pub fn new(orchestrator: Arc<LearningOrchestrator>) -> Self {
        Self { orchestrator }
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
            .orchestrator
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

#[cfg(all(test, feature = "libsql"))]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockExternalMemoryState {
        paths: Arc<Mutex<Vec<String>>>,
    }

    async fn mock_external_memory_handler(
        axum::extract::State(state): axum::extract::State<MockExternalMemoryState>,
        uri: axum::http::Uri,
    ) -> axum::response::Response {
        let path = uri.path().to_string();
        state
            .paths
            .lock()
            .expect("mock external memory paths")
            .push(path.clone());
        if path == "/" || path == "/search" || path == "/memories" {
            (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({
                    "results": [{
                        "id": "external-1",
                        "content": "external memory test recall"
                    }]
                })),
            )
                .into_response()
        } else {
            (
                axum::http::StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": "not found"})),
            )
                .into_response()
        }
    }

    async fn spawn_external_memory_server() -> (String, Arc<Mutex<Vec<String>>>) {
        let paths = Arc::new(Mutex::new(Vec::new()));
        let state = MockExternalMemoryState {
            paths: Arc::clone(&paths),
        };
        let app = axum::Router::new()
            .fallback(axum::routing::any(mock_external_memory_handler))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind external memory mock");
        let addr = listener.local_addr().expect("external memory mock addr");
        tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("external memory mock server");
        });
        (format!("http://{addr}"), paths)
    }

    #[tokio::test]
    async fn external_memory_setup_status_recall_export_and_off_use_active_provider() {
        let (store, _guard) = crate::testing::test_db().await;
        let (base_url, paths) = spawn_external_memory_server().await;
        let orchestrator = Arc::new(LearningOrchestrator::new(Arc::clone(&store), None, None));
        let mut ctx = JobContext::default();
        ctx.user_id = "external-memory-tool-user".to_string();

        let setup = ExternalMemorySetupTool::new(Arc::clone(&orchestrator));
        let status = ExternalMemoryStatusTool::new(Arc::clone(&orchestrator));
        let recall = ExternalMemoryRecallTool::new(Arc::clone(&orchestrator));
        let export = ExternalMemoryExportTool::new(Arc::clone(&orchestrator));
        let off = ExternalMemoryOffTool::new(Arc::clone(&orchestrator));

        let setup_output = setup
            .execute(
                serde_json::json!({
                    "provider": "openmemory",
                    "base_url": base_url,
                    "api_key": "tool-secret",
                    "enabled": true,
                    "activate": true
                }),
                &ctx,
            )
            .await
            .expect("setup openmemory");
        assert_eq!(setup_output.result["provider"], "openmemory");
        assert_eq!(
            setup_output.result["active_status"]["readiness"],
            serde_json::json!("ready")
        );

        let status_output = status
            .execute(serde_json::json!({}), &ctx)
            .await
            .expect("status openmemory");
        assert_eq!(status_output.result["active_provider"], "openmemory");
        assert_eq!(status_output.result["active_status"]["readiness"], "ready");

        let recall_output = recall
            .execute(
                serde_json::json!({"query": "what do you remember?", "limit": 3}),
                &ctx,
            )
            .await
            .expect("recall openmemory");
        assert_eq!(recall_output.result["provider"], "openmemory");
        assert_eq!(recall_output.result["count"], 1);

        let export_output = export
            .execute(serde_json::json!({"content": "remember this"}), &ctx)
            .await
            .expect("export openmemory");
        assert_eq!(export_output.result["provider"], "openmemory");
        assert_eq!(export_output.result["status"], "exported");

        let off_output = off
            .execute(serde_json::json!({}), &ctx)
            .await
            .expect("disable active provider");
        assert_eq!(off_output.result["active"], false);

        let status_after_off = status
            .execute(serde_json::json!({}), &ctx)
            .await
            .expect("status after off");
        assert_eq!(status_after_off.result["active"], false);
        assert_eq!(
            status_after_off.result["tool_extensions"],
            serde_json::json!([])
        );

        let paths = paths.lock().expect("external memory mock paths");
        assert!(paths.iter().any(|path| path == "/"));
        assert!(paths.iter().any(|path| path == "/search"));
        assert!(paths.iter().any(|path| path == "/memories"));
    }

    #[tokio::test]
    async fn external_memory_recall_and_export_fail_closed_when_provider_unhealthy() {
        let (store, _guard) = crate::testing::test_db().await;
        let orchestrator = Arc::new(LearningOrchestrator::new(Arc::clone(&store), None, None));
        let mut ctx = JobContext::default();
        ctx.user_id = "external-memory-unhealthy-user".to_string();

        let setup = ExternalMemorySetupTool::new(Arc::clone(&orchestrator));
        setup
            .execute(
                serde_json::json!({
                    "provider": "custom_http",
                    "base_url": "http://127.0.0.1:1",
                    "enabled": true,
                    "activate": true
                }),
                &ctx,
            )
            .await
            .expect("setup unavailable custom_http");

        let recall = ExternalMemoryRecallTool::new(Arc::clone(&orchestrator));
        let err = recall
            .execute(serde_json::json!({"query": "unavailable"}), &ctx)
            .await
            .expect_err("unhealthy recall should fail before recall call");
        assert!(err.to_string().contains("unavailable"));

        let export = ExternalMemoryExportTool::new(orchestrator);
        let err = export
            .execute(serde_json::json!({"content": "unavailable"}), &ctx)
            .await
            .expect_err("unhealthy export should fail before export call");
        assert!(err.to_string().contains("unavailable"));
    }
}
