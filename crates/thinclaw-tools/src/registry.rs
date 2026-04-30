//! Root-independent tool registry storage and filtering.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use thinclaw_llm_core::ToolDefinition;
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

fn deny_reason_for_profile(
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

fn deny_reason_for_lane(
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
