use scrappy_mcp_tools::events::{StatusReporter, ToolEvent};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::Manager;
use tracing::info;

// ---------------------------------------------------------------------------
// TauriEventReporter — forwards ToolEvents to the frontend via Tauri events
// ---------------------------------------------------------------------------

/// The payload emitted on the `"tool_event"` Tauri channel.
/// Frontend can listen with `listen("tool_event", handler)`.
#[derive(Debug, Clone, Serialize)]
pub struct ToolEventPayload {
    /// One of: "status", "tool_activity", "progress"
    pub kind: String,
    /// Human-readable message
    pub message: String,
    /// Tool name (for tool_activity events)
    pub tool_name: Option<String>,
    /// Progress percentage 0–100 (for progress events)
    pub percentage: Option<f32>,
    /// Tool dispatch status: "running", "complete", "failed"
    pub status: Option<String>,
}

/// A reporter that bridges `ToolEvent`s to the Tauri event system so the
/// frontend can display live tool-call progress when tools are invoked via
/// the OpenClaw gateway / MCP IPC path.
pub struct TauriEventReporter {
    app: tauri::AppHandle,
}

impl TauriEventReporter {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }
}

#[async_trait::async_trait]
impl StatusReporter for TauriEventReporter {
    async fn report(&self, event: ToolEvent) {
        use tauri::Emitter;

        let payload = match &event {
            ToolEvent::ToolActivity {
                tool_name,
                input_summary,
                status,
            } => ToolEventPayload {
                kind: "tool_activity".into(),
                message: format!("{}: {}", tool_name, input_summary),
                tool_name: Some(tool_name.clone()),
                percentage: None,
                status: Some(status.clone()),
            },
            ToolEvent::Status { msg, .. } => ToolEventPayload {
                kind: "status".into(),
                message: msg.clone(),
                tool_name: None,
                percentage: None,
                status: None,
            },
            ToolEvent::Progress {
                percentage,
                message,
            } => ToolEventPayload {
                kind: "progress".into(),
                message: message.clone(),
                tool_name: None,
                percentage: Some(*percentage),
                status: None,
            },
        };

        let _ = self.app.emit("tool_event", &payload);
    }
}

/// Handles incoming RPC requests from OpenClawEngine (OpenClaw)
pub struct McpRequestHandler {
    app: tauri::AppHandle,
}

impl McpRequestHandler {
    pub fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    /// Handle an incoming RPC request
    pub async fn handle_request(&self, method: &str, params: Value) -> Result<Value, String> {
        info!("[ipc] Handling request: {} ({})", method, params);

        match method {
            "mcp.list_tools" => self.list_tools().await,
            "mcp.search_tools" => self.search_tools(params).await,
            "mcp.call_tool" => self.call_tool(params).await,
            "mcp.list_skills" => self.list_skills().await,
            "mcp.run_skill" => self.run_skill(params).await,
            "mcp.save_skill" => self.save_skill(params).await,
            _ => Err(format!("Unknown method: {}", method)),
        }
    }

    fn get_mcp_config(&self) -> crate::rig_lib::sandbox_factory::McpOrchestratorConfig {
        use crate::rig_lib::sandbox_factory::McpOrchestratorConfig;
        use tauri::Manager;

        // Load mcp_base_url / mcp_auth_token from the live ConfigManager so that
        // values saved in Settings > Gateway tab are honoured here too.
        let user_config = self
            .app
            .try_state::<crate::config::ConfigManager>()
            .map(|cm| cm.get_config());

        McpOrchestratorConfig {
            mcp_base_url: user_config.as_ref().and_then(|c| c.mcp_base_url.clone()),
            mcp_auth_token: user_config.as_ref().and_then(|c| c.mcp_auth_token.clone()),
            sandbox_enabled: true,
            user_skills_path: Some(
                self.app
                    .path()
                    .app_config_dir()
                    .unwrap_or_default()
                    .join("skills"),
            ),
            builtin_skills_path: Some(
                self.app
                    .path()
                    .resource_dir()
                    .unwrap_or_default()
                    .join("scrappy-mcp-tools/skills/built_in"),
            ),
        }
    }

