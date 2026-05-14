//! Root-independent sub-agent DTOs and policy helpers.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thinclaw_identity::ResolvedIdentity;
use thinclaw_types::{
    SubagentMemoryMode, SubagentSkillMode, SubagentTaskPacket, SubagentToolMode, ToolProfile,
};
use uuid::Uuid;

pub const SUBAGENT_MAX_ITERATIONS: usize = 30;
pub const DEFAULT_TIMEOUT_SECS: u64 = 300;
pub const DEFAULT_MAX_CONCURRENT: usize = 5;
pub const SUBAGENT_PROGRESS_PREVIEW_MAX: usize = 80;

/// Configuration for the sub-agent system.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// Maximum number of concurrent sub-agents.
    pub max_concurrent: usize,
    /// Default timeout for sub-agents in seconds.
    pub default_timeout_secs: u64,
    /// Whether sub-agents can spawn other sub-agents.
    pub allow_nested: bool,
    /// Maximum tool iterations per sub-agent.
    pub max_tool_iterations: usize,
    /// Default execution profile for delegated sub-agents.
    pub default_tool_profile: ToolProfile,
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            default_timeout_secs: DEFAULT_TIMEOUT_SECS,
            allow_nested: false,
            max_tool_iterations: SUBAGENT_MAX_ITERATIONS,
            default_tool_profile: ToolProfile::ExplicitOnly,
        }
    }
}

/// Result from a completed sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResult {
    /// The sub-agent's unique ID.
    pub agent_id: Uuid,
    /// Display name of the sub-agent.
    pub name: String,
    /// The sub-agent's final response text.
    pub response: String,
    /// How many tool iterations were used.
    pub iterations: usize,
    /// Duration the sub-agent ran.
    pub duration_ms: u64,
    /// Whether the sub-agent completed successfully.
    pub success: bool,
    /// Error message if the sub-agent failed.
    pub error: Option<String>,
}

/// Status of a running sub-agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SubagentStatus {
    Running,
    Completed,
    Failed(String),
    TimedOut,
    Cancelled,
}

/// A completed sub-agent result ready for injection into the main agent loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentResultMessage {
    /// The sub-agent result.
    pub result: SubagentResult,
    /// Channel the parent agent was on when it spawned this sub-agent.
    pub channel_name: String,
    /// User ID to re-inject the result under.
    pub parent_user_id: String,
    /// Resolved identity so the re-injected message lands in the same session scope.
    pub parent_identity: Option<ResolvedIdentity>,
    /// Metadata for routing (contains thread_id etc).
    pub channel_metadata: serde_json::Value,
    /// Thread ID of the parent conversation.
    pub parent_thread_id: String,
}

/// Info about a sub-agent (serializable).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub id: Uuid,
    pub name: String,
    pub task: String,
    pub status: SubagentStatus,
    pub spawned_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentLearningCompletion {
    pub summary: &'static str,
    pub confidence: f32,
    pub correction_count: u64,
    pub repeated_failures: u64,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRoutineCompletion {
    pub run_status: crate::routine::RunStatus,
    pub summary: String,
    pub lifecycle_event: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct SubagentSystemPromptSections<'a> {
    pub workspace_prompt: Option<&'a str>,
    pub base_system_prompt: &'a str,
    pub task_packet: &'a SubagentTaskPacket,
    pub skill_context: Option<&'a str>,
    pub allowed_tools: Option<&'a [String]>,
    pub allowed_skills: Option<&'a [String]>,
    pub memory_mode: &'a SubagentMemoryMode,
    pub tool_mode: &'a SubagentToolMode,
    pub skill_mode: &'a SubagentSkillMode,
    pub tool_profile_label: &'a str,
}

/// Request to spawn a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnRequest {
    /// Display name for the sub-agent.
    pub name: String,
    /// Task description -- becomes the user message in the sub-agent's context.
    pub task: String,
    /// Optional custom system prompt. If None, a task-focused default is used.
    pub system_prompt: Option<String>,
    /// Optional model override for the sub-agent.
    pub model: Option<String>,
    /// Structured task packet used as the canonical bounded assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_packet: Option<SubagentTaskPacket>,
    /// How the sub-agent may source memory/context beyond the provided task packet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_mode: Option<SubagentMemoryMode>,
    /// Tool gating policy for the sub-agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_mode: Option<SubagentToolMode>,
    /// Skill gating policy for the sub-agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_mode: Option<SubagentSkillMode>,
    /// Optional execution profile override for the sub-agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_profile: Option<ToolProfile>,
    /// Optional list of allowed tool names. If None, all tools are available.
    pub allowed_tools: Option<Vec<String>>,
    /// Optional list of allowed skill names. If None, all skills remain visible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    /// Optional principal owner for workspace-scoped tool access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    /// Optional actor owner for actor-scoped memory overlays.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    /// Optional routed agent workspace UUID for memory/tool isolation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_workspace_id: Option<Uuid>,
    /// Timeout in seconds. Falls back to config default.
    pub timeout_secs: Option<u64>,
    /// If true, wait for the sub-agent to complete and return its result inline.
    /// If false, return immediately and re-inject the result on completion.
    #[serde(default)]
    pub wait: bool,
}

