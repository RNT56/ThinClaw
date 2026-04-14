//! Dispatcher helper functions.
//!
//! Standalone utility functions extracted from `dispatcher.rs` to keep
//! the core agentic loop focused. Includes:
//! - Standalone tool execution (for parallel JoinSet tasks)
//! - Auth result parsing (tool_auth / tool_activate)
//! - Message compaction for context-length retries
//! - String truncation utilities

use crate::error::Error;
use crate::llm::ChatMessage;

/// Execute a chat tool without requiring `&Agent`.
///
/// This standalone function enables parallel invocation from spawned JoinSet
/// tasks, which cannot borrow `&self`. It replicates the logic from
/// `Agent::execute_chat_tool`.
pub(crate) async fn execute_chat_tool_standalone(
    tools: &crate::tools::ToolRegistry,
    safety: &crate::safety::SafetyLayer,
    tool_name: &str,
    params: &serde_json::Value,
    job_ctx: &crate::context::JobContext,
) -> Result<String, Error> {
    if !crate::tools::ToolRegistry::tool_name_allowed_by_metadata(&job_ctx.metadata, tool_name) {
        return Err(crate::error::ToolError::ExecutionFailed {
            name: tool_name.to_string(),
            reason: "Tool is not permitted in this agent context".to_string(),
        }
        .into());
    }

    let tool = tools
        .get(tool_name)
        .await
        .ok_or_else(|| crate::error::ToolError::NotFound {
            name: tool_name.to_string(),
        })?;

    // Validate tool parameters
    let validation = safety.validator().validate_tool_params(params);
    if !validation.is_valid {
        let details = validation
            .errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(crate::error::ToolError::InvalidParameters {
            name: tool_name.to_string(),
            reason: format!("Invalid tool parameters: {}", details),
        }
        .into());
    }

    tracing::debug!(
        tool = %tool_name,
        params = %params,
        "Tool call started"
    );

    // Execute with per-tool timeout
    let timeout = tool.execution_timeout();
    let start = std::time::Instant::now();
    let result = tokio::time::timeout(timeout, async {
        tool.execute(params.clone(), job_ctx).await
    })
    .await;
    let elapsed = start.elapsed();

    match &result {
        Ok(Ok(output)) => {
            let result_str = serde_json::to_string(&output.result)
                .unwrap_or_else(|_| "<serialize error>".to_string());
            tracing::debug!(
                tool = %tool_name,
                elapsed_ms = elapsed.as_millis() as u64,
                result = %result_str,
                "Tool call succeeded"
            );
        }
        Ok(Err(e)) => {
            tracing::debug!(
                tool = %tool_name,
                elapsed_ms = elapsed.as_millis() as u64,
                error = %e,
                "Tool call failed"
            );
        }
        Err(_) => {
            tracing::debug!(
                tool = %tool_name,
                elapsed_ms = elapsed.as_millis() as u64,
                timeout_secs = timeout.as_secs(),
                "Tool call timed out"
            );
        }
    }

    let result = result
        .map_err(|_| crate::error::ToolError::Timeout {
            name: tool_name.to_string(),
            timeout,
        })?
        .map_err(|e| crate::error::ToolError::ExecutionFailed {
            name: tool_name.to_string(),
            reason: e.to_string(),
        })?;

    serde_json::to_string_pretty(&result.result).map_err(|e| {
        crate::error::ToolError::ExecutionFailed {
            name: tool_name.to_string(),
            reason: format!("Failed to serialize result: {}", e),
        }
        .into()
    })
}

/// Parsed auth result fields for emitting StatusUpdate::AuthRequired.
pub(crate) struct ParsedAuthData {
    pub(crate) auth_url: Option<String>,
    pub(crate) setup_url: Option<String>,
}