    async fn list_tools(&self) -> Result<Value, String> {
        self.search_tools(serde_json::json!({ "query": "" })).await
    }
    async fn search_tools(&self, params: Value) -> Result<Value, String> {
        use crate::rig_lib::tool_discovery;
        use scrappy_mcp_tools::skills::manager::SkillManager;

        let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let config = self.get_mcp_config();

        // Setup managers
        let skill_manager = config
            .user_skills_path
            .as_ref()
            .map(|up| SkillManager::new(up.clone(), config.builtin_skills_path.clone()));

        let mcp_client = config.mcp_base_url.as_ref().and_then(|url| {
            scrappy_mcp_tools::McpClient::new(scrappy_mcp_tools::McpConfig {
                base_url: url.clone(),
                auth_token: config.mcp_auth_token.clone().unwrap_or_default(),
                timeout_ms: 10000,
            })
            .ok()
        });

        let result = tool_discovery::search_all_tools(
            query,
            mcp_client.as_ref(),
            skill_manager.as_ref(),
            true, // include host tools
        )
        .await;

        Ok(serde_json::to_value(result).map_err(|e| e.to_string())?)
    }

    async fn list_skills(&self) -> Result<Value, String> {
        use scrappy_mcp_tools::skills::manager::SkillManager;

        // In real app, resolved paths from AppHandle
        let app_dir = self.app.path().app_config_dir().unwrap_or_default();
        let user_skills_dir = app_dir.join("skills");
        let builtin_skills_dir = self
            .app
            .path()
            .resource_dir()
            .unwrap_or_default()
            .join("scrappy-mcp-tools/skills/built_in");

        let manager = SkillManager::new(user_skills_dir, Some(builtin_skills_dir));
        let skills = manager.list_skills().map_err(|e| e.to_string())?;

        // Map to simple JSON list
        let list: Vec<Value> = skills
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "name": s.manifest.name,
                    "description": s.manifest.description,
                    "parameters": s.manifest.parameters
                })
            })
            .collect();

        Ok(serde_json::json!({ "skills": list }))
    }

    async fn run_skill(&self, params: Value) -> Result<Value, String> {
        use crate::rig_lib::unified_provider::ProviderKind;
        use crate::rig_lib::{sandbox_factory, RigManager};
        use scrappy_mcp_tools::skills::manager::SkillManager;
        use std::collections::HashMap;

        let skill_id = params
            .get("skill_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing skill_id")?;
        let args = params
            .get("args")
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();

        // 1. Prepare Skill Script
        let config = self.get_mcp_config();
        let manager = SkillManager::new(
            config.user_skills_path.unwrap_or_default(),
            config.builtin_skills_path,
        );

        // Convert args map
        let mut skill_args = HashMap::new();
        for (k, v) in args {
            skill_args.insert(k, v);
        }

        let script = manager
            .prepare_script(skill_id, &skill_args)
            .map_err(|e| e.to_string())?;

        // 2. Setup Sandbox (Transient)
        let base_url = "http://127.0.0.1:0/v1".to_string();
        let provider_kind = ProviderKind::Local;
        let rig = Arc::new(RigManager::new(
            provider_kind,
            base_url,
            "default".into(),
            Some(self.app.clone()),
            None,
            8192,
            None,
            true,
            None,
            None,
            None,
        ));

        let config = sandbox_factory::McpOrchestratorConfig {
            mcp_base_url: None,
            mcp_auth_token: None,
            sandbox_enabled: true,
            user_skills_path: Some(
                self.app
                    .path()
                    .app_config_dir()
                    .unwrap_or_default()
                    .join("skills"),
            ),
            builtin_skills_path: Some(
                self.app
                    .path()
                    .resource_dir()
                    .unwrap_or_default()
                    .join("scrappy-mcp-tools/skills/built_in"),
            ),
        };

        let reporter = Arc::new(TauriEventReporter::new(self.app.clone()));
        let sandbox = sandbox_factory::create_sandbox(rig.clone(), &config, reporter)
            .ok_or("Failed to create sandbox")?;

        info!("[ipc] Running Skill: {}", skill_id);

        // 3. Execute
        let result = sandbox
            .execute(&script)
            .map_err(|e| format!("Skill execution failed: {:?}", e))?;

        Ok(serde_json::json!({
            "content": [
                { "type": "text", "text": result.output }
            ],
            "isError": false
        }))
    }

    async fn save_skill(&self, params: Value) -> Result<Value, String> {
        use scrappy_mcp_tools::skills::manager::SkillManager;
        use scrappy_mcp_tools::skills::manifest::{SkillManifest, SkillParameter};

        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Missing id")?;
        let script = params
            .get("script")
            .and_then(|v| v.as_str())
            .ok_or("Missing script")?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let parameters: Vec<SkillParameter> = params
            .get("parameters")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let config = self.get_mcp_config();

        if let Some(up) = config.user_skills_path {
            let manifest = SkillManifest {
                name: id.to_string(),
                description,
                version: "1.0.0".to_string(),
                author: Some("Openclaw".to_string()),
                tools_used: vec![],
                parameters,
                script_file: format!("{}.rhai", id),
            };

            let manager = SkillManager::new(up, config.builtin_skills_path);
            manager
                .save_skill(id, manifest, script)
                .map_err(|e| e.to_string())?;

            Ok(serde_json::json!({
                "success": true,
                "id": id
            }))
        } else {
            Err("User skills path not configured".to_string())
        }
    }

    async fn call_tool(&self, params: Value) -> Result<Value, String> {
        use crate::rig_lib::tool_router::{summarize_result, ToolRouter};
        use crate::rig_lib::unified_provider::ProviderKind;
        use crate::rig_lib::{sandbox_factory, RigManager};
        use scrappy_mcp_tools::skills::manager::SkillManager;
        use std::sync::Arc;

        let tool_name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'name' in params")?;

        let args = params
            .get("arguments")
            .or_else(|| params.get("args"))
            .cloned()
            .unwrap_or(serde_json::json!({}));

        info!(
            "[ipc] ToolRouter dispatching: {} with args: {}",
            tool_name, args
        );

        // 1. Setup managers and config
        let config = self.get_mcp_config();

        // 2. Setup transient RigManager (needed for host tools)
        let base_url = "http://127.0.0.1:0/v1".to_string();
        let rig = Arc::new(RigManager::new(
            ProviderKind::Local,
            base_url,
            "default".into(),
            Some(self.app.clone()),
            None,
            8192,
            None,
            true,
            None,
            None,
            None,
        ));

        // 3. Setup Sandbox — now with TauriEventReporter instead of SilentReporter
        let reporter = Arc::new(TauriEventReporter::new(self.app.clone()));
        let sandbox = sandbox_factory::create_sandbox(rig.clone(), &config, reporter)
            .ok_or("Failed to create sandbox")?;

        // 4. Setup Managers
        let skill_manager = SkillManager::new(
            config.user_skills_path.unwrap_or_default(),
            config.builtin_skills_path,
        );

        let mcp_client = config.mcp_base_url.as_ref().and_then(|url| {
            scrappy_mcp_tools::McpClient::new(scrappy_mcp_tools::McpConfig {
                base_url: url.clone(),
                auth_token: config.mcp_auth_token.clone().unwrap_or_default(),
                timeout_ms: 10000,
            })
            .ok()
        });

        // 5. Use Router
        let router = ToolRouter {
            mcp_client: mcp_client.as_ref(),
            skill_manager: Some(&skill_manager),
            sandbox: Some(&sandbox),
        };

        let result = router.call(tool_name, args).await?;

        // 6. Summarize/Truncate output (Auto-summarization Middleware)
        // Limit is configurable via UserConfig.mcp_tool_result_max_chars (default: 5000).
        let max_chars = self
            .app
            .try_state::<crate::config::ConfigManager>()
            .map(|cm| cm.get_config().mcp_tool_result_max_chars as usize)
            .unwrap_or(5000);
        Ok(summarize_result(result, max_chars))
    }
}