impl SubagentSpawnRequest {
    pub fn normalize_strict(
        &mut self,
        inherited_tools: Option<&[String]>,
        inherited_skills: Option<&[String]>,
        default_tool_profile: ToolProfile,
    ) {
        let objective = self
            .task_packet
            .as_ref()
            .map(|packet| packet.objective.trim().to_string())
            .filter(|objective| !objective.is_empty())
            .unwrap_or_else(|| self.task.trim().to_string());

        let packet = self
            .task_packet
            .get_or_insert_with(SubagentTaskPacket::default);
        packet.objective = objective.clone();
        packet.todos.retain(|item| !item.trim().is_empty());
        packet
            .acceptance_criteria
            .retain(|item| !item.trim().is_empty());
        packet.constraints.retain(|item| !item.trim().is_empty());
        packet
            .provided_context
            .retain(|item| !item.title.trim().is_empty() || !item.content.trim().is_empty());
        if packet
            .parent_summary
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            packet.parent_summary = None;
        }

        self.task = objective;
        self.memory_mode = Some(self.memory_mode.clone().unwrap_or_default());
        self.tool_mode = Some(self.tool_mode.clone().unwrap_or_default());
        self.skill_mode = Some(self.skill_mode.clone().unwrap_or_default());
        self.tool_profile = Some(self.tool_profile.unwrap_or(default_tool_profile));

        let requested_tools = self.allowed_tools.take();
        let normalized_tools = normalize_capability_allowlist(inherited_tools, requested_tools);
        self.allowed_tools = if inherited_tools.is_some()
            || self.tool_profile == Some(ToolProfile::ExplicitOnly)
            || !normalized_tools.is_empty()
        {
            Some(normalized_tools)
        } else {
            None
        };
        self.allowed_skills = Some(normalize_capability_allowlist(
            inherited_skills,
            self.allowed_skills.take(),
        ));
    }

    pub fn task_packet(&self) -> SubagentTaskPacket {
        let mut packet = self.task_packet.clone().unwrap_or_default();
        if packet.objective.trim().is_empty() {
            packet.objective = self.task.clone();
        }
        packet
    }
}

pub fn truncate_progress_preview(value: &str, max_len: usize) -> String {
    if value.chars().count() <= max_len {
        return value.to_string();
    }

    let truncated: String = value.chars().take(max_len.saturating_sub(3)).collect();
    format!("{truncated}...")
}