/// Extract auth_url and setup_url from a tool_auth result JSON string.
pub(crate) fn parse_auth_result(result: &Result<String, Error>) -> ParsedAuthData {
    let parsed = result
        .as_ref()
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    ParsedAuthData {
        auth_url: parsed
            .as_ref()
            .and_then(|v| v.get("auth_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        setup_url: parsed
            .as_ref()
            .and_then(|v| v.get("setup_url"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    }
}

/// Check if a tool_auth result indicates the extension is awaiting a token.
///
/// Returns `Some((extension_name, instructions))` if the tool result contains
/// `awaiting_token: true`, meaning the thread should enter auth mode.
pub(crate) fn check_auth_required(
    tool_name: &str,
    result: &Result<String, Error>,
) -> Option<(String, String)> {
    if tool_name != "tool_auth" && tool_name != "tool_activate" {
        return None;
    }
    let output = result.as_ref().ok()?;
    let parsed: serde_json::Value = serde_json::from_str(output).ok()?;
    if parsed.get("awaiting_token") != Some(&serde_json::Value::Bool(true)) {
        return None;
    }
    let name = parsed.get("name")?.as_str()?.to_string();
    let instructions = parsed
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("Please provide your API token/key.")
        .to_string();
    Some((name, instructions))
}

/// Compact messages for retry after a context-length-exceeded error.
///
/// Keeps all `System` messages (which carry the system prompt and instructions),
/// finds the last `User` message, and retains it plus every subsequent message
/// (the current turn's assistant tool calls and tool results). A short note is
/// Truncate a string to `max_chars`, appending "…" if truncated.
/// Used for tool result previews in UI events.
pub(crate) fn truncate_preview(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let mut end = max_chars;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

/// inserted so the LLM knows earlier history was dropped.
pub(crate) fn compact_messages_for_retry(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    use crate::llm::Role;

    let mut compacted = Vec::new();

    // Find the last User message index
    let last_user_idx = messages.iter().rposition(|m| m.role == Role::User);

    if let Some(idx) = last_user_idx {
        // Keep System messages that appear BEFORE the last User message.
        // System messages after that point (e.g. nudges) are included in the
        // slice extension below, avoiding duplication.
        for msg in &messages[..idx] {
            if msg.role == Role::System {
                compacted.push(msg.clone());
            }
        }

        // Only add a compaction note if there was earlier history that is being dropped
        if idx > 0 {
            compacted.push(ChatMessage::system(
                "[Note: Earlier conversation history was automatically compacted \
                 to fit within the context window. The most recent exchange is preserved below.]",
            ));
        }

        // Keep the last User message and everything after it
        compacted.extend_from_slice(&messages[idx..]);
    } else {
        // No user messages found (shouldn't happen normally); keep everything,
        // with system messages first to preserve prompt ordering.
        for msg in messages {
            if msg.role == Role::System {
                compacted.push(msg.clone());
            }
        }
        for msg in messages {
            if msg.role != Role::System {
                compacted.push(msg.clone());
            }
        }
    }

    // Defensive sanitize: if the input messages already contained orphaned
    // tool_result messages (e.g. the hard cap fired just before a
    // ContextLengthExceeded retry), ensure they are promoted to user messages
    // before the compacted slice is returned to the caller. This makes the
    // function correct regardless of the caller's input state.
    crate::llm::sanitize_tool_messages(&mut compacted);

    compacted
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    use crate::agent::agent_loop::{Agent, AgentDeps};
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::agent::session::Session;
    use crate::channels::ChannelManager;
    use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
    use crate::context::ContextManager;
    use crate::error::Error;
    use crate::hooks::HookRegistry;
    use crate::llm::{
        CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCall,
        ToolCompletionRequest, ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::tools::ToolRegistry;

    use super::check_auth_required;

    /// Minimal LLM provider for unit tests that always returns a static response.
    struct StaticLlmProvider;

    #[async_trait]
    impl LlmProvider for StaticLlmProvider {
        fn model_name(&self) -> &str {
            "static-mock"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            Ok(CompletionResponse {
                content: "Sure, let me help you with that.".to_string(),
                provider_model: None,
                cost_usd: None,
                thinking_content: None,
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                provider_model: None,
                cost_usd: None,
                tool_calls: Vec::new(),
                thinking_content: None,
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
            })
        }
    }

    /// Build a minimal `Agent` for unit testing (no DB, no workspace, no extensions).
    fn make_test_agent() -> Agent {
        let deps = AgentDeps {
            store: None,
            llm: Arc::new(StaticLlmProvider),
            cheap_llm: None,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: true,
                redact_pii_in_prompts: true,
                smart_approval_mode: "off".to_string(),
                external_scanner_mode: "off".to_string(),
                external_scanner_path: None,
            })),
            tools: Arc::new(ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks: Arc::new(HookRegistry::new()),
            cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
            sse_sender: None,
            agent_router: None,
            agent_registry: None,
            canvas_store: None,
            subagent_executor: None,
            cost_tracker: None,
            response_cache: None,
            llm_runtime: None,
            routing_policy: None,
            model_override: None,
            restart_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        Agent::new(
            AgentConfig {
                name: "test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(60),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_tool_iterations: 50,
                max_context_messages: 200,
                thinking_enabled: false,
                thinking_budget_tokens: 10_000,
                auto_approve_tools: false,
                subagent_transparency_level: "balanced".to_string(),
                model_thinking_overrides: std::collections::HashMap::new(),
                workspace_mode: "unrestricted".to_string(),
                workspace_root: None,
                notify_channel: None,
                model_guidance_enabled: true,
                cli_skin: "cockpit".to_string(),
                persona_seed: "default".to_string(),
                checkpoints_enabled: true,
                max_checkpoints: 50,
                browser_backend: "chromium".to_string(),
                cloud_browser_provider: None,
            },
            deps,
            Arc::new(ChannelManager::new()),
            None,
            None,
            None,
            Some(Arc::new(ContextManager::new(1))),
            None,
        )
    }

    #[test]
    fn test_make_test_agent_succeeds() {
        // Verify that a test agent can be constructed without panicking.
        let _agent = make_test_agent();
    }

    #[test]
    fn test_auto_approved_tool_is_respected() {
        let _agent = make_test_agent();
        let mut session = Session::new("user-1");
        session.auto_approve_tool("http");

        // A non-shell tool that is auto-approved should be approved.
        assert!(session.is_tool_auto_approved("http"));
        // A tool that hasn't been auto-approved should not be.
        assert!(!session.is_tool_auto_approved("shell"));
    }

    #[test]
    fn test_shell_destructive_command_requires_explicit_approval() {
        // requires_explicit_approval() detects destructive commands that
        // should return ApprovalRequirement::Always from ShellTool.
        use crate::tools::builtin::shell::requires_explicit_approval;

        let destructive_cmds = [
            "rm -rf /tmp/test",
            "git push --force origin main",
            "git reset --hard HEAD~5",
        ];
        for cmd in &destructive_cmds {
            assert!(
                requires_explicit_approval(cmd),
                "'{}' should require explicit approval",
                cmd
            );
        }

        let safe_cmds = ["git status", "cargo build", "ls -la"];
        for cmd in &safe_cmds {
            assert!(
                !requires_explicit_approval(cmd),
                "'{}' should not require explicit approval",
                cmd
            );
        }
    }

    #[test]
    fn test_pending_approval_serialization_backcompat_without_deferred_calls() {
        // PendingApproval from before the deferred_tool_calls field was added
        // should deserialize with an empty vec (via #[serde(default)]).
        let json = serde_json::json!({
            "request_id": uuid::Uuid::new_v4(),
            "tool_name": "http",
            "parameters": {"url": "https://example.com", "method": "GET"},
            "description": "Make HTTP request",
            "tool_call_id": "call_123",
            "context_messages": [{"role": "user", "content": "go"}]
        })
        .to_string();

        let parsed: crate::agent::session::PendingApproval =
            serde_json::from_str(&json).expect("should deserialize without deferred_tool_calls");

        assert!(parsed.deferred_tool_calls.is_empty());
        assert_eq!(parsed.tool_name, "http");
        assert_eq!(parsed.tool_call_id, "call_123");
    }

    #[test]
    fn test_pending_approval_serialization_roundtrip_with_deferred_calls() {
        let pending = crate::agent::session::PendingApproval {
            request_id: uuid::Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "echo hi"}),
            description: "Run shell command".to_string(),
            tool_call_id: "call_1".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![
                ToolCall {
                    id: "call_2".to_string(),
                    name: "http".to_string(),
                    arguments: serde_json::json!({"url": "https://example.com"}),
                },
                ToolCall {
                    id: "call_3".to_string(),
                    name: "echo".to_string(),
                    arguments: serde_json::json!({"message": "done"}),
                },
            ],
        };

        let json = serde_json::to_string(&pending).expect("serialize");
        let parsed: crate::agent::session::PendingApproval =
            serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.deferred_tool_calls.len(), 2);
        assert_eq!(parsed.deferred_tool_calls[0].name, "http");
        assert_eq!(parsed.deferred_tool_calls[1].name, "echo");
    }

    #[test]
    fn test_detect_auth_awaiting_positive() {
        let result: Result<String, Error> = Ok(serde_json::json!({
            "name": "telegram",
            "kind": "WasmTool",
            "awaiting_token": true,
            "status": "awaiting_token",
            "instructions": "Please provide your Telegram Bot API token."
        })
        .to_string());

        let detected = check_auth_required("tool_auth", &result);
        assert!(detected.is_some());
        let (name, instructions) = detected.unwrap();
        assert_eq!(name, "telegram");
        assert!(instructions.contains("Telegram Bot API"));
    }

    #[test]
    fn test_detect_auth_awaiting_not_awaiting() {
        let result: Result<String, Error> = Ok(serde_json::json!({
            "name": "telegram",
            "kind": "WasmTool",
            "awaiting_token": false,
            "status": "authenticated"
        })
        .to_string());

        assert!(check_auth_required("tool_auth", &result).is_none());
    }

    #[test]
    fn test_detect_auth_awaiting_wrong_tool() {
        let result: Result<String, Error> = Ok(serde_json::json!({
            "name": "telegram",
            "awaiting_token": true,
        })
        .to_string());

        assert!(check_auth_required("tool_list", &result).is_none());
    }

    #[test]
    fn test_detect_auth_awaiting_error_result() {
        let result: Result<String, Error> =
            Err(crate::error::ToolError::NotFound { name: "x".into() }.into());
        assert!(check_auth_required("tool_auth", &result).is_none());
    }

    #[test]
    fn test_detect_auth_awaiting_default_instructions() {
        let result: Result<String, Error> = Ok(serde_json::json!({
            "name": "custom_tool",
            "awaiting_token": true,
            "status": "awaiting_token"
        })
        .to_string());

        let (_, instructions) = check_auth_required("tool_auth", &result).unwrap();
        assert_eq!(instructions, "Please provide your API token/key.");
    }

    #[test]
    fn test_detect_auth_awaiting_tool_activate() {
        let result: Result<String, Error> = Ok(serde_json::json!({
            "name": "slack",
            "kind": "McpServer",
            "awaiting_token": true,
            "status": "awaiting_token",
            "instructions": "Provide your Slack Bot token."
        })
        .to_string());

        let detected = check_auth_required("tool_activate", &result);
        assert!(detected.is_some());
        let (name, instructions) = detected.unwrap();
        assert_eq!(name, "slack");
        assert!(instructions.contains("Slack Bot"));
    }

    #[test]
    fn test_detect_auth_awaiting_tool_activate_not_awaiting() {
        let result: Result<String, Error> = Ok(serde_json::json!({
            "name": "slack",
            "tools_loaded": ["slack_post_message"],
            "message": "Activated"
        })
        .to_string());

        assert!(check_auth_required("tool_activate", &result).is_none());
    }

    #[tokio::test]
    async fn test_execute_chat_tool_standalone_success() {
        use crate::config::SafetyConfig;
        use crate::context::JobContext;
        use crate::safety::SafetyLayer;
        use crate::tools::ToolRegistry;
        use crate::tools::builtin::EchoTool;

        let registry = ToolRegistry::new();
        registry.register(std::sync::Arc::new(EchoTool)).await;

        let safety = SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "off".to_string(),
            external_scanner_path: None,
        });

        let job_ctx = JobContext::with_user("test", "chat", "test session");

        let result = super::execute_chat_tool_standalone(
            &registry,
            &safety,
            "echo",
            &serde_json::json!({"message": "hello"}),
            &job_ctx,
        )
        .await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_chat_tool_standalone_not_found() {
        use crate::config::SafetyConfig;
        use crate::context::JobContext;
        use crate::safety::SafetyLayer;
        use crate::tools::ToolRegistry;

        let registry = ToolRegistry::new();
        let safety = SafetyLayer::new(&SafetyConfig {
            max_output_length: 100_000,
            injection_check_enabled: false,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "off".to_string(),
            external_scanner_path: None,
        });
        let job_ctx = JobContext::with_user("test", "chat", "test session");

        let result = super::execute_chat_tool_standalone(
            &registry,
            &safety,
            "nonexistent",
            &serde_json::json!({}),
            &job_ctx,
        )
        .await;

        assert!(result.is_err());
    }

    // ---- compact_messages_for_retry tests ----

    use super::compact_messages_for_retry;
    use crate::llm::{ChatMessage, Role};

    #[test]
    fn test_compact_keeps_system_and_last_user_exchange() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("First question"),
            ChatMessage::assistant("First answer"),
            ChatMessage::user("Second question"),
            ChatMessage::assistant("Second answer"),
            ChatMessage::user("Third question"),
            ChatMessage::assistant_with_tool_calls(
                None,
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "echo".to_string(),
                    arguments: serde_json::json!({"message": "hi"}),
                }],
            ),
            ChatMessage::tool_result("call_1", "echo", "hi"),
        ];

        let compacted = compact_messages_for_retry(&messages);

        // Should have: system prompt + compaction note + last user msg + tool call + tool result
        assert_eq!(compacted.len(), 5);
        assert_eq!(compacted[0].role, Role::System);
        assert_eq!(compacted[0].content, "You are a helpful assistant.");
        assert_eq!(compacted[1].role, Role::System); // compaction note
        assert!(compacted[1].content.contains("compacted"));
        assert_eq!(compacted[2].role, Role::User);
        assert_eq!(compacted[2].content, "Third question");
        assert_eq!(compacted[3].role, Role::Assistant); // tool call
        assert_eq!(compacted[4].role, Role::Tool); // tool result
    }

    #[test]
    fn test_compact_preserves_multiple_system_messages() {
        let messages = vec![
            ChatMessage::system("System prompt"),
            ChatMessage::system("Skill context"),
            ChatMessage::user("Old question"),
            ChatMessage::assistant("Old answer"),
            ChatMessage::system("Nudge message"),
            ChatMessage::user("Current question"),
        ];

        let compacted = compact_messages_for_retry(&messages);

        // 3 system messages + compaction note + last user message
        assert_eq!(compacted.len(), 5);
        assert_eq!(compacted[0].content, "System prompt");
        assert_eq!(compacted[1].content, "Skill context");
        assert_eq!(compacted[2].content, "Nudge message");
        assert!(compacted[3].content.contains("compacted")); // note
        assert_eq!(compacted[4].content, "Current question");
    }

    #[test]
    fn test_compact_single_user_message_keeps_everything() {
        let messages = vec![
            ChatMessage::system("System prompt"),
            ChatMessage::user("Only question"),
        ];

        let compacted = compact_messages_for_retry(&messages);

        // system + compaction note + user
        assert_eq!(compacted.len(), 3);
        assert_eq!(compacted[0].content, "System prompt");
        assert!(compacted[1].content.contains("compacted"));
        assert_eq!(compacted[2].content, "Only question");
    }

    #[test]
    fn test_compact_no_user_messages_keeps_non_system() {
        let messages = vec![
            ChatMessage::system("System prompt"),
            ChatMessage::assistant("Stray assistant message"),
        ];

        let compacted = compact_messages_for_retry(&messages);

        // system + assistant (no user message found, keeps all non-system)
        assert_eq!(compacted.len(), 2);
        assert_eq!(compacted[0].role, Role::System);
        assert_eq!(compacted[1].role, Role::Assistant);
    }

    #[test]
    fn test_compact_drops_old_history_but_keeps_current_turn_tools() {
        // Simulate a multi-turn conversation where the current turn has
        // multiple tool calls and results.
        let messages = vec![
            ChatMessage::system("System prompt"),
            ChatMessage::user("Question 1"),
            ChatMessage::assistant("Answer 1"),
            ChatMessage::user("Question 2"),
            ChatMessage::assistant("Answer 2"),
            ChatMessage::user("Question 3"),
            ChatMessage::assistant("Answer 3"),
            ChatMessage::user("Current question"),
            ChatMessage::assistant_with_tool_calls(
                None,
                vec![
                    ToolCall {
                        id: "c1".to_string(),
                        name: "http".to_string(),
                        arguments: serde_json::json!({}),
                    },
                    ToolCall {
                        id: "c2".to_string(),
                        name: "echo".to_string(),
                        arguments: serde_json::json!({}),
                    },
                ],
            ),
            ChatMessage::tool_result("c1", "http", "response data"),
            ChatMessage::tool_result("c2", "echo", "echoed"),
        ];

        let compacted = compact_messages_for_retry(&messages);

        // system + note + user + assistant(tool_calls) + tool_result + tool_result
        assert_eq!(compacted.len(), 6);
        assert_eq!(compacted[0].content, "System prompt");
        assert!(compacted[1].content.contains("compacted"));
        assert_eq!(compacted[2].content, "Current question");
        assert!(compacted[3].tool_calls.is_some()); // assistant with tool calls
        assert_eq!(compacted[4].name.as_deref(), Some("http"));
        assert_eq!(compacted[5].name.as_deref(), Some("echo"));
    }

    #[test]
    fn test_compact_no_duplicate_system_after_last_user() {
        // A system nudge message injected AFTER the last user message must
        // not be duplicated — it should only appear once (via extend_from_slice).
        let messages = vec![
            ChatMessage::system("System prompt"),
            ChatMessage::user("Question"),
            ChatMessage::system("Nudge: wrap up"),
            ChatMessage::assistant_with_tool_calls(
                None,
                vec![ToolCall {
                    id: "c1".to_string(),
                    name: "echo".to_string(),
                    arguments: serde_json::json!({}),
                }],
            ),
            ChatMessage::tool_result("c1", "echo", "done"),
        ];

        let compacted = compact_messages_for_retry(&messages);

        // system prompt + note + user + nudge + assistant + tool_result = 6
        assert_eq!(compacted.len(), 6);
        assert_eq!(compacted[0].content, "System prompt");
        assert!(compacted[1].content.contains("compacted"));
        assert_eq!(compacted[2].content, "Question");
        assert_eq!(compacted[3].content, "Nudge: wrap up"); // not duplicated
        assert_eq!(compacted[4].role, Role::Assistant);
        assert_eq!(compacted[5].role, Role::Tool);

        // Verify "Nudge: wrap up" appears exactly once
        let nudge_count = compacted
            .iter()
            .filter(|m| m.content == "Nudge: wrap up")
            .count();
        assert_eq!(nudge_count, 1);
    }
}
