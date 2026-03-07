//! Sub-agent executor — lightweight in-process agentic loops.
//!
//! Spawns parallel sub-agents as tokio tasks, each with their own LLM context,
//! system prompt, and filtered tool set. Results are injected back into the
//! parent agent's message stream via an mpsc channel.
//!
//! ```text
//!   Main Agent ─────►  SubagentExecutor::spawn()
//!                          │
//!                          ├── tokio::spawn(mini_agentic_loop)
//!                          │       ├── LLM call with task prompt
//!                          │       ├── Tool calls (filtered set)
//!                          │       ├── emit_user_message → forwarded
//!                          │       └── Return final text response
//!                          │
//!                          └── inject_tx → parent loop
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, watch};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::channels::{ChannelManager, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::llm::{
    ChatMessage, LlmProvider, Reasoning, ReasoningContext, RespondResult, ToolDefinition,
};
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

/// Maximum tool iterations for a sub-agent (less than the main agent).
const SUBAGENT_MAX_ITERATIONS: usize = 30;

/// Default sub-agent timeout.
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// Maximum number of concurrent sub-agents.
const DEFAULT_MAX_CONCURRENT: usize = 5;

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
}

impl Default for SubagentConfig {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT,
            default_timeout_secs: DEFAULT_TIMEOUT_SECS,
            allow_nested: false,
            max_tool_iterations: SUBAGENT_MAX_ITERATIONS,
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

/// Handle to a running sub-agent.
pub struct SubagentHandle {
    pub id: Uuid,
    pub name: String,
    pub task: String,
    pub status: SubagentStatus,
    pub spawned_at: chrono::DateTime<chrono::Utc>,
    cancel_tx: watch::Sender<bool>,
    join_handle: Option<JoinHandle<SubagentResult>>,
}

/// Request to spawn a sub-agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpawnRequest {
    /// Display name for the sub-agent.
    pub name: String,
    /// Task description — becomes the user message in the sub-agent's context.
    pub task: String,
    /// Optional custom system prompt. If None, a task-focused default is used.
    pub system_prompt: Option<String>,
    /// Optional model override for the sub-agent.
    pub model: Option<String>,
    /// Optional list of allowed tool names. If None, all tools are available.
    pub allowed_tools: Option<Vec<String>>,
    /// Timeout in seconds. Falls back to config default.
    pub timeout_secs: Option<u64>,
    /// If true, the parent waits for the sub-agent to complete.
    #[serde(default)]
    pub wait: bool,
}

/// The sub-agent executor manages sub-agent lifecycle.
pub struct SubagentExecutor {
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    channels: Arc<ChannelManager>,
    config: SubagentConfig,
    /// Currently active sub-agents.
    active: Arc<RwLock<HashMap<Uuid, SubagentHandle>>>,
}

