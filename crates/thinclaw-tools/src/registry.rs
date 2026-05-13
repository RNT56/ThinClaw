//! Root-independent tool registry storage and filtering.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "document-extraction")]
use crate::builtin::ExtractDocumentTool;
use crate::builtin::{
    ADVISOR_TOOL_NAME, AgentManagementPort, AgentThinkTool, AppleMailTool, ApplyPatchTool,
    CanvasTool, ClarifyTool, ConsultAdvisorTool, CreateAgentTool, DesktopAutonomyPort,
    DesktopAutonomyTool, DeviceInfoTool, EchoTool, EmitUserMessageTool, ExtensionManagementPort,
    FileToolHost, GrepTool, HomeAssistantTool, HttpTool, JsonTool, ListAgentsTool, ListDirTool,
    LlmListModelsTool, LlmSelectTool, MessageAgentTool, MoaTool, ProcessTool, ReadFileTool,
    RemoveAgentTool, SearchFilesTool, SendMessageFn, SendMessageTool, SharedModelOverride,
    SharedProcessRegistry, SharedTodoStore, TimeTool, TodoTool, ToolActivateTool, ToolAuthTool,
    ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool, TtsTool, UpdateAgentTool,
    VisionAnalyzeTool, WriteFileTool,
};
use crate::execution::LocalExecutionBackend;
use crate::wasm::SharedCredentialRegistry;
#[cfg(feature = "wasm-runtime")]
use crate::wasm::{
    Capabilities, HostToolInvoker, OAuthRefreshConfig, ResourceLimits, WasmError, WasmStorageError,
    WasmToolRuntime, WasmToolStore, WasmToolWrapper,
};
use thinclaw_llm_core::{LlmProvider, ToolDefinition};
use thinclaw_secrets::SecretsStore;
use thinclaw_tools_core::{
    ApprovalRequirement, RateLimiter, Tool, ToolDescriptor, ToolDomain, ToolExecutionLane,
    ToolProfile,
};
use tokio::sync::RwLock;

/// Names of built-in tools that cannot be shadowed by dynamic registrations.
pub const PROTECTED_TOOL_NAMES: &[&str] = &[
    "echo",
    "device_info",
    "time",
    "json",
    "http",
    "shell",
    "read_file",
    "write_file",
    "list_dir",
    "apply_patch",
    "memory_search",
    "session_search",
    "memory_write",
    "memory_read",
    "memory_tree",
    "memory_delete",
    "create_job",
    "list_jobs",
    "job_status",
    "cancel_job",
    "build_software",
    "tool_search",
    "tool_install",
    "tool_auth",
    "tool_activate",
    "tool_list",
    "tool_remove",
    "routine_create",
    "routine_list",
    "routine_update",
    "routine_delete",
    "routine_history",
    "skill_list",
    "skill_read",
    "skill_inspect",
    "skill_search",
    "skill_check",
    "skill_install",
    "skill_update",
    "skill_audit",
    "skill_snapshot",
    "skill_publish",
    "skill_tap_list",
    "skill_tap_add",
    "skill_tap_remove",
    "skill_tap_refresh",
    "skill_remove",
    "skill_trust_promote",
    "tts",
    "browser",
    "canvas",
    "agent_think",
    "emit_user_message",
    "spawn_subagent",
    "list_subagents",
    "cancel_subagent",
    "apple_mail",
    "llm_select",
    "llm_list_models",
    "create_agent",
    "list_agents",
    "update_agent",
    "remove_agent",
    "message_agent",
    "extract_document",
    "consult_advisor",
    "prompt_manage",
    "skill_manage",
    "learning_status",
    "learning_history",
    "learning_feedback",
    "learning_proposal_review",
    "external_memory_recall",
    "external_memory_export",
    "external_memory_setup",
    "external_memory_off",
    "external_memory_status",
    "process",
    "todo",
    "clarify",
    "vision_analyze",
    "image_generate",
    "comfy_health",
    "comfy_check_deps",
    "comfy_run_workflow",
    "comfy_manage",
    "send_message",
    "nostr_actions",
    "homeassistant",
    "mixture_of_agents",
    "execute_code",
    "search_files",
    "desktop_apps",
    "desktop_ui",
    "desktop_screen",
    "desktop_calendar_native",
    "desktop_numbers_native",
    "desktop_pages_native",
    "autonomy_control",
];

