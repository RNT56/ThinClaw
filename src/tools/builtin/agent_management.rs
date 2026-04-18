//! Agent management tools — create, list, update, remove, and message agents.
//!
//! These tools allow the LLM to manage persistent agent workspaces and
//! communicate with other agents (A2A) through conversation.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::agent_registry::AgentRegistry;
use crate::context::JobContext;
use crate::tools::ToolProfile;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

fn parse_tool_profile_param(value: &str) -> Result<ToolProfile, ToolError> {
    match value {
        "standard" => Ok(ToolProfile::Standard),
        "restricted" => Ok(ToolProfile::Restricted),
        "explicit_only" => Ok(ToolProfile::ExplicitOnly),
        other => Err(ToolError::InvalidParameters(format!(
            "Invalid tool_profile '{other}'"
        ))),
    }
}

// ── CreateAgentTool ──────────────────────────────────────────────────

/// Tool for creating a new persistent agent workspace.
pub struct CreateAgentTool {
    registry: Arc<AgentRegistry>,
}

impl CreateAgentTool {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for CreateAgentTool {
    fn name(&self) -> &str {
        "create_agent"
    }

    fn description(&self) -> &str {
        "Create a new persistent agent workspace. Each agent has its own identity, \
         system prompt, model, memory, and workspace. Agents persist across restarts. \
         Use this to create specialized agents for specific tasks (code review, research, etc.)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "Unique ID for the agent (2-32 chars, lowercase, alphanumeric/hyphens/underscores only). Example: 'code-reviewer', 'research_bot'"
                },
                "display_name": {
                    "type": "string",
                    "description": "Human-friendly display name (1-64 chars). Example: 'Code Reviewer'"
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional system prompt defining the agent's personality and capabilities"
                },
                "model": {
                    "type": "string",
                    "description": "Optional model override (e.g. 'openai/gpt-4o', 'anthropic/claude-sonnet'). Uses parent's model if unset."
                },
                "channels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: channels this agent is bound to (e.g. ['telegram']). Empty = all channels."
                },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: trigger keywords that route messages to this agent."
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: tool allowlist for this agent. When set, only these tools plus core internal tools are exposed."
                },
                "allowed_skills": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: skill allowlist for this agent. When set, only these skills are visible/readable."
                },
                "tool_profile": {
                    "type": "string",
                    "enum": ["standard", "restricted", "explicit_only"],
                    "description": "Optional: execution profile override for this agent. Omit to inherit the main-agent profile."
                },
                "is_default": {
                    "type": "boolean",
                    "description": "Whether this is the default agent that receives unrouted messages. Default: false."
                }
            },
            "required": ["agent_id", "display_name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'agent_id'".into())
            })?;

        let display_name = params
            .get("display_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'display_name'".into())
            })?;

        let system_prompt = params.get("system_prompt").and_then(|v| v.as_str());
        let model = params.get("model").and_then(|v| v.as_str());
        let channels: Vec<String> = params
            .get("channels")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let keywords: Vec<String> = params
            .get("keywords")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let allowed_tools: Option<Vec<String>> = params
            .get("allowed_tools")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let allowed_skills: Option<Vec<String>> = params
            .get("allowed_skills")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let tool_profile = params
            .get("tool_profile")
            .and_then(|v| v.as_str())
            .map(parse_tool_profile_param)
            .transpose()?;
        let is_default = params
            .get("is_default")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match self
            .registry
            .create_agent(
                agent_id,
                display_name,
                system_prompt,
                model,
                channels,
                keywords,
                is_default,
                allowed_tools,
                allowed_skills,
                tool_profile,
            )
            .await
        {
            Ok(record) => Ok(ToolOutput::text(
                serde_json::to_string_pretty(&json!({
                    "status": "created",
                    "agent_id": record.agent_id,
                    "display_name": record.display_name,
                    "model": record.model,
                    "is_default": record.is_default,
                    "allowed_tools": record.allowed_tools,
                    "allowed_skills": record.allowed_skills,
                    "tool_profile": record.tool_profile.map(|profile| profile.as_str().to_string()),
                    "workspace_seeded": true,
                    "duration_ms": start.elapsed().as_millis(),
                }))
                .unwrap(),
                start.elapsed(),
            )),
            Err(e) => Ok(ToolOutput::text(
                format!("Error creating agent: {e}"),
                start.elapsed(),
            )),
        }
    }
}

// ── ListAgentsTool ───────────────────────────────────────────────────

/// Tool for listing all registered agent workspaces.
pub struct ListAgentsTool {
    registry: Arc<AgentRegistry>,
}

impl ListAgentsTool {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ListAgentsTool {
    fn name(&self) -> &str {
        "list_agents"
    }

