use scrappy_mcp_tools::events::{StatusReporter, ToolEvent};
use serde_json::Value;
use std::sync::Arc;
use tauri::Manager;
use tracing::info;

// A dummy reporter that drops events, since OpenClawEngine doesn't consume the XML stream yet.
// In the future, we can forward these events to OpenClawEngine via IPC if needed.
struct SilentReporter;
#[async_trait::async_trait]
impl StatusReporter for SilentReporter {
    async fn report(&self, _event: ToolEvent) {
        // Drop it
    }
}

/// Handles incoming RPC requests from OpenClawEngine (OpenClaw)
pub struct McpRequestHandler {
    app: tauri::AppHandle,
    // We need to construct a RigManager on demand or have one handy.
    // Ideally, we'd grab the main RigManager from AppState if it was stored there.
    // For now, we will construct a transient one using default config for each request
    // or store a shared one. To save overhead, let's assume valid defaults.
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
        McpOrchestratorConfig {
            mcp_base_url: None, // TODO: Load from AppState/Config
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

        let reporter = Arc::new(SilentReporter);
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
        use std::sync::Arc; // Added this import

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

        // 3. Setup Sandbox
        let sandbox =
            sandbox_factory::create_sandbox(rig.clone(), &config, Arc::new(SilentReporter))
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
        Ok(summarize_result(result, 5000))
    }
}