const IMPLICIT_CAPABILITY_TOOLS: &[&str] = &["agent_think", "emit_user_message"];
const HIDDEN_BY_DEFAULT_TOOL_NAMES: &[&str] = &[
    "external_memory_recall",
    "external_memory_export",
    "external_memory_setup",
    "external_memory_off",
    "external_memory_status",
];
const SKILL_ADMIN_TOOLS: &[&str] = &[
    "skill_search",
    "skill_check",
    "skill_install",
    "skill_update",
    "skill_audit",
    "skill_snapshot",
    "skill_publish",
    "skill_tap_list",
    "skill_tap_add",
    "skill_tap_remove",
    "skill_tap_refresh",
    "skill_remove",
    "skill_reload",
    "skill_trust_promote",
    "skill_manage",
];

/// Registry of available tools.
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    builtin_names: RwLock<HashSet<String>>,
    rate_limiter: RateLimiter,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            builtin_names: RwLock::new(HashSet::new()),
            rate_limiter: RateLimiter::new(),
        }
    }

    /// Get the shared rate limiter for checking built-in tool limits.
    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }

    /// Register a tool. Rejects dynamic tools that try to shadow a built-in name.
    pub async fn register(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        if self.builtin_names.read().await.contains(&name)
            || PROTECTED_TOOL_NAMES.contains(&name.as_str())
        {
            return;
        }
        self.tools.write().await.insert(name.clone(), tool);
    }

    /// Register a tool as built-in using async locks.
    pub async fn register_builtin(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools.write().await.insert(name.clone(), tool);
        self.builtin_names.write().await.insert(name.clone());
    }

    /// Register a tool (sync version for startup, marks as built-in).
    pub fn register_sync(&self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        if let Ok(mut tools) = self.tools.try_write() {
            tools.insert(name.clone(), tool);
            if let Ok(mut builtins) = self.builtin_names.try_write() {
                builtins.insert(name.clone());
            }
        }
    }

    /// Register the root-independent default built-ins.
    ///
    /// Host-specific tools such as browser backends, app adapters, and
    /// sandbox/job orchestration are intentionally registered by the root/app
    /// layer after it has concrete runtime dependencies.
    pub fn register_core_builtin_tools(
        &self,
        credential_registry: Option<Arc<SharedCredentialRegistry>>,
        secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) {
        self.register_sync(Arc::new(EchoTool));
        self.register_sync(Arc::new(TimeTool));
        self.register_sync(Arc::new(JsonTool));
        self.register_sync(Arc::new(DeviceInfoTool::new()));
        self.register_sync(Arc::new(CanvasTool));
        self.register_sync(Arc::new(ClarifyTool));
        self.register_sync(Arc::new(AgentThinkTool));
        self.register_sync(Arc::new(EmitUserMessageTool));

        let mut http = HttpTool::new();
        if let (Some(credential_registry), Some(secrets_store)) =
            (credential_registry, secrets_store)
        {
            http = http.with_credentials(credential_registry, secrets_store);
        }
        self.register_sync(Arc::new(http));

        #[cfg(feature = "document-extraction")]
        self.register_sync(Arc::new(ExtractDocumentTool));

        if let Some(home_assistant) = HomeAssistantTool::from_env() {
            self.register_sync(Arc::new(home_assistant));
            tracing::info!("Registered Home Assistant tool (HASS_URL + HASS_TOKEN)");
        }
    }

    /// Register filesystem development tools with an optional base directory.
    pub fn register_filesystem_tools(
        &self,
        base_dir: Option<PathBuf>,
        file_host: Arc<dyn FileToolHost>,
    ) {
        if let Some(base_dir) = base_dir {
            self.register_sync(Arc::new(
                ReadFileTool::new()
                    .with_base_dir(base_dir.clone())
                    .with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(
                WriteFileTool::new()
                    .with_base_dir(base_dir.clone())
                    .with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(ListDirTool::new().with_base_dir(base_dir.clone())));
            self.register_sync(Arc::new(
                ApplyPatchTool::new()
                    .with_base_dir(base_dir.clone())
                    .with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(GrepTool::new().with_base_dir(base_dir)));
        } else {
            self.register_sync(Arc::new(
                ReadFileTool::new().with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(
                WriteFileTool::new().with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(ListDirTool::new()));
            self.register_sync(Arc::new(ApplyPatchTool::new().with_host(file_host)));
            self.register_sync(Arc::new(GrepTool::new()));
        }
    }

    /// Register extension-management tools from a host-provided lifecycle port.
    pub fn register_extension_management_tools(&self, port: Arc<dyn ExtensionManagementPort>) {
        self.register_sync(Arc::new(ToolSearchTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolInstallTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolAuthTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolActivateTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolListTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolRemoveTool::new(port)));
    }

    /// Register desktop-autonomy tools from a host-provided desktop port.
    pub fn register_desktop_autonomy_tools(&self, port: Arc<dyn DesktopAutonomyPort>) {
        self.register_sync(Arc::new(DesktopAutonomyTool::apps(Arc::clone(&port))));
        self.register_sync(Arc::new(DesktopAutonomyTool::ui(Arc::clone(&port))));
        self.register_sync(Arc::new(DesktopAutonomyTool::screen(Arc::clone(&port))));
        self.register_sync(Arc::new(DesktopAutonomyTool::calendar_native(Arc::clone(
            &port,
        ))));
        self.register_sync(Arc::new(DesktopAutonomyTool::numbers_native(Arc::clone(
            &port,
        ))));
        self.register_sync(Arc::new(DesktopAutonomyTool::pages_native(Arc::clone(
            &port,
        ))));
        self.register_sync(Arc::new(DesktopAutonomyTool::control(port)));
    }

    /// Register agent-management tools from a host-provided agent registry port.
    pub fn register_agent_management_tools(&self, port: Arc<dyn AgentManagementPort>) {
        self.register_sync(Arc::new(CreateAgentTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ListAgentsTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(UpdateAgentTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(RemoveAgentTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(MessageAgentTool::new(port)));
    }

    /// Register LLM model selection/discovery tools.
    pub fn register_llm_tools(
        &self,
        model_override: SharedModelOverride,
        primary_llm: Arc<dyn LlmProvider>,
        cheap_llm: Option<Arc<dyn LlmProvider>>,
    ) {
        self.register_sync(Arc::new(LlmSelectTool::new(model_override)));
        self.register_sync(Arc::new(LlmListModelsTool::new(primary_llm, cheap_llm)));
    }

    /// Register advisor consultation when the advisor lane is ready.
    pub fn register_advisor_tool(&self, advisor_ready: bool) {
        if advisor_ready {
            self.register_sync(Arc::new(ConsultAdvisorTool));
        }
    }

    /// Reconcile advisor tool visibility with current advisor readiness.
    pub async fn reconcile_advisor_tool_readiness(&self, advisor_ready: bool) {
        if advisor_ready {
            self.register_advisor_tool(true);
        } else {
            let _ = self.unregister(ADVISOR_TOOL_NAME).await;
        }
    }

    /// Register the extracted vision analysis tool.
    pub fn register_vision_tool(&self, llm: Arc<dyn LlmProvider>) {
        self.register_sync(Arc::new(VisionAnalyzeTool::new(llm)));
    }

    /// Register the extracted Mixture-of-Agents tool if the model set is viable.
    pub fn register_moa_tool(
        &self,
        primary: Arc<dyn LlmProvider>,
        cheap: Option<Arc<dyn LlmProvider>>,
        reference_models: Vec<String>,
        aggregator_model: Option<String>,
        min_successful: usize,
    ) -> bool {
        let tool = MoaTool::new(
            primary,
            cheap,
            reference_models,
            aggregator_model,
            min_successful,
        );
        if !tool.is_viable() {
            return false;
        }
        self.register_sync(Arc::new(tool));
        true
    }

    /// Register the extracted cross-platform send-message tool.
    pub fn register_send_message_tool(&self, send_fn: Option<SendMessageFn>) {
        let mut tool = SendMessageTool::new();
        if let Some(send_fn) = send_fn {
            tool = tool.with_send_fn(send_fn);
        }
        self.register_sync(Arc::new(tool));
    }

    /// Register the extracted background process tool.
    pub fn register_process_tool(
        &self,
        registry: SharedProcessRegistry,
        backend: Option<Arc<dyn LocalExecutionBackend>>,
    ) {
        let mut tool = ProcessTool::new(registry);
        if let Some(backend) = backend {
            tool = tool.with_backend(backend);
        }
        self.register_sync(Arc::new(tool));
    }

    /// Register the extracted in-session todo tool.
    pub fn register_todo_tool(&self, store: SharedTodoStore) {
        self.register_sync(Arc::new(TodoTool::new(store)));
    }

    /// Register the extracted TTS tool.
    pub fn register_tts_tool(
        &self,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
        output_dir: PathBuf,
    ) {
        self.register_sync(Arc::new(TtsTool::new(secrets, output_dir)));
    }

    /// Register the extracted Apple Mail tool.
    pub fn register_apple_mail_tool(&self, db_path: Option<PathBuf>) -> bool {
        let tool = if let Some(path) = db_path {
            AppleMailTool::new(path)
        } else if let Some(tool) = AppleMailTool::auto_detect() {
            tool
        } else {
            return false;
        };
        self.register_sync(Arc::new(tool));
        true
    }

    /// Register the extracted filename/path search tool.
    pub fn register_search_files_tool(&self, base_dir: Option<PathBuf>) {
        let mut tool = SearchFilesTool::new();
        if let Some(base_dir) = base_dir {
            tool = tool.with_base_dir(base_dir);
        }
        self.register_sync(Arc::new(tool));
    }

    /// Register a WASM tool from bytes.
    ///
    /// This validates and compiles the component, wraps it as a tool, and
    /// optionally publishes declared credential mappings for the shared HTTP
    /// injection registry.
    #[cfg(feature = "wasm-runtime")]
    pub async fn register_wasm_tool<I>(
        &self,
        reg: WasmToolRegistration<'_, I>,
        credential_registry: Option<&SharedCredentialRegistry>,
    ) -> Result<(), WasmError>
    where
        I: HostToolInvoker + 'static,
    {
        let prepared = reg
            .runtime
            .prepare(reg.name, reg.wasm_bytes, reg.limits)
            .await?;

        let credential_mappings = reg
            .capabilities
            .http
            .as_ref()
            .map(|http| http.credentials.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();

        let mut wrapper = WasmToolWrapper::new(Arc::clone(reg.runtime), prepared, reg.capabilities);

        if let Some(description) = reg.description {
            wrapper = wrapper.with_description(description);
        }
        if let Some(schema) = reg.schema {
            wrapper = wrapper.with_schema(schema);
        }
        if let Some(store) = reg.secrets_store {
            wrapper = wrapper.with_secrets_store(store);
        }
        if let Some(oauth) = reg.oauth_refresh {
            wrapper = wrapper.with_oauth_refresh(oauth);
        }
        if let Some(invoker) = reg.tool_invoker {
            wrapper = wrapper.with_tool_invoker(invoker);
        }

        self.register(Arc::new(wrapper)).await;

        if let Some(registry) = credential_registry
            && !credential_mappings.is_empty()
        {
            let count = credential_mappings.len();
            registry.add_mappings(credential_mappings);
            tracing::debug!(
                name = reg.name,
                credential_count = count,
                "Added credential mappings from WASM tool"
            );
        }

        tracing::info!(name = reg.name, "Registered WASM tool");
        Ok(())
    }

    /// Register a WASM tool from persisted storage.
    #[cfg(feature = "wasm-runtime")]
    pub async fn register_wasm_tool_from_storage<I>(
        &self,
        store: &dyn WasmToolStore,
        runtime: &Arc<WasmToolRuntime>,
        user_id: &str,
        name: &str,
        tool_invoker: Option<Arc<I>>,
        credential_registry: Option<&SharedCredentialRegistry>,
    ) -> Result<(), WasmRegistrationError>
    where
        I: HostToolInvoker + 'static,
    {
        let tool_with_binary = store
            .get_with_binary(user_id, name)
            .await
            .map_err(WasmRegistrationError::Storage)?;

        let capabilities = store
            .get_capabilities(tool_with_binary.tool.id)
            .await
            .map_err(WasmRegistrationError::Storage)?
            .map(|capabilities| capabilities.to_capabilities())
            .unwrap_or_default();

        self.register_wasm_tool(
            WasmToolRegistration {
                name: &tool_with_binary.tool.name,
                wasm_bytes: &tool_with_binary.wasm_binary,
                runtime,
                capabilities,
                limits: None,
                description: Some(&tool_with_binary.tool.description),
                schema: Some(tool_with_binary.tool.parameters_schema.clone()),
                secrets_store: None,
                oauth_refresh: None,
                tool_invoker,
            },
            credential_registry,
        )
        .await
        .map_err(WasmRegistrationError::Wasm)?;

        tracing::info!(
            name = tool_with_binary.tool.name,
            user_id = user_id,
            trust_level = %tool_with_binary.tool.trust_level,
            "Registered WASM tool from storage"
        );

        Ok(())
    }

    /// Unregister a tool.
    pub async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.write().await.remove(name)
    }

    /// Get a tool by name.
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.read().await.get(name).cloned()
    }

    /// Check if a tool exists.
    pub async fn has(&self, name: &str) -> bool {
        self.tools.read().await.contains_key(name)
    }

    /// List all tool names.
    pub async fn list(&self) -> Vec<String> {
        self.tools.read().await.keys().cloned().collect()
    }

    /// Get the number of registered tools.
    pub fn count(&self) -> usize {
        self.tools.try_read().map(|t| t.len()).unwrap_or(0)
    }

    /// Get all tools.
    pub async fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.read().await.values().cloned().collect()
    }

    /// Get tool descriptors for internal routing and policy decisions.
    pub async fn tool_descriptors(&self) -> Vec<ToolDescriptor> {
        let mut descriptors = self
            .tools
            .read()
            .await
            .values()
            .map(|tool| tool.descriptor())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    /// Get a single tool descriptor by name.
    pub async fn tool_descriptor(&self, name: &str) -> Option<ToolDescriptor> {
        self.tools
            .read()
            .await
            .get(name)
            .map(|tool| tool.descriptor())
    }

    fn descriptor_to_definition(descriptor: ToolDescriptor) -> ToolDefinition {
        ToolDefinition {
            name: descriptor.name,
            description: descriptor.description,
            parameters: descriptor.parameters,
        }
    }

    /// Get tool definitions for LLM function calling.
    pub async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_descriptors()
            .await
            .into_iter()
            .map(Self::descriptor_to_definition)
            .collect()
    }

    /// Parse an optional string-array allowlist from metadata.
    pub fn metadata_string_list(metadata: &serde_json::Value, key: &str) -> Option<Vec<String>> {
        metadata.get(key).and_then(|value| {
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
        })
    }

    /// Check whether a skill is allowed by metadata-scoped capabilities.
    pub fn skill_name_allowed_by_metadata(metadata: &serde_json::Value, skill_name: &str) -> bool {
        match Self::metadata_string_list(metadata, "allowed_skills") {
            Some(allowed_skills) => {
                let allowed: HashSet<&str> = allowed_skills.iter().map(String::as_str).collect();
                allowed.contains(skill_name)
            }
            None => true,
        }
    }

    /// Check whether a tool name is allowed by the provided capability bundle.
    pub fn tool_name_allowed_for_capabilities(
        tool_name: &str,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
    ) -> bool {
        if allowed_skills.is_some() && SKILL_ADMIN_TOOLS.contains(&tool_name) {
            return false;
        }

        match allowed_tools {
            Some(allowed_tools) => {
                allowed_tools.iter().any(|name| name == tool_name)
                    || IMPLICIT_CAPABILITY_TOOLS.contains(&tool_name)
            }
            None => true,
        }
    }

    /// Check whether a tool name is allowed by metadata-scoped capabilities.
    pub fn tool_name_allowed_by_metadata(metadata: &serde_json::Value, tool_name: &str) -> bool {
        let allowed_tools = Self::metadata_string_list(metadata, "allowed_tools");
        let allowed_skills = Self::metadata_string_list(metadata, "allowed_skills");
        Self::tool_name_allowed_for_capabilities(
            tool_name,
            allowed_tools.as_deref(),
            allowed_skills.as_deref(),
        )
    }

    fn filter_tool_definitions_for_capabilities(
        defs: Vec<ToolDefinition>,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
        visible_hidden_tools: Option<&[String]>,
    ) -> Vec<ToolDefinition> {
        defs.into_iter()
            .filter(|def| {
                Self::tool_name_allowed_for_capabilities(&def.name, allowed_tools, allowed_skills)
                    && Self::tool_name_visible_for_turn(&def.name, visible_hidden_tools)
            })
            .collect()
    }

    /// Filter tool definitions by execution lane/profile metadata in addition to capability grants.
    pub async fn filter_tool_definitions_for_execution_profile(
        &self,
        defs: Vec<ToolDefinition>,
        lane: ToolExecutionLane,
        profile: ToolProfile,
        metadata: &serde_json::Value,
    ) -> Vec<ToolDefinition> {
        let allowed_names: HashSet<String> = self
            .all()
            .await
            .into_iter()
            .filter_map(|tool| {
                let descriptor = tool.descriptor();
                (tool_allowed_for_lane(tool.as_ref(), &descriptor, lane)
                    && descriptor_allowed_for_profile(&descriptor, lane, profile, metadata))
                .then_some(descriptor.name)
            })
            .collect();

        defs.into_iter()
            .filter(|def| allowed_names.contains(&def.name))
            .collect()
    }

    fn tool_name_visible_for_turn(
        tool_name: &str,
        visible_hidden_tools: Option<&[String]>,
    ) -> bool {
        if !HIDDEN_BY_DEFAULT_TOOL_NAMES.contains(&tool_name) {
            return true;
        }

        visible_hidden_tools
            .map(|visible| visible.iter().any(|name| name == tool_name))
            .unwrap_or(false)
    }

    /// Get tool definitions filtered for a routed agent/subagent capability bundle.
    pub async fn tool_definitions_for_capabilities(
        &self,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
        visible_hidden_tools: Option<&[String]>,
    ) -> Vec<ToolDefinition> {
        let defs = self.tool_definitions().await;
        Self::filter_tool_definitions_for_capabilities(
            defs,
            allowed_tools,
            allowed_skills,
            visible_hidden_tools,
        )
    }

    /// Get tool definitions filtered for autonomous execution.
    pub async fn tool_definitions_for_autonomous(&self) -> Vec<ToolDefinition> {
        const DISPATCHER_ONLY_TOOLS: &[&str] =
            &["spawn_subagent", "list_subagents", "cancel_subagent"];

        let mut defs = self
            .all()
            .await
            .into_iter()
            .filter(|tool| {
                tool.requires_approval(&serde_json::json!({})) != ApprovalRequirement::Always
                    && !DISPATCHER_ONLY_TOOLS.contains(&tool.name())
            })
            .map(|tool| tool.descriptor())
            .collect::<Vec<_>>();
        defs.sort_by(|left, right| left.name.cmp(&right.name));
        let defs = defs
            .into_iter()
            .map(Self::descriptor_to_definition)
            .collect();
        Self::filter_tool_definitions_for_capabilities(defs, None, None, None)
    }

    /// Get autonomous tool definitions filtered by a capability bundle.
    pub async fn tool_definitions_for_autonomous_capabilities(
        &self,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
        visible_hidden_tools: Option<&[String]>,
    ) -> Vec<ToolDefinition> {
        let defs = self.tool_definitions_for_autonomous().await;
        Self::filter_tool_definitions_for_capabilities(
            defs,
            allowed_tools,
            allowed_skills,
            visible_hidden_tools,
        )
    }

    /// Get tool definitions for specific tools.
    pub async fn tool_definitions_for(&self, names: &[&str]) -> Vec<ToolDefinition> {
        let tools = self.tools.read().await;
        names
            .iter()
            .filter_map(|name| tools.get(*name))
            .map(|tool| Self::descriptor_to_definition(tool.descriptor()))
            .collect()
    }

    /// Get tool definitions filtered by domain.
    pub async fn tool_definitions_for_domain(&self, domain: ToolDomain) -> Vec<ToolDefinition> {
        let mut defs = self
            .all()
            .await
            .into_iter()
            .filter(|tool| tool.domain() == domain)
            .map(|tool| Self::descriptor_to_definition(tool.descriptor()))
            .collect::<Vec<_>>();
        defs.sort_by(|left, right| left.name.cmp(&right.name));
        defs
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("count", &self.count())
            .finish()
    }
}

pub fn deny_reason_for_profile(
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
    profile: ToolProfile,
    metadata: &serde_json::Value,
) -> Option<String> {
    if !ToolRegistry::tool_name_allowed_by_metadata(metadata, &descriptor.name) {
        return Some("Tool is not permitted in this agent context".to_string());
    }

    let explicit_tools = ToolRegistry::metadata_string_list(metadata, "allowed_tools");
    if descriptor.is_coordination_tool() {
        return None;
    }

    if let Some(explicit_tools) = explicit_tools {
        if explicit_tools.iter().any(|name| name == &descriptor.name) {
            return None;
        }

        return Some(format!(
            "Tool '{}' is not granted in this delegated context. Add it to allowed_tools or keep this step in the main agent.",
            descriptor.name
        ));
    }

    let implicitly_allowed = match profile {
        ToolProfile::Standard => true,
        ToolProfile::Restricted => descriptor.is_safe_read_only_orchestrator(),
        ToolProfile::ExplicitOnly => false,
        ToolProfile::Acp => descriptor_allowed_for_acp(descriptor),
    };

    if implicitly_allowed {
        None
    } else {
        Some(format!(
            "Tool '{}' is blocked in the {} lane under the '{}' tool profile. Grant it explicitly via allowed_tools or keep this work in the main agent.",
            descriptor.name,
            lane.as_str(),
            profile.as_str()
        ))
    }
}

fn descriptor_allowed_for_acp(descriptor: &ToolDescriptor) -> bool {
    let name = descriptor.name.as_str();
    if descriptor.is_coordination_tool() {
        return true;
    }

    matches!(
        name,
        "read_file"
            | "write_file"
            | "list_dir"
            | "apply_patch"
            | "grep"
            | "search_files"
            | "shell"
            | "process"
            | "execute_code"
            | "session_search"
            | "browser"
            | "vision_analyze"
            | "llm_list_models"
            | "llm_select"
    ) || name.starts_with("memory_")
        || name.starts_with("external_memory_")
        || name.starts_with("skill_")
}

pub fn deny_reason_for_lane(
    tool: &dyn Tool,
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
) -> Option<String> {
    if !matches!(
        lane,
        ToolExecutionLane::Scheduler
            | ToolExecutionLane::Worker
            | ToolExecutionLane::WorkerRuntime
            | ToolExecutionLane::Subagent
    ) {
        return None;
    }

    const DISPATCHER_ONLY_TOOLS: &[&str] = &["spawn_subagent", "list_subagents", "cancel_subagent"];
    if DISPATCHER_ONLY_TOOLS.contains(&descriptor.name.as_str()) {
        return Some(format!(
            "Tool '{}' requires dispatcher interception and is not available in the {} lane.",
            descriptor.name,
            lane.as_str()
        ));
    }

    if tool.requires_approval(&serde_json::json!({})) == ApprovalRequirement::Always {
        return Some(format!(
            "Tool '{}' requires explicit human approval and cannot run in the {} lane.",
            descriptor.name,
            lane.as_str()
        ));
    }

    None
}

/// Check whether a tool descriptor is usable for the given lane/profile/metadata tuple.
pub fn descriptor_allowed_for_profile(
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
    profile: ToolProfile,
    metadata: &serde_json::Value,
) -> bool {
    deny_reason_for_profile(descriptor, lane, profile, metadata).is_none()
}

/// Check whether a concrete tool may be exposed/executed in the given lane at all.
pub fn tool_allowed_for_lane(
    tool: &dyn Tool,
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
) -> bool {
    deny_reason_for_lane(tool, descriptor, lane).is_none()
}

/// Error when registering a WASM tool from storage.
#[cfg(feature = "wasm-runtime")]
#[derive(Debug, thiserror::Error)]
pub enum WasmRegistrationError {
    #[error("Storage error: {0}")]
    Storage(#[from] WasmStorageError),

    #[error("WASM error: {0}")]
    Wasm(#[from] WasmError),
}

/// Configuration for registering a WASM tool with the extracted registry.
#[cfg(feature = "wasm-runtime")]
pub struct WasmToolRegistration<'a, I: HostToolInvoker> {
    /// Unique name for the tool.
    pub name: &'a str,
    /// Raw WASM component bytes.
    pub wasm_bytes: &'a [u8],
    /// WASM runtime for compilation and execution.
    pub runtime: &'a Arc<WasmToolRuntime>,
    /// Security capabilities to grant the tool.
    pub capabilities: Capabilities,
    /// Optional resource limits (uses runtime defaults if omitted).
    pub limits: Option<ResourceLimits>,
    /// Optional description override.
    pub description: Option<&'a str>,
    /// Optional parameter schema override.
    pub schema: Option<serde_json::Value>,
    /// Secrets store for credential injection at request time.
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// OAuth refresh configuration for auto-refreshing expired tokens.
    pub oauth_refresh: Option<OAuthRefreshConfig>,
    /// Optional host-mediated bridge for WASM tool_invoke aliases.
    pub tool_invoker: Option<Arc<I>>,
}