    fn description(&self) -> &str {
        "List all registered agent workspaces with their configurations. \
         Shows agent IDs, display names, models, channel bindings, and more."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let agents = self.registry.list_agents().await;

        let agent_list: Vec<serde_json::Value> = agents
            .iter()
            .map(|ws| {
                json!({
                    "agent_id": ws.agent_id,
                    "display_name": ws.display_name,
                    "model": ws.model,
                    "is_default": ws.is_default,
                    "bound_channels": ws.bound_channels,
                    "trigger_keywords": ws.trigger_keywords,
                    "allowed_tools": ws.allowed_tools,
                    "allowed_skills": ws.allowed_skills,
                    "tool_profile": ws.tool_profile.map(|profile| profile.as_str().to_string()),
                    "has_system_prompt": ws.system_prompt.is_some(),
                })
            })
            .collect();

        let default_agent = agents
            .iter()
            .find(|ws| ws.is_default)
            .map(|ws| &ws.agent_id);

        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&json!({
                "agents": agent_list,
                "total": agents.len(),
                "default_agent": default_agent,
            }))
            .unwrap(),
            start.elapsed(),
        ))
    }
}

// ── UpdateAgentTool ──────────────────────────────────────────────────

/// Tool for updating an existing agent workspace.
pub struct UpdateAgentTool {
    registry: Arc<AgentRegistry>,
}

impl UpdateAgentTool {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for UpdateAgentTool {
    fn name(&self) -> &str {
        "update_agent"
    }

    fn description(&self) -> &str {
        "Update an existing agent workspace configuration. \
         Only specified fields are updated; others remain unchanged."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "ID of the agent to update"
                },
                "display_name": {
                    "type": "string",
                    "description": "New display name"
                },
                "system_prompt": {
                    "type": "string",
                    "description": "New system prompt (set to null to clear)"
                },
                "model": {
                    "type": "string",
                    "description": "New model override (set to null to clear)"
                },
                "channels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New channel bindings"
                },
                "keywords": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New trigger keywords"
                },
                "allowed_tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New tool allowlist (set to null to clear)"
                },
                "allowed_skills": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New skill allowlist (set to null to clear)"
                },
                "tool_profile": {
                    "type": "string",
                    "enum": ["standard", "restricted", "explicit_only"],
                    "description": "New execution profile override (set to null to clear)"
                },
                "is_default": {
                    "type": "boolean",
                    "description": "Set as default agent"
                }
            },
            "required": ["agent_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'agent_id'".into())
            })?;

        let display_name = params.get("display_name").and_then(|v| v.as_str());
        let system_prompt = if params.get("system_prompt").is_some() {
            Some(params.get("system_prompt").and_then(|v| v.as_str()))
        } else {
            None
        };
        let model = if params.get("model").is_some() {
            Some(params.get("model").and_then(|v| v.as_str()))
        } else {
            None
        };
        let channels: Option<Vec<String>> = params
            .get("channels")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let keywords: Option<Vec<String>> = params
            .get("keywords")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let allowed_tools: Option<Option<Vec<String>>> = if params.get("allowed_tools").is_some() {
            Some(
                params
                    .get("allowed_tools")
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
            )
        } else {
            None
        };
        let allowed_skills: Option<Option<Vec<String>>> = if params.get("allowed_skills").is_some()
        {
            Some(
                params
                    .get("allowed_skills")
                    .and_then(|v| serde_json::from_value(v.clone()).ok()),
            )
        } else {
            None
        };
        let tool_profile: Option<Option<ToolProfile>> = if params.get("tool_profile").is_some() {
            Some(
                params
                    .get("tool_profile")
                    .and_then(|v| v.as_str())
                    .map(parse_tool_profile_param)
                    .transpose()?,
            )
        } else {
            None
        };
        let is_default = params.get("is_default").and_then(|v| v.as_bool());

        match self
            .registry
            .update_agent(
                agent_id,
                display_name,
                system_prompt,
                model,
                channels,
                keywords,
                is_default,
                allowed_tools,
                allowed_skills,
                tool_profile,
            )
            .await
        {
            Ok(record) => Ok(ToolOutput::text(
                serde_json::to_string_pretty(&json!({
                    "status": "updated",
                    "agent_id": record.agent_id,
                    "display_name": record.display_name,
                    "allowed_tools": record.allowed_tools,
                    "allowed_skills": record.allowed_skills,
                    "tool_profile": record.tool_profile.map(|profile| profile.as_str().to_string()),
                }))
                .unwrap(),
                start.elapsed(),
            )),
            Err(e) => Ok(ToolOutput::text(
                format!("Error updating agent: {e}"),
                start.elapsed(),
            )),
        }
    }
}

// ── RemoveAgentTool ──────────────────────────────────────────────────

