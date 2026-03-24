//! LLM reasoning capabilities for planning, tool selection, and evaluation.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::LlmError;

use crate::llm::cost_tracker::{CostEntry, CostTracker};
use crate::llm::{
    ChatMessage, CompletionRequest, LlmProvider, ToolCall, ToolCompletionRequest, ToolDefinition,
};
use crate::safety::SafetyLayer;

// Response cleaning and tag stripping
pub use super::reasoning_tags::{
    SuccessEvaluation, clean_response, extract_json, recover_tool_calls_from_content,
};

/// Token the agent returns when it has nothing to say (e.g. in group chats).
/// The dispatcher should check for this and suppress the message.
pub const SILENT_REPLY_TOKEN: &str = "NO_REPLY";

/// Check if a response is a silent reply (the agent has nothing to say).
///
/// Returns true if the trimmed text is exactly the silent reply token or
/// contains only the token surrounded by whitespace/punctuation.
pub fn is_silent_reply(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed == SILENT_REPLY_TOKEN
        || trimmed.starts_with(SILENT_REPLY_TOKEN)
            && trimmed.len() <= SILENT_REPLY_TOKEN.len() + 4
            && trimmed[SILENT_REPLY_TOKEN.len()..]
                .chars()
                .all(|c| c.is_whitespace() || c.is_ascii_punctuation())
}
/// Context for reasoning operations.
pub struct ReasoningContext {
    /// Conversation history.
    pub messages: Vec<ChatMessage>,
    /// Available tools.
    pub available_tools: Vec<ToolDefinition>,
    /// Job description if working on a job.
    pub job_description: Option<String>,
    /// Current state description.
    pub current_state: Option<String>,
    /// Opaque metadata forwarded to the LLM provider (e.g. thread_id for chaining).
    pub metadata: std::collections::HashMap<String, String>,
    /// When true, force a text-only response (ignore available tools).
    /// Used by the agentic loop to guarantee termination near the iteration limit.
    pub force_text: bool,
    /// Extended thinking configuration. When enabled, compatible providers
    /// will return their chain-of-thought reasoning alongside the response.
    pub thinking: crate::llm::ThinkingConfig,
}

impl ReasoningContext {
    /// Create a new reasoning context.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            available_tools: Vec::new(),
            job_description: None,
            current_state: None,
            metadata: std::collections::HashMap::new(),
            force_text: false,
            thinking: Default::default(),
        }
    }

    /// Add a message to the context.
    pub fn with_message(mut self, message: ChatMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Set messages directly (for session-based context).
    pub fn with_messages(mut self, messages: Vec<ChatMessage>) -> Self {
        self.messages = messages;
        self
    }

    /// Set available tools.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.available_tools = tools;
        self
    }

    /// Set job description.
    pub fn with_job(mut self, description: impl Into<String>) -> Self {
        self.job_description = Some(description.into());
        self
    }

    /// Set metadata (forwarded to the LLM provider).
    pub fn with_metadata(mut self, metadata: std::collections::HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }
}

impl Default for ReasoningContext {
    fn default() -> Self {
        Self::new()
    }
}

/// A planned action to take.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    /// Tool to use.
    pub tool_name: String,
    /// Parameters for the tool.
    pub parameters: serde_json::Value,
    /// Reasoning for this action.
    pub reasoning: String,
    /// Expected outcome.
    pub expected_outcome: String,
}

/// Result of planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    /// Overall goal understanding.
    pub goal: String,
    /// Planned sequence of actions.
    pub actions: Vec<PlannedAction>,
    /// Estimated total cost.
    pub estimated_cost: Option<f64>,
    /// Estimated total time in seconds.
    pub estimated_time_secs: Option<u64>,
    /// Confidence in the plan (0-1).
    pub confidence: f64,
}

/// Result of tool selection.
#[derive(Debug, Clone)]
pub struct ToolSelection {
    /// Selected tool name.
    pub tool_name: String,
    /// Parameters for the tool.
    pub parameters: serde_json::Value,
    /// Reasoning for the selection.
    pub reasoning: String,
    /// Alternative tools considered.
    pub alternatives: Vec<String>,
    /// The tool call ID from the LLM response.
    ///
    /// OpenAI-compatible providers assign each tool call a unique ID that must
    /// be echoed back in the corresponding tool result message. Without this,
    /// the provider cannot match results to their originating calls.
    pub tool_call_id: String,
}

/// Token usage from a single LLM call.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl TokenUsage {
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