impl SubagentExecutor {
    /// Create a new sub-agent executor.
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        safety: Arc<SafetyLayer>,
        tools: Arc<ToolRegistry>,
        channels: Arc<ChannelManager>,
        config: SubagentConfig,
    ) -> Self {
        Self {
            llm,
            safety,
            tools,
            channels,
            config,
            active: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Spawn a sub-agent.
    ///
    /// If `request.wait` is true, this blocks until the sub-agent completes and
    /// returns its result directly. If false, it spawns in the background and
    /// returns immediately with the agent ID.
    pub async fn spawn(
        &self,
        request: SubagentSpawnRequest,
        channel_name: &str,
        channel_metadata: &serde_json::Value,
    ) -> Result<SubagentResult, Error> {
        // Check concurrency limit
        {
            let active = self.active.read().await;
            let running = active
                .values()
                .filter(|h| h.status == SubagentStatus::Running)
                .count();
            if running >= self.config.max_concurrent {
                return Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
                    name: "spawn_subagent".to_string(),
                    reason: format!(
                        "Maximum concurrent sub-agents reached ({}/{})",
                        running, self.config.max_concurrent
                    ),
                }));
            }
        }

        let id = Uuid::new_v4();
        let (cancel_tx, cancel_rx) = watch::channel(false);

        let timeout = Duration::from_secs(
            request
                .timeout_secs
                .unwrap_or(self.config.default_timeout_secs),
        );
        let max_iterations = self.config.max_tool_iterations;

        // Build system prompt
        let system_prompt = request.system_prompt.clone().unwrap_or_else(|| {
            format!(
                "You are a focused sub-agent named '{}'. \
                 You have been delegated a specific task by the main agent.\n\n\
                 Complete the task thoroughly and concisely. \
                 Return a clear, actionable summary when done.\n\n\
                 You can use `emit_user_message` to share progress with the user.",
                request.name
            )
        });

        let name = request.name.clone();
        let task = request.task.clone();
        let wait = request.wait;

        // Clone shared deps for the spawned task
        let llm = self.llm.clone();
        let safety = self.safety.clone();
        let tools = self.tools.clone();
        let channels = self.channels.clone();
        let ch_name = channel_name.to_string();
        let ch_meta = channel_metadata.clone();
        let allowed = request.allowed_tools.clone();

        let agent_name = name.clone();
        let agent_task = task.clone();

        let join_handle = tokio::spawn(async move {
            let start = Instant::now();

            // Notify user that sub-agent has started
            let _ = channels
                .send_status(
                    &ch_name,
                    StatusUpdate::AgentMessage {
                        content: format!("🔀 Sub-agent '{}' started: {}", agent_name, agent_task),
                        message_type: "progress".to_string(),
                    },
                    &ch_meta,
                )
                .await;

            let result = tokio::time::timeout(
                timeout,
                run_subagent_loop(
                    llm,
                    safety,
                    tools,
                    channels.clone(),
                    &system_prompt,
                    &agent_task,
                    &ch_name,
                    &ch_meta,
                    cancel_rx,
                    max_iterations,
                    allowed.as_deref(),
                ),
            )
            .await;

            let elapsed = start.elapsed();

            match result {
                Ok(Ok((response, iterations))) => {
                    let _ = channels
                        .send_status(
                            &ch_name,
                            StatusUpdate::AgentMessage {
                                content: format!(
                                    "✅ Sub-agent '{}' completed ({} iterations, {:.1}s)",
                                    agent_name,
                                    iterations,
                                    elapsed.as_secs_f64()
                                ),
                                message_type: "progress".to_string(),
                            },
                            &ch_meta,
                        )
                        .await;

                    SubagentResult {
                        agent_id: id,
                        name: agent_name,
                        response,
                        iterations,
                        duration_ms: elapsed.as_millis() as u64,
                        success: true,
                        error: None,
                    }
                }
                Ok(Err(e)) => {
                    let err_msg = e.to_string();
                    let _ = channels
                        .send_status(
                            &ch_name,
                            StatusUpdate::AgentMessage {
                                content: format!(
                                    "❌ Sub-agent '{}' failed: {}",
                                    agent_name, err_msg
                                ),
                                message_type: "warning".to_string(),
                            },
                            &ch_meta,
                        )
                        .await;

                    SubagentResult {
                        agent_id: id,
                        name: agent_name,
                        response: String::new(),
                        iterations: 0,
                        duration_ms: elapsed.as_millis() as u64,
                        success: false,
                        error: Some(err_msg),
                    }
                }
                Err(_timeout) => {
                    let _ = channels
                        .send_status(
                            &ch_name,
                            StatusUpdate::AgentMessage {
                                content: format!(
                                    "⏰ Sub-agent '{}' timed out after {:.0}s",
                                    agent_name,
                                    elapsed.as_secs_f64()
                                ),
                                message_type: "warning".to_string(),
                            },
                            &ch_meta,
                        )
                        .await;

                    SubagentResult {
                        agent_id: id,
                        name: agent_name,
                        response: String::new(),
                        iterations: 0,
                        duration_ms: elapsed.as_millis() as u64,
                        success: false,
                        error: Some("Timed out".to_string()),
                    }
                }
            }
        });

        // Track the handle
        {
            let mut active = self.active.write().await;
            active.insert(
                id,
                SubagentHandle {
                    id,
                    name: name.clone(),
                    task: task.clone(),
                    status: SubagentStatus::Running,
                    spawned_at: chrono::Utc::now(),
                    cancel_tx,
                    join_handle: if wait { Some(join_handle) } else { None },
                },
            );
        }

        if wait {
            // Wait for the sub-agent to complete
            let handle = {
                let mut active = self.active.write().await;
                active.get_mut(&id).and_then(|h| h.join_handle.take())
            };

            if let Some(jh) = handle {
                let result = jh.await.map_err(|e| {
                    Error::Tool(crate::error::ToolError::ExecutionFailed {
                        name: "spawn_subagent".to_string(),
                        reason: format!("Sub-agent task panicked: {}", e),
                    })
                })?;

                // Update status
                {
                    let mut active = self.active.write().await;
                    if let Some(h) = active.get_mut(&id) {
                        h.status = if result.success {
                            SubagentStatus::Completed
                        } else if result.error.as_deref() == Some("Timed out") {
                            SubagentStatus::TimedOut
                        } else {
                            SubagentStatus::Failed(result.error.clone().unwrap_or_default())
                        };
                    }
                }

                Ok(result)
            } else {
                Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
                    name: "spawn_subagent".to_string(),
                    reason: "Failed to get sub-agent join handle".to_string(),
                }))
            }
        } else {
            // Fire and forget — return immediately with the ID
            Ok(SubagentResult {
                agent_id: id,
                name,
                response: format!("Sub-agent spawned (id: {})", id),
                iterations: 0,
                duration_ms: 0,
                success: true,
                error: None,
            })
        }
    }

    /// Cancel a running sub-agent.
    pub async fn cancel(&self, agent_id: Uuid) -> bool {
        let mut active = self.active.write().await;
        if let Some(handle) = active.get_mut(&agent_id) {
            if handle.status == SubagentStatus::Running {
                let _ = handle.cancel_tx.send(true);
                handle.status = SubagentStatus::Cancelled;
                // Abort the task if we have a handle
                if let Some(jh) = handle.join_handle.take() {
                    jh.abort();
                }
                tracing::info!(agent_id = %agent_id, "Sub-agent cancelled");
                return true;
            }
        }
        false
    }

    /// List all sub-agents (active and finished).
    pub async fn list(&self) -> Vec<SubagentInfo> {
        let active = self.active.read().await;
        active
            .values()
            .map(|h| SubagentInfo {
                id: h.id,
                name: h.name.clone(),
                task: h.task.clone(),
                status: h.status.clone(),
                spawned_at: h.spawned_at.to_rfc3339(),
            })
            .collect()
    }

    /// Count running sub-agents.
    pub async fn running_count(&self) -> usize {
        let active = self.active.read().await;
        active
            .values()
            .filter(|h| h.status == SubagentStatus::Running)
            .count()
    }

    /// Remove completed/failed entries older than the given duration.
    pub async fn cleanup(&self, max_age: Duration) {
        let cutoff = chrono::Utc::now() - chrono::Duration::from_std(max_age).unwrap_or_default();
        let mut active = self.active.write().await;
        active.retain(|_, h| h.status == SubagentStatus::Running || h.spawned_at > cutoff);
    }
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