pub fn extract_subagent_message(arguments: &serde_json::Value) -> Option<String> {
    ["message", "content"]
        .into_iter()
        .find_map(|key| arguments.get(key).and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
}

pub fn with_subagent_thread_metadata(
    metadata: &serde_json::Value,
    parent_thread_id: &str,
    channel_name: &str,
) -> serde_json::Value {
    let mut merged = if metadata.is_object() {
        metadata.clone()
    } else {
        serde_json::json!({})
    };

    if let Some(object) = merged.as_object_mut() {
        object.insert(
            "channel".to_string(),
            serde_json::Value::String(channel_name.to_string()),
        );
        object.insert(
            "thread_id".to_string(),
            serde_json::Value::String(parent_thread_id.to_string()),
        );
    }

    merged
}

pub fn llm_metadata_from_json(value: &serde_json::Value) -> HashMap<String, String> {
    value
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| match value {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(text) => Some((key.clone(), text.clone())),
                    serde_json::Value::Bool(boolean) => Some((key.clone(), boolean.to_string())),
                    serde_json::Value::Number(number) => Some((key.clone(), number.to_string())),
                    serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                        serde_json::to_string(value)
                            .ok()
                            .map(|json| (key.clone(), json))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn normalize_subagent_progress_category(message_type: &str) -> &'static str {
    match message_type {
        "progress" => "milestone",
        "interim_result" => "finding",
        "question" => "question",
        "warning" => "warning",
        "tool" => "activity",
        _ => "update",
    }
}

pub fn subagent_status_from_result(result: &SubagentResult) -> SubagentStatus {
    if result.success {
        SubagentStatus::Completed
    } else if result.error.as_deref() == Some("Timed out") {
        SubagentStatus::TimedOut
    } else {
        SubagentStatus::Failed(
            result
                .error
                .clone()
                .unwrap_or_else(|| "Unknown error".to_string()),
        )
    }
}

pub fn subagent_learning_completion(result: &SubagentResult) -> SubagentLearningCompletion {
    SubagentLearningCompletion {
        summary: if result.success {
            "Sub-agent completed successfully"
        } else {
            "Sub-agent failed to complete task"
        },
        confidence: if result.success { 0.82 } else { 0.38 },
        correction_count: if result.success { 0 } else { 1 },
        repeated_failures: if result.success { 0 } else { 1 },
        metadata: serde_json::json!({
            "subagent_id": result.agent_id,
            "subagent_name": result.name,
            "success": result.success,
            "iterations": result.iterations,
            "duration_ms": result.duration_ms,
            "error": result.error,
            "response_preview": truncate_progress_preview(&result.response, 240),
            "target_type": "subagent",
            "target": result.name,
            "correction_count": if result.success { 0 } else { 1 },
            "repeated_failures": if result.success { 0 } else { 1 },
        }),
    }
}

pub fn subagent_routine_completion(result: &SubagentResult) -> SubagentRoutineCompletion {
    SubagentRoutineCompletion {
        run_status: if result.success {
            crate::routine::RunStatus::Ok
        } else {
            crate::routine::RunStatus::Failed
        },
        summary: if result.success {
            result.response.clone()
        } else {
            result
                .error
                .clone()
                .unwrap_or_else(|| "Unknown error".to_string())
        },
        lifecycle_event: if result.success {
            "completed"
        } else {
            "failed"
        },
    }
}

pub fn subagent_routine_actor(
    parent_identity_actor: Option<&str>,
    request_actor_id: Option<&str>,
    parent_user_id: &str,
) -> String {
    parent_identity_actor
        .or(request_actor_id)
        .unwrap_or(parent_user_id)
        .to_string()
}

pub fn should_reinject_subagent_result(metadata: &serde_json::Value) -> bool {
    metadata
        .get("reinject_result")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

pub fn render_subagent_system_prompt(sections: SubagentSystemPromptSections<'_>) -> String {
    let mut rendered = Vec::new();

    if let Some(workspace_prompt) = sections
        .workspace_prompt
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push(workspace_prompt.to_string());
    }

    rendered.push(format!(
        "## Sub-agent Mission\n\n{}",
        sections.base_system_prompt
    ));
    rendered.push(format!(
        "## Task Packet\n\n{}",
        render_task_packet(sections.task_packet)
    ));
    rendered.push(format!(
        "## Operating Contract\n\n\
         - Use the supplied task packet as the primary source of truth.\n\
         - Do not assume access to the parent agent's broader memory, transcript history, or personal context.\n\
         - Do not browse or search for additional context unless the parent explicitly granted the necessary tools.\n\
         - If the packet is insufficient, ask the parent for what is missing instead of widening scope.\n\
         - Complete the bounded assignment against the acceptance criteria and todos.\n\n\
         Memory mode: `{}`\n\
         Tool mode: `{}`\n\
         Tool profile: `{}`\n\
         Skill mode: `{}`\n\
         Explicit tool grants: {}\n\
         Explicit skill grants: {}",
        subagent_memory_mode_label(sections.memory_mode),
        subagent_tool_mode_label(sections.tool_mode),
        sections.tool_profile_label,
        subagent_skill_mode_label(sections.skill_mode),
        format_allowlist(sections.allowed_tools),
        format_allowlist(sections.allowed_skills),
    ));

    if let Some(skill_context) = sections
        .skill_context
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        rendered.push(format!("## Skills\n{skill_context}"));
    }

    rendered.join("\n\n")
}

pub fn subagent_tool_activity_message(tool_name: &str, arguments: &serde_json::Value) -> String {
    let tool_label = tool_name.replace('_', " ");

    if let Some(path) = first_argument_preview(arguments, &["path", "target", "file"]) {
        return format!("Running {tool_label} on {path}");
    }

    if let Some(query) = first_argument_preview(arguments, &["query", "q", "pattern", "task"]) {
        return format!("Running {tool_label} for {query}");
    }

    if let Some(url) = first_argument_preview(arguments, &["url"]) {
        return format!("Running {tool_label} on {url}");
    }

    if let Some(command) = first_argument_preview(arguments, &["command", "cmd"]) {
        return format!("Running {tool_label}: {command}");
    }

    format!("Running {tool_label}")
}

pub fn subagent_tool_warning_message(tool_name: &str, detail: &str) -> String {
    format!(
        "{tool_name} needs attention: {}",
        truncate_progress_preview(detail.trim(), SUBAGENT_PROGRESS_PREVIEW_MAX)
    )
}

pub fn normalize_capability_allowlist(
    inherited: Option<&[String]>,
    requested: Option<Vec<String>>,
) -> Vec<String> {
    let mut merged = match (inherited, requested) {
        (Some(inherited), Some(requested)) => {
            let inherited: std::collections::HashSet<&str> =
                inherited.iter().map(String::as_str).collect();
            requested
                .into_iter()
                .filter(|name| inherited.contains(name.as_str()))
                .collect::<Vec<_>>()
        }
        (Some(inherited), None) => inherited.to_vec(),
        (None, Some(requested)) => requested,
        (None, None) => Vec::new(),
    };
    merged.sort();
    merged.dedup();
    merged
}

pub fn subagent_memory_tool_names() -> &'static [&'static str] {
    &[
        "session_search",
        "memory_search",
        "memory_read",
        "external_memory_recall",
        "external_memory_status",
    ]
}