/// Result of a response with potential tool calls.
///
/// Used by the agent loop to handle tool execution before returning a final response.
#[derive(Debug, Clone)]
pub enum RespondResult {
    /// A text response (no tools needed).
    Text(String),
    /// The model wants to call tools. Caller should execute them and call back.
    /// Includes the optional content from the assistant message (some models
    /// include explanatory text alongside tool calls).
    ToolCalls {
        tool_calls: Vec<ToolCall>,
        content: Option<String>,
    },
}

/// A `RespondResult` bundled with the token usage from the LLM call that produced it.
#[derive(Debug, Clone)]
pub struct RespondOutput {
    pub result: RespondResult,
    pub usage: TokenUsage,
    /// Extended thinking / chain-of-thought content from the LLM, if available.
    pub thinking_content: Option<String>,
}

/// Reasoning engine for the agent.
pub struct Reasoning {
    llm: Arc<dyn LlmProvider>,
    #[allow(dead_code)] // Will be used for sanitizing tool outputs
    safety: Arc<SafetyLayer>,
    /// Optional workspace for loading identity/system prompts.
    workspace_system_prompt: Option<String>,
    /// Optional skill context block to inject into system prompt.
    skill_context: Option<String>,
    /// Channel name (e.g. "discord", "telegram") for formatting hints.
    channel: Option<String>,
    /// Model name for runtime context.
    model_name: Option<String>,
    /// Whether this is a group chat context.
    is_group_chat: bool,
    /// Workspace mode: "unrestricted", "sandboxed", or "project".
    workspace_mode: Option<String>,
    /// Workspace root directory (for sandboxed/project modes).
    workspace_root: Option<String>,
    /// Shared cost tracker — records every LLM call for the Cost Dashboard.
    cost_tracker: Option<Arc<tokio::sync::Mutex<CostTracker>>>,
    /// Shared response cache — records hits/misses for the Cache Dashboard.
    response_cache:
        Option<Arc<tokio::sync::RwLock<crate::llm::response_cache_ext::CachedResponseStore>>>,
}