/// Run a mini agentic loop for a sub-agent.
///
/// This is a simplified version of `Agent::run_agentic_loop()` that doesn't
/// need sessions, threads, undo, compaction, or hooks. It just runs
/// LLM → tool → LLM → ... until a text response or iteration limit.
#[allow(clippy::too_many_arguments)]
async fn run_subagent_loop(
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
    tools: Arc<ToolRegistry>,
    channels: Arc<ChannelManager>,
    system_prompt: &str,
    task: &str,
    channel_name: &str,
    channel_metadata: &serde_json::Value,
    cancel_rx: watch::Receiver<bool>,
    max_iterations: usize,
    allowed_tools: Option<&[String]>,
) -> Result<(String, usize), Error> {
    let mut context_messages = vec![ChatMessage::user(task.to_string())];

    let job_ctx = JobContext::with_user("subagent", "subagent", "Sub-agent task");

    // Build tool definitions (filtered if needed)
    let all_defs = tools.tool_definitions().await;
    let tool_defs: Vec<ToolDefinition> = match allowed_tools {
        Some(names) => all_defs
            .into_iter()
            .filter(|d| {
                names.iter().any(|n| n == &d.name)
                    || d.name == "agent_think"
                    || d.name == "emit_user_message"
            })
            .collect(),
        None => all_defs,
    };

    let reasoning =
        Reasoning::new(llm, safety.clone()).with_system_prompt(system_prompt.to_string());

    for iteration in 0..max_iterations {
        // Check cancellation
        if *cancel_rx.borrow() {
            return Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
                name: "subagent".to_string(),
                reason: "Cancelled".to_string(),
            }));
        }

        // Force text on last iteration
        let force_text = iteration >= max_iterations.saturating_sub(1);

        let ctx = ReasoningContext {
            messages: context_messages.clone(),
            available_tools: if force_text {
                vec![]
            } else {
                tool_defs.clone()
            },
            job_description: None,
            current_state: None,
            metadata: std::collections::HashMap::new(),
            force_text,
            thinking: Default::default(),
        };

        let output = reasoning.respond_with_tools(&ctx).await?;

        match output.result {
            RespondResult::Text(text) => {
                return Ok((text, iteration + 1));
            }
            RespondResult::ToolCalls {
                tool_calls,
                content,
            } => {
                // Add the assistant message with tool calls to context
                context_messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                // Execute each tool call
                for tc in tool_calls {
                    // Handle emit_user_message specially — forward to user
                    if tc.name == "emit_user_message" {
                        let content_val = tc
                            .arguments
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("[no content]");
                        let msg_type = tc
                            .arguments
                            .get("message_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("progress");

                        let _ = channels
                            .send_status(
                                channel_name,
                                StatusUpdate::AgentMessage {
                                    content: content_val.to_string(),
                                    message_type: msg_type.to_string(),
                                },
                                channel_metadata,
                            )
                            .await;

                        context_messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            serde_json::json!({
                                "status": "message_sent",
                                "message_type": msg_type,
                            })
                            .to_string(),
                        ));
                        continue;
                    }

                    // Handle agent_think — just record it
                    if tc.name == "agent_think" {
                        let thought = tc
                            .arguments
                            .get("thought")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        context_messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            serde_json::json!({
                                "status": "thought_recorded",
                                "thought": thought,
                            })
                            .to_string(),
                        ));
                        continue;
                    }

                    // Execute normal tool
                    let tool = match tools.get(&tc.name).await {
                        Some(t) => t,
                        None => {
                            context_messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                format!("Error: tool '{}' not found", tc.name),
                            ));
                            continue;
                        }
                    };

                    // Check tool is in allowed list
                    if let Some(names) = allowed_tools {
                        if !names.iter().any(|n| n == &tc.name) {
                            context_messages.push(ChatMessage::tool_result(
                                &tc.id,
                                &tc.name,
                                format!("Error: tool '{}' not allowed for this sub-agent", tc.name),
                            ));
                            continue;
                        }
                    }

                    let tool_timeout = tool.execution_timeout();
                    let result = tokio::time::timeout(
                        tool_timeout,
                        tool.execute(tc.arguments.clone(), &job_ctx),
                    )
                    .await;

                    let result_str = match result {
                        Ok(Ok(output)) => {
                            // Convert serde_json::Value to string for the tool result
                            let raw = match &output.result {
                                serde_json::Value::String(s) => s.clone(),
                                other => other.to_string(),
                            };
                            let sanitized = safety.sanitize_tool_output(&tc.name, &raw);
                            sanitized.content
                        }
                        Ok(Err(e)) => format!("Error: {}", e),
                        Err(_) => format!("Error: tool '{}' timed out", tc.name),
                    };

                    context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, result_str));
                }
            }
        }
    }

    Err(Error::Tool(crate::error::ToolError::ExecutionFailed {
        name: "subagent".to_string(),
        reason: format!("Exceeded maximum iterations ({})", max_iterations),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_config_defaults() {
        let config = SubagentConfig::default();
        assert_eq!(config.max_concurrent, 5);
        assert_eq!(config.default_timeout_secs, 300);
        assert!(!config.allow_nested);
        assert_eq!(config.max_tool_iterations, 30);
    }

    #[test]
    fn test_subagent_status_equality() {
        assert_eq!(SubagentStatus::Running, SubagentStatus::Running);
        assert_ne!(SubagentStatus::Running, SubagentStatus::Completed);
        assert_eq!(
            SubagentStatus::Failed("err".to_string()),
            SubagentStatus::Failed("err".to_string())
        );
    }

    #[test]
    fn test_subagent_result_serialization() {
        let result = SubagentResult {
            agent_id: Uuid::new_v4(),
            name: "researcher".to_string(),
            response: "Found 3 papers".to_string(),
            iterations: 5,
            duration_ms: 3200,
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("researcher"));
        assert!(json.contains("Found 3 papers"));

        let deserialized: SubagentResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "researcher");
        assert_eq!(deserialized.iterations, 5);
    }

    #[test]
    fn test_spawn_request_with_defaults() {
        let req = SubagentSpawnRequest {
            name: "test".to_string(),
            task: "do something".to_string(),
            system_prompt: None,
            model: None,
            allowed_tools: None,
            timeout_secs: None,
            wait: true,
        };
        assert_eq!(req.name, "test");
        assert!(req.wait);
        assert!(req.allowed_tools.is_none());
    }
}
