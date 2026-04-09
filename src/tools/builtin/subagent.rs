//! Sub-agent tools — spawn, list, and cancel sub-agents from within the agentic loop.
//!
//! These tools enable the main agent to delegate parallel work to sub-agents.
//! The tools are intercepted by the dispatcher similarly to `emit_user_message`.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use crate::agent::subagent_executor::{SubagentExecutor, SubagentSpawnRequest};
use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

// ── SpawnSubagentTool ─────────────────────────────────────────────────

/// Tool for spawning a sub-agent to handle a delegated task.
pub struct SpawnSubagentTool {
    #[allow(dead_code)] // Retained for Arc reference counting; dispatcher handles execution
    executor: Arc<SubagentExecutor>,
}

impl SpawnSubagentTool {
    pub fn new(executor: Arc<SubagentExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl Tool for SpawnSubagentTool {
    fn name(&self) -> &str {
        "spawn_subagent"
    }

    fn description(&self) -> &str {
        "Spawn a focused sub-agent to handle a specific task. \
         With wait=true (default), the tool returns the sub-agent's result inline. \
         With wait=false, the sub-agent continues in the background and its result \
         is injected back automatically when it finishes. Use this to delegate \
         parallel work, break down complex tasks, or run independent research/analysis."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Short descriptive name for the sub-agent (e.g. 'researcher', 'code-reviewer', 'data-analyzer')"
                },
                "task": {
                    "type": "string",
                    "description": "Clear, specific task description for the sub-agent. Be detailed about what you need."
                },
                "tools": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: specific tools the sub-agent can use (e.g. ['http', 'shell', 'read_file']). If omitted, all tools are available."
                },
                "skills": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional: specific skills the sub-agent can read/list. If omitted, all skills remain visible."
                },
                "system_prompt": {
                    "type": "string",
                    "description": "Optional: custom system prompt for the sub-agent. If omitted, a task-focused default is used."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional: timeout in seconds (default: 300 = 5 minutes)"
                },
                "wait": {
                    "type": "boolean",
                    "description": "If true (default), wait for the sub-agent to complete and return its result. If false, spawn in background."
                }
            },
            "required": ["name", "task"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'name'".to_string())
            })?
            .to_string();

        let task = params
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'task'".to_string())
            })?
            .to_string();

        let tools = params.get("tools").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        });
        let skills = params.get("skills").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        });

        let system_prompt = params
            .get("system_prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let timeout_secs = params.get("timeout_secs").and_then(|v| v.as_u64());

        let wait = params.get("wait").and_then(|v| v.as_bool()).unwrap_or(true);

        let request = SubagentSpawnRequest {
            name,
            task,
            system_prompt,
            model: None,
            allowed_tools: tools,
            allowed_skills: skills,
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs,
            wait,
        };

        // The tool outputs a JSON action request.
        // The dispatcher intercepts this and routes it to the SubagentExecutor.
        let result = json!({
            "action": "spawn_subagent",
            "request": serde_json::to_value(&request).unwrap_or_default(),
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_approval(
        &self,
        _params: &serde_json::Value,
    ) -> crate::tools::tool::ApprovalRequirement {
        crate::tools::tool::ApprovalRequirement::Never
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ── ListSubagentsTool ─────────────────────────────────────────────────

/// Tool for listing active and recent sub-agents.
pub struct ListSubagentsTool {
    executor: Arc<SubagentExecutor>,
}

impl ListSubagentsTool {
    pub fn new(executor: Arc<SubagentExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl Tool for ListSubagentsTool {
    fn name(&self) -> &str {
        "list_subagents"
    }

    fn description(&self) -> &str {
        "List all active and recent sub-agents with their status, task description, \
         and timing information."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let agents = self.executor.list().await;

        if agents.is_empty() {
            return Ok(ToolOutput::success(
                json!({"subagents": [], "total": 0, "running": 0}),
                start.elapsed(),
            ));
        }

        let running = agents
            .iter()
            .filter(|a| a.status == crate::agent::subagent_executor::SubagentStatus::Running)
            .count();

        let result = json!({
            "subagents": agents,
            "total": agents.len(),
            "running": running,
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

// ── CancelSubagentTool ────────────────────────────────────────────────

/// Tool for cancelling a running sub-agent.
pub struct CancelSubagentTool {
    executor: Arc<SubagentExecutor>,
}

impl CancelSubagentTool {
    pub fn new(executor: Arc<SubagentExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl Tool for CancelSubagentTool {
    fn name(&self) -> &str {
        "cancel_subagent"
    }

    fn description(&self) -> &str {
        "Cancel a running sub-agent by its ID. Use list_subagents to find agent IDs."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "agent_id": {
                    "type": "string",
                    "description": "UUID of the sub-agent to cancel"
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

        let agent_id_str = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                ToolError::InvalidParameters("Missing required parameter 'agent_id'".to_string())
            })?;

        let agent_id = uuid::Uuid::parse_str(agent_id_str)
            .map_err(|_| ToolError::InvalidParameters(format!("Invalid UUID: {}", agent_id_str)))?;

        let cancelled = self.executor.cancel(agent_id).await;

        let result = if cancelled {
            json!({
                "status": "cancelled",
                "agent_id": agent_id_str,
            })
        } else {
            json!({
                "status": "not_found_or_already_done",
                "agent_id": agent_id_str,
            })
        };

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_request_serialization() {
        let request = SubagentSpawnRequest {
            name: "researcher".to_string(),
            task: "Find papers about AI".to_string(),
            system_prompt: None,
            model: None,
            allowed_tools: Some(vec!["http".to_string(), "read_file".to_string()]),
            allowed_skills: None,
            principal_id: None,
            actor_id: None,
            agent_workspace_id: None,
            timeout_secs: Some(120),
            wait: true,
        };
        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("researcher"));
        assert!(json.contains("Find papers about AI"));

        let deserialized: SubagentSpawnRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "researcher");
        assert_eq!(deserialized.allowed_tools.unwrap().len(), 2);
    }

    #[test]
    fn test_spawn_request_defaults() {
        let json = r#"{"name":"test","task":"do work"}"#;
        let request: SubagentSpawnRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.name, "test");
        assert!(request.system_prompt.is_none());
        assert!(request.model.is_none());
        assert!(request.allowed_tools.is_none());
        assert!(request.timeout_secs.is_none());
        assert!(!request.wait);
    }
}