impl Reasoning {
    /// Create a new reasoning engine.
    pub fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self {
            llm,
            safety,
            workspace_system_prompt: None,
            skill_context: None,
            channel: None,
            model_name: None,
            is_group_chat: false,
            workspace_mode: None,
            workspace_root: None,
            cost_tracker: None,
            response_cache: None,
        }
    }

    /// Wire a shared cost tracker so every LLM call is recorded.
    ///
    /// The tracker is read by `tauri_commands::cost_summary()` / `cost_export_csv()`.
    pub fn with_cost_tracker(mut self, tracker: Arc<tokio::sync::Mutex<CostTracker>>) -> Self {
        self.cost_tracker = Some(tracker);
        self
    }

    /// Wire a shared response cache so every LLM call records hits/misses.
    ///
    /// The cache is read by `tauri_commands::cache_stats()`.
    pub fn with_response_cache(
        mut self,
        cache: Arc<tokio::sync::RwLock<crate::llm::response_cache_ext::CachedResponseStore>>,
    ) -> Self {
        self.response_cache = Some(cache);
        self
    }

    /// Set a custom system prompt from workspace identity files.
    ///
    /// This is typically loaded from workspace.system_prompt() which combines
    /// AGENTS.md, SOUL.md, USER.md, and IDENTITY.md into a unified prompt.
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        if !prompt.is_empty() {
            self.workspace_system_prompt = Some(prompt);
        }
        self
    }

    /// Set skill context to inject into the system prompt.
    ///
    /// The context block contains sanitized prompt content from active skills,
    /// wrapped in `<skill>` delimiters with trust metadata.
    pub fn with_skill_context(mut self, context: String) -> Self {
        if !context.is_empty() {
            self.skill_context = Some(context);
        }
        self
    }

    /// Set the channel name for channel-specific formatting hints.
    pub fn with_channel(mut self, channel: impl Into<String>) -> Self {
        let ch = channel.into();
        if !ch.is_empty() {
            self.channel = Some(ch);
        }
        self
    }

    /// Set the model name for runtime context.
    pub fn with_model_name(mut self, name: impl Into<String>) -> Self {
        let n = name.into();
        if !n.is_empty() {
            self.model_name = Some(n);
        }
        self
    }

    /// Set whether this is a group chat context.
    pub fn with_group_chat(mut self, is_group: bool) -> Self {
        self.is_group_chat = is_group;
        self
    }

    /// Set workspace mode and optional root directory.
    ///
    /// The mode determines what filesystem guidance is injected into the system prompt:
    /// - `"unrestricted"` — full filesystem access (Cursor-style coding assistant)
    /// - `"sandboxed"` — file tools confined to workspace_root
    /// - `"project"` — shell cwd defaults to workspace_root, file tools unrestricted
    pub fn with_workspace_mode(
        mut self,
        mode: impl Into<String>,
        root: Option<impl Into<String>>,
    ) -> Self {
        let m = mode.into();
        if !m.is_empty() {
            self.workspace_mode = Some(m);
        }
        if let Some(r) = root {
            let r = r.into();
            if !r.is_empty() {
                self.workspace_root = Some(r);
            }
        }
        self
    }

    /// Run a simple LLM completion with automatic response cleaning.
    ///
    /// This is the preferred entry point for code paths that call the LLM
    /// outside the agentic loop (e.g. `/summarize`, `/suggest`, heartbeat,
    /// compaction). It ensures `clean_response` is always applied so
    /// reasoning tags never leak to users or get stored in the workspace.
    pub async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<(String, TokenUsage), LlmError> {
        // Try cache first for non-tool completions
        let cache_key = Self::make_cache_key(&request.messages);
        if let Some(ref cache) = self.response_cache {
            // CachedResponseStore::get() takes &mut self (updates last_accessed for LRU),
            // so we need a write() guard rather than read(). Pre-existing bug fixed.
            let mut guard = cache.write().await;
            if guard.is_cacheable(false, false)
                && let Some(cached) = guard.get(&cache_key)
            {
                tracing::debug!("Response cache HIT");
                let usage = TokenUsage {
                    input_tokens: 0,
                    output_tokens: 0,
                };
                return Ok((cached, usage));
            }
        }

        let response = self.llm.complete(request).await?;
        let usage = TokenUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        };
        self.record_cost(&usage).await;

        let cleaned = clean_response(&response.content);

        // Store in cache
        if let Some(ref cache) = self.response_cache {
            let model = self.llm.active_model_name();
            cache.write().await.set(&cache_key, cleaned.clone(), &model);
        }

        Ok((cleaned, usage))
    }

    /// Build a simple cache key from the last 2 messages (role + content hash).
    ///
    /// Uses `DefaultHasher` (guaranteed to be SipHasher13 since Rust 1.71).
    /// The cache is process-local so cross-version stability is not required.
    fn make_cache_key(messages: &[ChatMessage]) -> String {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        // Use last 2 messages to keep the key short but distinctive
        for msg in messages.iter().rev().take(2) {
            msg.content.hash(&mut hasher);
            format!("{:?}", msg.role).hash(&mut hasher);
        }
        format!("llm:{:016x}", hasher.finish())
    }

    /// Record token usage + cost into the shared CostTracker (fire-and-forget).
    async fn record_cost(&self, usage: &TokenUsage) {
        let Some(ref tracker) = self.cost_tracker else {
            return;
        };
        let model = self.llm.active_model_name();
        let cost_usd = {
            let (input_rate, output_rate) = self.llm.cost_per_token();
            let input = rust_decimal::Decimal::from(usage.input_tokens);
            let output = rust_decimal::Decimal::from(usage.output_tokens);
            let total = input * input_rate + output * output_rate;
            use rust_decimal::prelude::ToPrimitive;
            total.to_f64().unwrap_or(0.0)
        };
        let agent_id = self
            .channel
            .clone()
            .unwrap_or_else(|| "default".to_string());
        tracing::debug!(
            model = %model,
            input_tokens = usage.input_tokens,
            output_tokens = usage.output_tokens,
            cost_usd = format!("{:.6}", cost_usd),
            "CostTracker: recorded LLM call"
        );
        let entry = CostEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            agent_id: Some(agent_id),
            provider: model.clone(),
            model,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cost_usd,
            request_id: None,
        };
        tracker.lock().await.record(entry);
    }

    /// Generate a plan for completing a goal.
    pub async fn plan(&self, context: &ReasoningContext) -> Result<ActionPlan, LlmError> {
        let system_prompt = self.build_planning_prompt(context);

        let mut messages = vec![ChatMessage::system(system_prompt)];
        messages.extend(context.messages.clone());

        if let Some(ref job) = context.job_description {
            messages.push(ChatMessage::user(format!(
                "Please create a plan to complete this job:\n\n{}",
                job
            )));
        }

        let request = CompletionRequest::new(messages)
            .with_max_tokens(2048)
            .with_temperature(0.3);

        let response = self.llm.complete(request).await?;

        // Record cost for planning LLM calls (feeds Cost Dashboard).
        let usage = TokenUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        };
        self.record_cost(&usage).await;

        // Parse the plan from the response
        self.parse_plan(&response.content)
    }

    /// Select the best tool for the current situation.
    pub async fn select_tool(
        &self,
        context: &ReasoningContext,
    ) -> Result<Option<ToolSelection>, LlmError> {
        let tools = self.select_tools(context).await?;
        Ok(tools.into_iter().next())
    }

    /// Select tools to execute (may return multiple for parallel execution).
    ///
    /// The LLM may return multiple tool calls if it determines they can be
    /// executed in parallel. This enables more efficient job completion.
    pub async fn select_tools(
        &self,
        context: &ReasoningContext,
    ) -> Result<Vec<ToolSelection>, LlmError> {
        if context.available_tools.is_empty() {
            return Ok(vec![]);
        }

        let mut request =
            ToolCompletionRequest::new(context.messages.clone(), context.available_tools.clone())
                .with_max_tokens(1024)
                .with_tool_choice("auto");
        request.metadata = context.metadata.clone();

        let response = self.llm.complete_with_tools(request).await?;

        // Record cost for this tool-selection call.
        let usage = TokenUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        };
        self.record_cost(&usage).await;

        let reasoning = response.content.unwrap_or_default();

        let selections: Vec<ToolSelection> = response
            .tool_calls
            .into_iter()
            .map(|tool_call| ToolSelection {
                tool_name: tool_call.name,
                parameters: tool_call.arguments,
                reasoning: reasoning.clone(),
                alternatives: vec![],
                tool_call_id: tool_call.id,
            })
            .collect();

        Ok(selections)
    }

    /// Evaluate whether a task was completed successfully.
    pub async fn evaluate_success(
        &self,
        context: &ReasoningContext,
        result: &str,
    ) -> Result<SuccessEvaluation, LlmError> {
        let system_prompt = r#"You are an evaluation assistant. Your job is to determine if a task was completed successfully.

Analyze the task description and the result, then provide:
1. Whether the task was successful (true/false)
2. A confidence score (0-1)
3. Detailed reasoning
4. Any issues found
5. Suggestions for improvement

Respond in JSON format:
{
    "success": true/false,
    "confidence": 0.0-1.0,
    "reasoning": "...",
    "issues": ["..."],
    "suggestions": ["..."]
}"#;

        let mut messages = vec![ChatMessage::system(system_prompt)];

        if let Some(ref job) = context.job_description {
            messages.push(ChatMessage::user(format!(
                "Task description:\n{}\n\nResult:\n{}",
                job, result
            )));
        } else {
            messages.push(ChatMessage::user(format!(
                "Result to evaluate:\n{}",
                result
            )));
        }

        let request = CompletionRequest::new(messages)
            .with_max_tokens(1024)
            .with_temperature(0.1);

        let response = self.llm.complete(request).await?;

        self.parse_evaluation(&response.content)
    }

    /// Generate a response to a user message.
    ///
    /// If tools are available in the context, uses tool completion mode.
    /// This is a convenience wrapper around `respond_with_tools()` that formats
    /// tool calls as text for simple cases. Use `respond_with_tools()` when you
    /// need to actually execute tool calls in an agentic loop.
    pub async fn respond(&self, context: &ReasoningContext) -> Result<String, LlmError> {
        let output = self.respond_with_tools(context).await?;
        match output.result {
            RespondResult::Text(text) => Ok(text),
            RespondResult::ToolCalls {
                tool_calls: calls, ..
            } => {
                // Format tool calls as text (legacy behavior for non-agentic callers)
                let tool_info: Vec<String> = calls
                    .iter()
                    .map(|tc| format!("`{}({})`", tc.name, tc.arguments))
                    .collect();
                Ok(format!("[Calling tools: {}]", tool_info.join(", ")))
            }
        }
    }

    /// Generate a response that may include tool calls, with token usage tracking.
    ///
    /// Returns `RespondOutput` containing the result and token usage from the LLM call.
    /// The caller should use `usage` to track cost/budget against the job.
    pub async fn respond_with_tools(
        &self,
        context: &ReasoningContext,
    ) -> Result<RespondOutput, LlmError> {
        let system_prompt = self.build_conversation_prompt(context);

        let mut messages = vec![ChatMessage::system(system_prompt)];
        messages.extend(context.messages.clone());

        // ── Pre-prompt context diagnostics ────────────────────────────
        // Log the context size before sending to the LLM. Helps debug
        // token limit errors and optimize prompt engineering.
        {
            let msg_count = messages.len();
            let char_count: usize = messages.iter().map(|m| m.estimated_chars()).sum();
            let tool_count = context.available_tools.len();
            let tool_def_chars: usize = context
                .available_tools
                .iter()
                .map(|t| t.name.len() + t.description.len() + 100) // ~100 chars overhead per tool schema
                .sum();
            tracing::debug!(
                messages = msg_count,
                est_prompt_chars = char_count + tool_def_chars,
                tools = tool_count,
                force_text = context.force_text,
                "Pre-prompt context diagnostics"
            );
        }

        let effective_tools = if context.force_text {
            Vec::new()
        } else {
            context.available_tools.clone()
        };

        // If we have tools, use tool completion mode
        if !effective_tools.is_empty() {
            let mut request = ToolCompletionRequest::new(messages, effective_tools)
                .with_max_tokens(4096)
                .with_temperature(0.7)
                .with_tool_choice("auto")
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();

            let response = self.llm.complete_with_tools(request).await?;
            let thinking = response.thinking_content.clone();
            let usage = TokenUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            };

            // Record cost for EVERY tool-completion LLM call (feeds Cost Dashboard).
            self.record_cost(&usage).await;

            // If there were tool calls, return them for execution
            if !response.tool_calls.is_empty() {
                return Ok(RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls: response.tool_calls,
                        content: response.content.map(|c| clean_response(&c)),
                    },
                    usage,
                    thinking_content: thinking,
                });
            }

            let content = response
                .content
                .unwrap_or_else(|| "I'm not sure how to respond to that.".to_string());

            // Some models (e.g. GLM-4.7) emit tool calls as XML tags in content
            // instead of using the structured tool_calls field. Try to recover
            // them before giving up and returning plain text.
            let recovered = recover_tool_calls_from_content(&content, &context.available_tools);
            if !recovered.is_empty() {
                let cleaned = clean_response(&content);
                return Ok(RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls: recovered,
                        content: if cleaned.is_empty() {
                            None
                        } else {
                            Some(cleaned)
                        },
                    },
                    usage,
                    thinking_content: thinking,
                });
            }

            // Guard against empty text after cleaning. This can happen
            // when reasoning models (e.g. GLM-5) return chain-of-thought
            // in reasoning_content wrapped in <think> tags and content is
            // null — the .or(reasoning_content) fallback picks it up, then
            // clean_response strips the think tags leaving an empty string.
            let cleaned = clean_response(&content);
            let final_text = if cleaned.trim().is_empty() {
                tracing::warn!(
                    "LLM response was empty after cleaning (original len={}), using fallback",
                    content.len()
                );
                "I'm not sure how to respond to that.".to_string()
            } else {
                cleaned
            };
            Ok(RespondOutput {
                result: RespondResult::Text(final_text),
                usage,
                thinking_content: thinking,
            })
        } else {
            // No tools, use simple completion
            let mut request = CompletionRequest::new(messages)
                .with_max_tokens(4096)
                .with_temperature(0.7)
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();

            let response = self.llm.complete(request).await?;
            let usage = TokenUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            };

            // Record cost for text-only completion LLM call (feeds Cost Dashboard).
            self.record_cost(&usage).await;

            let cleaned = clean_response(&response.content);
            let final_text = if cleaned.trim().is_empty() {
                tracing::warn!(
                    "LLM response was empty after cleaning (original len={}), using fallback",
                    response.content.len()
                );
                "I'm not sure how to respond to that.".to_string()
            } else {
                cleaned
            };
            Ok(RespondOutput {
                result: RespondResult::Text(final_text),
                usage,
                thinking_content: response.thinking_content,
            })
        }
    }

    /// Generate a response that may include tool calls, streaming text chunks via a callback.
    ///
    /// Behaves identically to `respond_with_tools()` but uses the LLM's streaming
    /// API. As text chunks arrive, `on_chunk` is called with each delta. The full
    /// accumulated text is returned in the `RespondOutput` for session recording.
    ///
    /// If the LLM returns tool calls instead of text, no streaming occurs and the
    /// result is returned directly (tool call responses go back to the LLM, not the user).
    pub async fn respond_with_tools_streaming<F>(
        &self,
        context: &ReasoningContext,
        mut on_chunk: F,
    ) -> Result<RespondOutput, LlmError>
    where
        F: FnMut(&str) + Send,
    {
        use futures::StreamExt;

        let system_prompt = self.build_conversation_prompt(context);

        let mut messages = vec![ChatMessage::system(system_prompt)];
        messages.extend(context.messages.clone());

        // Pre-prompt context diagnostics (same as non-streaming)
        {
            let msg_count = messages.len();
            let char_count: usize = messages.iter().map(|m| m.estimated_chars()).sum();
            let tool_count = context.available_tools.len();
            let tool_def_chars: usize = context
                .available_tools
                .iter()
                .map(|t| t.name.len() + t.description.len() + 100)
                .sum();
            tracing::debug!(
                messages = msg_count,
                est_prompt_chars = char_count + tool_def_chars,
                tools = tool_count,
                force_text = context.force_text,
                streaming = true,
                "Pre-prompt context diagnostics (streaming)"
            );
        }

        let effective_tools = if context.force_text {
            Vec::new()
        } else {
            context.available_tools.clone()
        };

        // Use streaming completion
        let mut stream = if !effective_tools.is_empty() {
            let mut request = ToolCompletionRequest::new(messages, effective_tools)
                .with_max_tokens(4096)
                .with_temperature(0.7)
                .with_tool_choice("auto")
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();
            self.llm.complete_stream_with_tools(request).await?
        } else {
            let mut request = CompletionRequest::new(messages)
                .with_max_tokens(4096)
                .with_temperature(0.7)
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();
            self.llm.complete_stream(request).await?
        };

        // Consume the stream, accumulating text and forwarding chunks
        let mut accumulated_text = String::new();
        let mut thinking_content: Option<String> = None;
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut final_usage = TokenUsage::default();
        let mut _finish_reason = crate::llm::FinishReason::Stop;

        // Accumulator for tool call deltas (index -> partial ToolCall)
        let mut partial_tool_calls: std::collections::HashMap<u32, (String, String, String)> =
            std::collections::HashMap::new();

        while let Some(chunk_result) = stream.next().await {
            match chunk_result? {
                crate::llm::StreamChunk::Text(text) => {
                    accumulated_text.push_str(&text);
                    on_chunk(&text);
                }
                crate::llm::StreamChunk::ReasoningDelta(delta) => {
                    thinking_content
                        .get_or_insert_with(String::new)
                        .push_str(&delta);
                }
                crate::llm::StreamChunk::ToolCall(tc) => {
                    tool_calls.push(tc);
                }
                crate::llm::StreamChunk::ToolCallDelta {
                    index,
                    id,
                    name,
                    arguments_delta,
                } => {
                    let entry = partial_tool_calls
                        .entry(index)
                        .or_insert_with(|| (String::new(), String::new(), String::new()));
                    if !id.is_empty() {
                        entry.0 = id;
                    }
                    if let Some(n) = name {
                        entry.1.push_str(&n);
                    }
                    if let Some(args) = arguments_delta {
                        entry.2.push_str(&args);
                    }
                }
                crate::llm::StreamChunk::Done {
                    input_tokens,
                    output_tokens,
                    finish_reason: fr,
                } => {
                    final_usage = TokenUsage {
                        input_tokens,
                        output_tokens,
                    };
                    _finish_reason = fr;

                    // Record cost when stream completes (feeds Cost Dashboard).
                    self.record_cost(&final_usage).await;
                }
            }
        }

        // Convert partial tool call deltas to complete ToolCalls
        for (_idx, (id, name, args)) in partial_tool_calls {
            if !name.is_empty() {
                let arguments: serde_json::Value =
                    serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
        }

        // If tool calls were returned, return them (no streaming to user)
        if !tool_calls.is_empty() {
            return Ok(RespondOutput {
                result: RespondResult::ToolCalls {
                    tool_calls,
                    content: if accumulated_text.is_empty() {
                        None
                    } else {
                        Some(clean_response(&accumulated_text))
                    },
                },
                usage: final_usage,
                thinking_content,
            });
        }

        // Clean the accumulated text
        let cleaned = clean_response(&accumulated_text);

        // Try to recover tool calls from XML-style content (same as non-streaming)
        if !context.force_text {
            let recovered = recover_tool_calls_from_content(&cleaned, &context.available_tools);
            if !recovered.is_empty() {
                return Ok(RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls: recovered,
                        content: if cleaned.is_empty() {
                            None
                        } else {
                            Some(cleaned)
                        },
                    },
                    usage: final_usage,
                    thinking_content,
                });
            }
        }

        let final_text = if cleaned.trim().is_empty() {
            tracing::warn!(
                "Streaming LLM response was empty after cleaning (original len={}), using fallback",
                accumulated_text.len()
            );
            "I'm not sure how to respond to that.".to_string()
        } else {
            cleaned
        };

        Ok(RespondOutput {
            result: RespondResult::Text(final_text),
            usage: final_usage,
            thinking_content,
        })
    }

    fn build_planning_prompt(&self, context: &ReasoningContext) -> String {
        let tools_desc = if context.available_tools.is_empty() {
            "No tools available.".to_string()
        } else {
            context
                .available_tools
                .iter()
                .map(|t| {
                    // Include the full parameter schema so the LLM can fill
                    // in required fields. Without this, every tool call in the
                    // plan ends up with empty `{}` parameters and fails.
                    let params =
                        serde_json::to_string(&t.parameters).unwrap_or_else(|_| "{}".to_string());
                    format!("### {}\n{}\nParameters: {}", t.name, t.description, params)
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        format!(
            r#"You are a planning assistant for an autonomous agent. Your job is to create detailed, actionable plans.

Available tools:
{tools_desc}

CRITICAL RULES:
- You MUST fill in the "parameters" object with ALL required fields from each tool's parameter schema.
- Do NOT leave "parameters" as an empty object {{}}.
- Only use tools whose required parameters you can provide.
- Do NOT use tools that require authentication or external credentials you don't have.

Respond with a JSON plan in this format:
{{
    "goal": "Clear statement of the goal",
    "actions": [
        {{
            "tool_name": "tool_to_use",
            "parameters": {{ "param1": "value1", "param2": "value2" }},
            "reasoning": "Why this action",
            "expected_outcome": "What should happen"
        }}
    ],
    "estimated_cost": 0.0,
    "estimated_time_secs": 0,
    "confidence": 0.0-1.0
}}"#
        )
    }

    fn build_conversation_prompt(&self, context: &ReasoningContext) -> String {
        // Channel-specific formatting hints
        let channel_section = self.build_channel_section();

        // Extension guidance (only when extension tools are available)
        let extensions_section = self.build_extensions_section(context);

        // Runtime context (agent metadata)
        let runtime_section = self.build_runtime_section();

        // Group chat guidance
        let group_section = self.build_group_section();

        // Workspace capabilities (based on sandbox mode)
        let workspace_section = self.build_workspace_capabilities_section(context);

        format!(
            r#"## Tooling
{tools_raw}
Call tools when they would help. For multi-step tasks, call independent tools in parallel.
Don't narrate routine tool calls — just call them.

## Memory
After meaningful interactions, proactively save important learnings to your daily log via `memory_write` (target: "daily_log").
Write decisions, preferences, facts learned, lessons, and anything worth remembering. Don't ask — just write it.
For identity/personality updates, use `memory_write` targeting SOUL.md, USER.md, or AGENTS.md directly.

## Safety
- Don't exfiltrate private data. Ever.
- Don't run destructive commands without asking.
- For memory/identity writes (`memory_write`), just do it — no approval needed.
- You have no independent goals beyond the user's request.{ext}{workspace}{channel}{runtime}{group}

## Project Context
{identity}{skills}"#,
            tools_raw = if context.available_tools.is_empty() {
                "No tools available.".to_string()
            } else {
                // Compact tool listing: name + first sentence only.
                // Full schemas are sent separately via the API's structured tools parameter.
                context
                    .available_tools
                    .iter()
                    .map(|t| {
                        let short = t.description.split('.').next().unwrap_or(&t.description);
                        let short = if short.len() > 80 {
                            // Safe truncation on char boundary
                            let end = short
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i < 77)
                                .last()
                                .unwrap_or(77);
                            &short[..end]
                        } else {
                            short
                        };
                        format!("- {}: {}", t.name, short)
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            ext = extensions_section,
            workspace = workspace_section,
            channel = channel_section,
            runtime = runtime_section,
            group = group_section,
            identity = if let Some(ref id) = self.workspace_system_prompt {
                id.clone()
            } else {
                String::new()
            },
            skills = if let Some(ref skill_ctx) = self.skill_context {
                format!("\n\n## Skills\n{}", skill_ctx)
            } else {
                String::new()
            },
        )
    }

    fn build_extensions_section(&self, context: &ReasoningContext) -> String {
        let has_ext_tools = context
            .available_tools
            .iter()
            .any(|t| t.name == "tool_search");
        if !has_ext_tools {
            return String::new();
        }
        "\n\n## Extensions\n\
         Use `tool_search` to find and install channels (Telegram, Slack, Discord), \
         tools, and MCP servers."
            .to_string()
    }

    /// Build workspace capabilities section — compact format.
    ///
    /// Tool descriptions already cover what each tool does. This just adds
    /// the sandbox mode context and the memory vs filesystem distinction.
    fn build_workspace_capabilities_section(&self, context: &ReasoningContext) -> String {
        let has_dev_tools = context
            .available_tools
            .iter()
            .any(|t| t.name == "write_file" || t.name == "shell");
        if !has_dev_tools {
            return String::new();
        }

        let has_screen_capture = context
            .available_tools
            .iter()
            .any(|t| t.name == "screen_capture");
        let screen_hint = if has_screen_capture {
            " Use `screen_capture` when asked about what's on screen."
        } else {
            ""
        };

        let mode = self.workspace_mode.as_deref().unwrap_or("unrestricted");
        let root = self.workspace_root.as_deref().unwrap_or("~/");

        match mode {
            "sandboxed" => format!(
                "\n\n## Workspace\nFilesystem sandboxed to `{root}`. Create files directly — never tell the user to do it manually.\n\
                 Agent memory (SOUL/MEMORY/daily) → `memory_write` | User files → `write_file`{screen_hint}"
            ),
            "project" => format!(
                "\n\n## Workspace\nProject root: `{root}`. Full filesystem access via tools. Create files directly.\n\
                 Agent memory → `memory_write` | User files → `write_file`{screen_hint}"
            ),
            _ => format!(
                "\n\n## Workspace\nFull filesystem access on user's device. Create files directly — never tell the user to do it manually.\n\
                 Agent memory (SOUL/MEMORY/daily) → `memory_write` | User files → `write_file`{screen_hint}"
            ),
        }
    }

    fn build_channel_section(&self) -> String {
        let channel = match self.channel.as_deref() {
            Some(c) => c,
            None => return String::new(),
        };
        let hints = match channel {
            "discord" => {
                "\
- No markdown tables (Discord renders them as plaintext). Use bullet lists instead.\n\
- Wrap multiple URLs in `<>` to suppress embeds: `<https://example.com>`."
            }
            "whatsapp" => {
                "\
- No markdown headers or tables (WhatsApp ignores them). Use **bold** for emphasis.\n\
- Keep messages concise; long replies get truncated on mobile."
            }
            "telegram" => {
                "\
- No markdown tables (Telegram strips them). Bullet lists and bold work well."
            }
            "slack" => {
                "\
- No markdown tables. Use Slack formatting: *bold*, _italic_, `code`.\n\
- Prefer threaded replies when responding to older messages."
            }
            _ => return String::new(),
        };
        format!("\n\n## Channel Formatting ({})\n{}", channel, hints)
    }

    fn build_runtime_section(&self) -> String {
        let mut parts = Vec::new();
        // Always note the execution context so the agent knows it's on-device
        parts.push("host=device".to_string());
        if let Some(ref ch) = self.channel {
            parts.push(format!("channel={}", ch));
        }
        if let Some(ref model) = self.model_name {
            parts.push(format!("model={}", model));
        }
        format!("\n\n## Runtime\n{}", parts.join(" | "))
    }

    fn build_group_section(&self) -> String {
        if !self.is_group_chat {
            return String::new();
        }
        format!(
            "\n\n## Group Chat\n\
             You are in a group chat. Be selective about when to contribute.\n\
             Respond when: directly addressed, can add genuine value, or correcting misinformation.\n\
             Stay silent when: casual banter, question already answered, nothing to add.\n\
             React with emoji when available instead of cluttering with messages.\n\
             You are a participant, not the user's proxy. Do not share their private context.\n\
             When you have nothing to say, respond with ONLY: {}\n\
             It must be your ENTIRE message. Never append it to an actual response.",
            SILENT_REPLY_TOKEN,
        )
    }

    fn parse_plan(&self, content: &str) -> Result<ActionPlan, LlmError> {
        // Try to extract JSON from the response
        let json_str = extract_json(content).unwrap_or(content);

        serde_json::from_str(json_str).map_err(|e| LlmError::InvalidResponse {
            provider: self.llm.model_name().to_string(),
            reason: format!("Failed to parse plan: {}", e),
        })
    }

    fn parse_evaluation(&self, content: &str) -> Result<SuccessEvaluation, LlmError> {
        let json_str = extract_json(content).unwrap_or(content);

        serde_json::from_str(json_str).map_err(|e| LlmError::InvalidResponse {
            provider: self.llm.model_name().to_string(),
            reason: format!("Failed to parse evaluation: {}", e),
        })
    }
}