/// Tool for removing a persistent agent workspace.
pub struct RemoveAgentTool {
    registry: Arc<AgentRegistry>,
}

impl RemoveAgentTool {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for RemoveAgentTool {
    fn name(&self) -> &str {
        "remove_agent"
    }

    fn description(&self) -> &str {
        "Remove a persistent agent workspace. The agent's memory and workspace data \
         will be preserved in the database but the agent will no longer receive messages. \
         Cannot remove the default agent unless force=true."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "ID of the agent to remove"
                },
                "force": {
                    "type": "boolean",
                    "description": "Force remove even if this is the default agent. Default: false."
                }
            },
            "required": ["agent_id"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'agent_id'".into())
            })?;

        let force = params
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        match self.registry.remove_agent(agent_id, force).await {
            Ok(true) => Ok(ToolOutput::text(
                serde_json::to_string_pretty(&json!({
                    "status": "removed",
                    "agent_id": agent_id,
                }))
                .unwrap(),
                start.elapsed(),
            )),
            Ok(false) => Ok(ToolOutput::text(
                format!("Agent '{}' not found", agent_id),
                start.elapsed(),
            )),
            Err(e) => Ok(ToolOutput::text(
                format!("Error removing agent: {e}"),
                start.elapsed(),
            )),
        }
    }
}

// ── MessageAgentTool ─────────────────────────────────────────────────

/// Tool for sending a message to another agent (A2A communication).
///
/// Runs a mini agentic loop with the target agent's workspace context,
/// system prompt, and model. Returns the target agent's response.
pub struct MessageAgentTool {
    registry: Arc<AgentRegistry>,
}

impl MessageAgentTool {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for MessageAgentTool {
    fn name(&self) -> &str {
        "message_agent"
    }

    fn description(&self) -> &str {
        "Send a message to another agent and receive their response (Agent-to-Agent communication). \
         The target agent processes the message with its own system prompt, workspace memory, \
         and model. Use this to collaborate with specialized agents (e.g., ask the code-reviewer \
         to review a function, or the researcher to look up information)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "ID of the target agent to message"
                },
                "message": {
                    "type": "string",
                    "description": "The message to send to the target agent"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional: timeout in seconds (default: 120, max: 300)"
                }
            },
            "required": ["agent_id", "message"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'agent_id'".into())
            })?;

        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'message'".into())
            })?;

        let timeout_secs = params
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(120)
            .min(300);

        // Resolve the target agent
        let record = match self.registry.get_agent_record(agent_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                return Ok(ToolOutput::text(
                    format!(
                        "Error: Agent '{}' not found. Use list_agents to see available agents.",
                        agent_id
                    ),
                    start.elapsed(),
                ));
            }
            Err(e) => {
                return Ok(ToolOutput::text(
                    format!("Error looking up agent '{}': {}", agent_id, e),
                    start.elapsed(),
                ));
            }
        };

        // Build target agent's workspace
        let user_id = &ctx.user_id;
        let target_workspace = self.registry.build_workspace_for_agent(&record, user_id);

        // Load target agent's system prompt: explicit config > workspace IDENTITY.md
        let system_prompt = if let Some(ref prompt) = record.system_prompt {
            prompt.clone()
        } else if let Some(ref ws) = target_workspace {
            match ws.read("IDENTITY.md").await {
                Ok(doc) => doc.content,
                Err(_) => format!("You are {}.", record.display_name),
            }
        } else {
            format!("You are {}.", record.display_name)
        };

        // Build context with target agent's workspace memory
        let mut context_parts = vec![system_prompt];

        // Load target agent's workspace memory into context
        if let Some(ref ws) = target_workspace
            && let Ok(memory) = ws.read("MEMORY.md").await
        {
            let content = memory.content.trim();
            if !content.is_empty() {
                context_parts.push(format!("\n--- Your Memory (MEMORY.md) ---\n{}", content));
            }
        }

        let full_context = context_parts.join("\n");

        // Return a structured A2A response that the dispatcher can use.
        // The actual LLM call happens at a higher level — the dispatcher
        // intercepts this result similar to how it handles spawn_subagent.
        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&json!({
                "a2a_request": true,
                "target_agent_id": agent_id,
                "target_workspace_id": record.id.to_string(),
                "target_display_name": record.display_name,
                "target_model": record.model,
                "target_allowed_tools": record.allowed_tools,
                "target_allowed_skills": record.allowed_skills,
                "target_tool_profile": record.tool_profile.map(|profile| profile.as_str().to_string()),
                "target_system_prompt": full_context,
                "message": message,
                "timeout_secs": timeout_secs,
                "duration_ms": start.elapsed().as_millis() as u64,
            }))
            .unwrap(),
            start.elapsed(),
        ))
    }
}