pub fn filter_tools_for_memory_mode(
    tools: Vec<String>,
    memory_mode: &SubagentMemoryMode,
) -> Vec<String> {
    if *memory_mode == SubagentMemoryMode::GrantedToolsOnly {
        return tools;
    }

    let blocked: std::collections::HashSet<&str> =
        subagent_memory_tool_names().iter().copied().collect();
    tools
        .into_iter()
        .filter(|tool| !blocked.contains(tool.as_str()))
        .collect()
}

pub fn subagent_memory_mode_label(mode: &SubagentMemoryMode) -> &'static str {
    match mode {
        SubagentMemoryMode::ProvidedContextOnly => "provided_context_only",
        SubagentMemoryMode::GrantedToolsOnly => "granted_tools_only",
    }
}

pub fn subagent_tool_mode_label(mode: &SubagentToolMode) -> &'static str {
    match mode {
        SubagentToolMode::ExplicitOnly => "explicit_only",
    }
}

pub fn subagent_skill_mode_label(mode: &SubagentSkillMode) -> &'static str {
    match mode {
        SubagentSkillMode::ExplicitOnly => "explicit_only",
    }
}

pub fn render_task_packet(packet: &SubagentTaskPacket) -> String {
    let mut sections = vec![format!("Objective: {}", packet.objective.trim())];

    if !packet.todos.is_empty() {
        sections.push(format!(
            "Todos:\n{}",
            packet
                .todos
                .iter()
                .map(|item| format!("- {}", item.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !packet.acceptance_criteria.is_empty() {
        sections.push(format!(
            "Acceptance Criteria:\n{}",
            packet
                .acceptance_criteria
                .iter()
                .map(|item| format!("- {}", item.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !packet.constraints.is_empty() {
        sections.push(format!(
            "Constraints:\n{}",
            packet
                .constraints
                .iter()
                .map(|item| format!("- {}", item.trim()))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    if !packet.provided_context.is_empty() {
        sections.push(format!(
            "Provided Context:\n{}",
            packet
                .provided_context
                .iter()
                .map(|item| format!("### {}\n{}", item.title.trim(), item.content.trim()))
                .collect::<Vec<_>>()
                .join("\n\n")
        ));
    }

    if let Some(summary) = packet
        .parent_summary
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        sections.push(format!("Parent Summary:\n{}", summary.trim()));
    }

    sections.join("\n\n")
}

fn first_argument_preview(arguments: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| arguments.get(*key))
        .and_then(|value| match value {
            serde_json::Value::String(s) => Some(truncate_progress_preview(
                s.trim(),
                SUBAGENT_PROGRESS_PREVIEW_MAX,
            )),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        })
        .filter(|value| !value.is_empty())
}

fn format_allowlist(items: Option<&[String]>) -> String {
    items
        .map(|items| {
            if items.is_empty() {
                "none".to_string()
            } else {
                items.join(", ")
            }
        })
        .unwrap_or_else(|| "none".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strict_inherits_parent_tool_and_skill_ceilings() {
        let mut request = SubagentSpawnRequest {
            name: "worker".to_string(),
            task: "  Do work  ".to_string(),
            system_prompt: None,
            model: None,
            task_packet: None,
            memory_mode: None,
            tool_mode: None,
            skill_mode: None,
            tool_profile: None,
            allowed_tools: None,
            allowed_skills: None,
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs: None,
            wait: false,
        };

        request.normalize_strict(
            Some(&["shell".to_string(), "read_file".to_string()]),
            Some(&["rust".to_string()]),
            ToolProfile::ExplicitOnly,
        );

        assert_eq!(request.task, "Do work");
        assert_eq!(
            request.allowed_tools,
            Some(vec!["read_file".to_string(), "shell".to_string()])
        );
        assert_eq!(request.allowed_skills, Some(vec!["rust".to_string()]));
    }

    #[test]
    fn subagent_tool_activity_message_uses_argument_hints() {
        assert_eq!(
            subagent_tool_activity_message("read_file", &serde_json::json!({"path": "/tmp/a"})),
            "Running read file on /tmp/a"
        );
    }

    #[test]
    fn subagent_status_and_completion_policy_follow_result() {
        let success = SubagentResult {
            agent_id: Uuid::new_v4(),
            name: "worker".to_string(),
            response: "done".to_string(),
            iterations: 3,
            duration_ms: 42,
            success: true,
            error: None,
        };
        assert_eq!(
            subagent_status_from_result(&success),
            SubagentStatus::Completed
        );
        assert_eq!(
            subagent_routine_completion(&success).lifecycle_event,
            "completed"
        );
        assert_eq!(subagent_learning_completion(&success).confidence, 0.82);

        let timeout = SubagentResult {
            success: false,
            error: Some("Timed out".to_string()),
            ..success.clone()
        };
        assert_eq!(
            subagent_status_from_result(&timeout),
            SubagentStatus::TimedOut
        );
        assert_eq!(subagent_routine_completion(&timeout).summary, "Timed out");
    }

    #[test]
    fn render_system_prompt_includes_contract_and_grants() {
        let packet = SubagentTaskPacket {
            objective: "Inspect the adapter".to_string(),
            todos: vec!["Read code".to_string()],
            acceptance_criteria: vec![],
            constraints: vec![],
            provided_context: vec![],
            parent_summary: None,
        };
        let tools = vec!["read_file".to_string()];
        let skills: Vec<String> = Vec::new();
        let prompt = render_subagent_system_prompt(SubagentSystemPromptSections {
            workspace_prompt: Some("Workspace prompt"),
            base_system_prompt: "You are focused.",
            task_packet: &packet,
            skill_context: Some("### Active Skills"),
            allowed_tools: Some(&tools),
            allowed_skills: Some(&skills),
            memory_mode: &SubagentMemoryMode::ProvidedContextOnly,
            tool_mode: &SubagentToolMode::ExplicitOnly,
            skill_mode: &SubagentSkillMode::ExplicitOnly,
            tool_profile_label: "explicit_only",
        });

        assert!(prompt.contains("Workspace prompt"));
        assert!(prompt.contains("Objective: Inspect the adapter"));
        assert!(prompt.contains("Explicit tool grants: read_file"));
        assert!(prompt.contains("Explicit skill grants: none"));
    }

    #[test]
    fn llm_metadata_from_json_stringifies_scalar_and_structured_values() {
        let metadata = llm_metadata_from_json(&serde_json::json!({
            "string": "value",
            "boolean": true,
            "number": 42,
            "object": { "nested": "yes" },
            "array": [1, 2],
            "null": null
        }));

        assert_eq!(metadata.get("string").map(String::as_str), Some("value"));
        assert_eq!(metadata.get("boolean").map(String::as_str), Some("true"));
        assert_eq!(metadata.get("number").map(String::as_str), Some("42"));
        assert_eq!(
            metadata.get("object").map(String::as_str),
            Some("{\"nested\":\"yes\"}")
        );
        assert_eq!(metadata.get("array").map(String::as_str), Some("[1,2]"));
        assert!(!metadata.contains_key("null"));
    }
}
