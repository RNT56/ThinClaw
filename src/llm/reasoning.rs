//! LLM reasoning capabilities for planning, tool selection, and evaluation.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::LlmError;

use crate::llm::cost_tracker::{CostEntry, CostTracker};
use crate::llm::model_guidance;
use crate::llm::prompt_stack::PromptStack;
use crate::llm::usage_tracking::mark_reasoning_request;
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
    /// Ephemeral context fragments routed alongside the conversation instead of
    /// being merged into the stable system preamble.
    pub context_documents: Vec<String>,
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
    /// Optional output cap for this reasoning turn.
    pub max_output_tokens: Option<u32>,
}

impl ReasoningContext {
    /// Create a new reasoning context.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            context_documents: Vec::new(),
            available_tools: Vec::new(),
            job_description: None,
            current_state: None,
            metadata: std::collections::HashMap::new(),
            force_text: false,
            thinking: Default::default(),
            max_output_tokens: None,
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

    /// Set ephemeral context documents to pass alongside the conversation.
    pub fn with_context_documents(mut self, documents: Vec<String>) -> Self {
        self.context_documents = documents
            .into_iter()
            .filter(|doc| !doc.trim().is_empty())
            .collect();
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

    /// Set a per-turn output cap.
    pub fn with_max_output_tokens(mut self, max_output_tokens: u32) -> Self {
        self.max_output_tokens = Some(max_output_tokens);
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
    pub routed_model_name: Option<String>,
    pub finish_reason: crate::llm::FinishReason,
    /// Extended thinking / chain-of-thought content from the LLM, if available.
    pub thinking_content: Option<String>,
}

fn merge_streamed_tool_calls(
    mut tool_calls: Vec<ToolCall>,
    partial_tool_calls: std::collections::HashMap<u32, (String, String, String)>,
) -> Vec<ToolCall> {
    for (_idx, (id, name, args)) in partial_tool_calls {
        if name.is_empty() {
            continue;
        }

        let arguments: serde_json::Value =
            serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);

        let safe_id = if id.trim().is_empty() {
            format!("call_{}", uuid::Uuid::new_v4().simple())
        } else {
            id
        };

        if let Some(existing) = tool_calls.iter_mut().find(|tc| tc.id == safe_id) {
            if existing.name.is_empty() {
                existing.name = name;
            }
            if existing.arguments.is_null() && !arguments.is_null() {
                existing.arguments = arguments;
            }
            continue;
        }

        tool_calls.push(ToolCall {
            id: safe_id,
            name,
            arguments,
        });
    }

    let mut seen_ids = std::collections::HashSet::new();
    tool_calls.retain(|tc| seen_ids.insert(tc.id.clone()));
    tool_calls
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthoritativeIntent {
    CurrentTime,
    TranscriptHistory,
    MemoryRecall,
    LocalState,
}

impl AuthoritativeIntent {
    fn label(self) -> &'static str {
        match self {
            Self::CurrentTime => "current time/date",
            Self::TranscriptHistory => "conversation history",
            Self::MemoryRecall => "remembered context",
            Self::LocalState => "local/device state",
        }
    }

    fn preferred_tools(self) -> &'static [&'static str] {
        match self {
            Self::CurrentTime => &["time"],
            Self::TranscriptHistory => &["session_search"],
            Self::MemoryRecall => &["memory_search", "memory_read", "external_memory_recall"],
            Self::LocalState => &["device_info", "homeassistant"],
        }
    }
}

#[derive(Debug, Clone)]
struct ToolRoutingDecision {
    available_tools: Vec<ToolDefinition>,
    tool_choice: &'static str,
    unavailable_instruction: Option<String>,
}

fn last_user_message(messages: &[ChatMessage]) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|message| matches!(message.role, crate::llm::Role::User))
        .map(|message| message.content.as_str())
}

fn detect_authoritative_intent(messages: &[ChatMessage]) -> Option<AuthoritativeIntent> {
    let text = last_user_message(messages)?.to_ascii_lowercase();

    let current_time = [
        "what time",
        "current time",
        "what date",
        "current date",
        "what day is it",
        "today's date",
        "what day is today",
        "what date is today",
        "what day is tomorrow",
        "what date is tomorrow",
        "what day was yesterday",
        "what date was yesterday",
        "right now",
        "local time",
    ];
    if current_time.iter().any(|needle| text.contains(needle)) {
        return Some(AuthoritativeIntent::CurrentTime);
    }

    let transcript_history = [
        "earlier in this conversation",
        "earlier in the conversation",
        "earlier in this chat",
        "conversation history",
        "chat history",
        "what did i say",
        "what did we say",
        "previous message",
        "scroll back",
        "session history",
    ];
    if transcript_history
        .iter()
        .any(|needle| text.contains(needle))
    {
        return Some(AuthoritativeIntent::TranscriptHistory);
    }

    let memory_recall = [
        "what do you remember",
        "what do you know about me",
        "from memory",
        "did we decide",
        "what did we decide",
        "my preference",
        "my preferences",
        "remembered",
    ];
    if memory_recall.iter().any(|needle| text.contains(needle)) {
        return Some(AuthoritativeIntent::MemoryRecall);
    }

    let local_state = [
        "disk space",
        "device info",
        "disk usage",
        "memory usage",
        "cpu usage",
        "system uptime",
        "lights on",
        "thermostat",
        "temperature at home",
        "home assistant",
    ];
    if local_state.iter().any(|needle| text.contains(needle)) {
        return Some(AuthoritativeIntent::LocalState);
    }

    None
}

fn authoritative_unavailable_instruction(intent: AuthoritativeIntent) -> String {
    format!(
        "The user is asking about {}. No authoritative tool for that intent is available in this turn. Do not guess or fabricate the answer; explain that the required tool is unavailable.",
        intent.label()
    )
}

fn schema_type_label(schema: &serde_json::Value) -> String {
    match schema.get("type") {
        Some(serde_json::Value::String(value)) => value.clone(),
        Some(serde_json::Value::Array(values)) => values
            .iter()
            .filter_map(|value| value.as_str())
            .collect::<Vec<_>>()
            .join("|"),
        _ => "any".to_string(),
    }
}

fn schema_required_set(schema: &serde_json::Value) -> std::collections::HashSet<String> {
    schema
        .get("required")
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn render_compact_schema_fields(schema: &serde_json::Value, depth: usize) -> Vec<String> {
    let Some(properties) = schema.get("properties").and_then(|value| value.as_object()) else {
        return Vec::new();
    };

    let required = schema_required_set(schema);
    let mut names = properties.keys().cloned().collect::<Vec<_>>();
    names.sort();

    names
        .into_iter()
        .filter_map(|name| {
            let property = properties.get(&name)?;
            let mut line = format!(
                "- {}{}: {}",
                name,
                if required.contains(&name) {
                    " (required)"
                } else {
                    ""
                },
                schema_type_label(property)
            );

            if let Some(enum_values) = property.get("enum").and_then(|value| value.as_array()) {
                let enum_preview = enum_values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .take(6)
                    .collect::<Vec<_>>();
                if !enum_preview.is_empty() {
                    line.push_str(&format!(" [{}]", enum_preview.join(", ")));
                }
            }

            if depth == 0 {
                if property.get("type").and_then(|value| value.as_str()) == Some("object") {
                    let nested = render_compact_schema_fields(property, depth + 1);
                    if !nested.is_empty() {
                        let nested_inline = nested
                            .into_iter()
                            .map(|value| value.trim_start_matches("- ").to_string())
                            .collect::<Vec<_>>()
                            .join("; ");
                        line.push_str(&format!(" {{ {} }}", nested_inline));
                    }
                } else if property.get("type").and_then(|value| value.as_str()) == Some("array")
                    && let Some(items) = property.get("items")
                {
                    let item_type = schema_type_label(items);
                    if item_type != "any" {
                        line.push_str(&format!(" of {}", item_type));
                    }
                    if items.get("type").and_then(|value| value.as_str()) == Some("object") {
                        let nested = render_compact_schema_fields(items, depth + 1);
                        if !nested.is_empty() {
                            let nested_inline = nested
                                .into_iter()
                                .map(|value| value.trim_start_matches("- ").to_string())
                                .collect::<Vec<_>>()
                                .join("; ");
                            line.push_str(&format!(" {{ {} }}", nested_inline));
                        }
                    }
                }
            }

            Some(line)
        })
        .collect()
}

fn compact_tool_card(tool: &ToolDefinition) -> String {
    let mut required = schema_required_set(&tool.parameters)
        .into_iter()
        .collect::<Vec<_>>();
    required.sort();
    let required_line = if required.is_empty() {
        "none".to_string()
    } else {
        required.join(", ")
    };
    let fields = render_compact_schema_fields(&tool.parameters, 0);
    let fields_text = if fields.is_empty() {
        "- none".to_string()
    } else {
        fields.join("\n")
    };

    format!(
        "### {}\n{}\nRequired fields: {}\nFields:\n{}",
        tool.name, tool.description, required_line, fields_text
    )
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
    /// Optional session-scoped personality overlay block.
    personality_overlay: Option<String>,
    /// Channel name (e.g. "discord", "telegram") for formatting hints.
    channel: Option<String>,
    /// Formatting guidance resolved from the active channel implementation.
    channel_formatting_hints: Option<String>,
    /// Model name for runtime context.
    model_name: Option<String>,
    /// Cheap model configured for lightweight tasks, if any.
    cheap_model_name: Option<String>,
    /// Whether model-family-specific guidance should be injected.
    model_guidance_enabled: bool,
    /// Whether this is a group chat context.
    is_group_chat: bool,
    /// Workspace mode: "unrestricted", "sandboxed", or "project".
    workspace_mode: Option<String>,
    /// Workspace root directory (for sandboxed/project modes).
    workspace_root: Option<String>,
    /// Names of all active channels (e.g. ["repl", "apple_mail", "telegram"]).
    /// Injected into the system prompt so the LLM knows what communication
    /// channels are connected and can use them instead of reaching for API tools.
    active_channels: Vec<String>,
    /// Shared cost tracker — records every LLM call for the Cost Dashboard.
    cost_tracker: Option<Arc<tokio::sync::Mutex<CostTracker>>>,
    /// Shared response cache — records hits/misses for the Cache Dashboard.
    response_cache:
        Option<Arc<tokio::sync::RwLock<crate::llm::response_cache_ext::CachedResponseStore>>>,
}

impl Reasoning {
    fn system_message(&self, content: impl Into<String>) -> ChatMessage {
        let mut message = ChatMessage::system(content);
        if self.llm.supports_prompt_caching() {
            message = message.with_provider_metadata(
                "anthropic",
                json!({"cache_control": {"type": "ephemeral"}}),
            );
        }
        message
    }

    fn mark_request_metadata(&self, metadata: &mut std::collections::HashMap<String, String>) {
        mark_reasoning_request(metadata, self.channel.as_deref());
    }

    /// Create a new reasoning engine.
    pub fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self {
            llm,
            safety,
            workspace_system_prompt: None,
            skill_context: None,
            personality_overlay: None,
            channel: None,
            channel_formatting_hints: None,
            model_name: None,
            cheap_model_name: None,
            model_guidance_enabled: true,
            is_group_chat: false,
            workspace_mode: None,
            workspace_root: None,
            active_channels: Vec::new(),
            cost_tracker: None,
            response_cache: None,
        }
    }

    /// Swap the underlying LLM provider at runtime.
    ///
    /// Used by the dispatcher when the agent calls `llm_select` to switch
    /// to a different model/provider mid-conversation. The swap takes effect
    /// on the next `respond_with_tools` call.
    pub fn swap_llm(&mut self, new_llm: Arc<dyn LlmProvider>) {
        self.model_name = Some(new_llm.active_model_name());
        self.llm = new_llm;
    }

    /// Get a clone of the current LLM provider handle.
    ///
    /// Used by the dispatcher to snapshot the original provider so it can be
    /// restored when an `llm_select(model="reset")` is issued.
    pub fn current_llm(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.llm)
    }

    /// Clone the current reasoning configuration onto a different LLM handle.
    pub fn fork_with_llm(&self, llm: Arc<dyn LlmProvider>) -> Self {
        let model_name = llm.active_model_name();
        Self {
            llm,
            safety: Arc::clone(&self.safety),
            workspace_system_prompt: self.workspace_system_prompt.clone(),
            skill_context: self.skill_context.clone(),
            personality_overlay: self.personality_overlay.clone(),
            channel: self.channel.clone(),
            channel_formatting_hints: self.channel_formatting_hints.clone(),
            model_name: Some(model_name),
            cheap_model_name: self.cheap_model_name.clone(),
            model_guidance_enabled: self.model_guidance_enabled,
            is_group_chat: self.is_group_chat,
            workspace_mode: self.workspace_mode.clone(),
            workspace_root: self.workspace_root.clone(),
            active_channels: self.active_channels.clone(),
            cost_tracker: self.cost_tracker.clone(),
            response_cache: self.response_cache.clone(),
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

    /// Set a temporary session personality overlay.
    pub fn with_personality_overlay(mut self, overlay: String) -> Self {
        if !overlay.is_empty() {
            self.personality_overlay = Some(overlay);
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

    /// Set formatting guidance resolved from the active channel implementation.
    pub fn with_channel_formatting_hints(mut self, hints: impl Into<String>) -> Self {
        let hints = hints.into();
        if !hints.is_empty() {
            self.channel_formatting_hints = Some(hints);
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

    pub fn with_cheap_model_name(mut self, name: Option<String>) -> Self {
        self.cheap_model_name = name.filter(|s| !s.is_empty());
        self
    }

    /// Enable or disable model-family-specific prompt guidance.
    pub fn with_model_guidance_enabled(mut self, enabled: bool) -> Self {
        self.model_guidance_enabled = enabled;
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

    /// Set the list of active channel names.
    ///
    /// These are injected into the system prompt so the LLM knows which
    /// communication channels are connected (e.g. apple_mail, telegram,
    /// imessage). This prevents the agent from reaching for API-based tools
    /// when a native channel already provides the same capability.
    pub fn with_active_channels(mut self, channels: Vec<String>) -> Self {
        self.active_channels = channels;
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
        let cache_key = Self::make_cache_key(&request.messages, &request.context_documents);
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
        self.record_cost(
            &usage,
            response.provider_model.as_deref(),
            response.cost_usd,
        )
        .await;

        let cleaned = clean_response(&response.content);

        // Store in cache
        if let Some(ref cache) = self.response_cache {
            let model = response
                .provider_model
                .clone()
                .unwrap_or_else(|| self.llm.active_model_name());
            cache.write().await.set(&cache_key, cleaned.clone(), &model);
        }

        Ok((cleaned, usage))
    }

    /// Build a simple cache key from the last 2 messages (role + content hash).
    ///
    /// Uses `DefaultHasher` (guaranteed to be SipHasher13 since Rust 1.71).
    /// The cache is process-local so cross-version stability is not required.
    fn make_cache_key(messages: &[ChatMessage], context_documents: &[String]) -> String {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        // Use last 2 messages to keep the key short but distinctive
        for msg in messages.iter().rev().take(2) {
            msg.content.hash(&mut hasher);
            format!("{:?}", msg.role).hash(&mut hasher);
        }
        for document in context_documents {
            document.hash(&mut hasher);
        }
        format!("llm:{:016x}", hasher.finish())
    }

    /// Record token usage + cost into the shared CostTracker (fire-and-forget).
    async fn record_cost(
        &self,
        usage: &TokenUsage,
        provider_model: Option<&str>,
        cost_usd: Option<f64>,
    ) {
        let Some(ref tracker) = self.cost_tracker else {
            return;
        };
        let model = provider_model
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_else(|| self.llm.active_model_name());
        let fallback_cost_usd = {
            let (input_rate, output_rate) = self.llm.cost_per_token();
            let input = rust_decimal::Decimal::from(usage.input_tokens);
            let output = rust_decimal::Decimal::from(usage.output_tokens);
            let total = input * input_rate + output * output_rate;
            use rust_decimal::prelude::ToPrimitive;
            total.to_f64().unwrap_or(0.0)
        };
        let cost_usd = cost_usd.unwrap_or(fallback_cost_usd);
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

    fn resolve_tool_routing_decision(&self, context: &ReasoningContext) -> ToolRoutingDecision {
        let intent = detect_authoritative_intent(&context.messages);

        if context.force_text {
            return ToolRoutingDecision {
                available_tools: Vec::new(),
                tool_choice: "none",
                unavailable_instruction: intent.map(authoritative_unavailable_instruction),
            };
        }

        if context.available_tools.is_empty() {
            return ToolRoutingDecision {
                available_tools: Vec::new(),
                tool_choice: "none",
                unavailable_instruction: intent.map(authoritative_unavailable_instruction),
            };
        }

        let Some(intent) = intent else {
            return ToolRoutingDecision {
                available_tools: context.available_tools.clone(),
                tool_choice: "auto",
                unavailable_instruction: None,
            };
        };

        let shortlisted = context
            .available_tools
            .iter()
            .filter(|tool| intent.preferred_tools().contains(&tool.name.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        if shortlisted.is_empty() {
            ToolRoutingDecision {
                available_tools: Vec::new(),
                tool_choice: "none",
                unavailable_instruction: Some(authoritative_unavailable_instruction(intent)),
            }
        } else {
            ToolRoutingDecision {
                available_tools: shortlisted,
                tool_choice: "required",
                unavailable_instruction: None,
            }
        }
    }

    fn tool_unavailable_note_message(&self, note: Option<&str>) -> Option<ChatMessage> {
        note.map(|note| self.system_message(note.to_string()))
    }

    /// Generate a plan for completing a goal.
    pub async fn plan(&self, context: &ReasoningContext) -> Result<ActionPlan, LlmError> {
        let system_prompt = self.build_planning_prompt(context);

        let mut messages = vec![self.system_message(system_prompt)];
        messages.extend(context.messages.clone());

        if let Some(ref job) = context.job_description {
            messages.push(ChatMessage::user(format!(
                "Please create a plan to complete this job:\n\n{}",
                job
            )));
        }

        let mut request = CompletionRequest::new(messages)
            .with_max_tokens(2048)
            .with_temperature(0.3);
        self.mark_request_metadata(&mut request.metadata);

        let response = self.llm.complete(request).await?;

        // Record cost for planning LLM calls (feeds Cost Dashboard).
        let usage = TokenUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        };
        self.record_cost(
            &usage,
            response.provider_model.as_deref(),
            response.cost_usd,
        )
        .await;

        // Parse the plan from the response
        self.parse_plan(&response.content)
    }

    /// Repair invalid parameters for a planned action using the full schema of the selected tool.
    pub async fn repair_plan_action(
        &self,
        context: &ReasoningContext,
        action: &PlannedAction,
        failure_reason: &str,
    ) -> Result<serde_json::Value, LlmError> {
        let tool = context
            .available_tools
            .iter()
            .find(|tool| tool.name == action.tool_name)
            .ok_or_else(|| LlmError::InvalidResponse {
                provider: self.llm.active_model_name(),
                reason: format!("Tool '{}' unavailable for repair", action.tool_name),
            })?;

        let schema =
            serde_json::to_string_pretty(&tool.parameters).unwrap_or_else(|_| "{}".to_string());
        let current =
            serde_json::to_string_pretty(&action.parameters).unwrap_or_else(|_| "{}".to_string());
        let repair_prompt = format!(
            "Repair the parameters for this planned action. Return ONLY a JSON object for the corrected parameters.\n\nTool: {}\nDescription: {}\nFailure: {}\nCurrent parameters:\n{}\n\nFull JSON Schema:\n{}",
            tool.name, tool.description, failure_reason, current, schema
        );

        let mut request = CompletionRequest::new(vec![
            self.system_message(
                "You repair tool-call parameter objects. Respond with a single JSON object and no prose.",
            ),
            ChatMessage::user(repair_prompt),
        ])
        .with_max_tokens(1024)
        .with_temperature(0.1);
        self.mark_request_metadata(&mut request.metadata);

        let response = self.llm.complete(request).await?;
        let usage = TokenUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        };
        self.record_cost(
            &usage,
            response.provider_model.as_deref(),
            response.cost_usd,
        )
        .await;

        let json_str = extract_json(&response.content).unwrap_or(&response.content);
        serde_json::from_str(json_str).map_err(|error| LlmError::InvalidResponse {
            provider: self.llm.active_model_name(),
            reason: format!("Failed to parse repaired parameters: {error}"),
        })
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

        let routing = self.resolve_tool_routing_decision(context);
        if routing.unavailable_instruction.is_some() {
            return Ok(vec![]);
        }

        if routing.available_tools.is_empty() {
            return Ok(vec![]);
        }

        let mut request =
            ToolCompletionRequest::new(context.messages.clone(), routing.available_tools)
                .with_context_documents(context.context_documents.clone())
                .with_max_tokens(1024)
                .with_tool_choice(routing.tool_choice);
        request.metadata = context.metadata.clone();
        self.mark_request_metadata(&mut request.metadata);

        let response = self.llm.complete_with_tools(request).await?;

        // Record cost for this tool-selection call.
        let usage = TokenUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        };
        self.record_cost(
            &usage,
            response.provider_model.as_deref(),
            response.cost_usd,
        )
        .await;

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

        let mut messages = vec![self.system_message(system_prompt)];

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

        let mut request = CompletionRequest::new(messages)
            .with_max_tokens(1024)
            .with_temperature(0.1);
        self.mark_request_metadata(&mut request.metadata);

        let response = self.llm.complete(request).await?;
        let usage = TokenUsage {
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
        };
        self.record_cost(
            &usage,
            response.provider_model.as_deref(),
            response.cost_usd,
        )
        .await;

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
        let routing = self.resolve_tool_routing_decision(context);

        let mut messages = vec![self.system_message(system_prompt)];
        if let Some(note) =
            self.tool_unavailable_note_message(routing.unavailable_instruction.as_deref())
        {
            messages.push(note);
        }
        messages.extend(context.messages.clone());

        // ── Pre-prompt context diagnostics ────────────────────────────
        // Log the context size before sending to the LLM. Helps debug
        // token limit errors and optimize prompt engineering.
        {
            let msg_count = messages.len();
            let char_count: usize = messages.iter().map(|m| m.estimated_chars()).sum();
            let tool_count = routing.available_tools.len();
            let tool_def_chars: usize = routing
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

        let effective_tools = routing.available_tools.clone();

        let max_output_tokens = context.max_output_tokens.unwrap_or(4096);

        // If we have tools, use tool completion mode
        if !effective_tools.is_empty() {
            let mut request = ToolCompletionRequest::new(messages, effective_tools)
                .with_context_documents(context.context_documents.clone())
                .with_max_tokens(max_output_tokens)
                .with_temperature(0.7)
                .with_tool_choice(routing.tool_choice)
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();
            self.mark_request_metadata(&mut request.metadata);

            let response = self.llm.complete_with_tools(request).await?;
            let thinking = response.thinking_content.clone();
            let usage = TokenUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            };

            // Record cost for EVERY tool-completion LLM call (feeds Cost Dashboard).
            self.record_cost(
                &usage,
                response.provider_model.as_deref(),
                response.cost_usd,
            )
            .await;

            // If there were tool calls, return them for execution
            if !response.tool_calls.is_empty() {
                return Ok(RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls: response.tool_calls,
                        content: response.content.map(|c| clean_response(&c)),
                    },
                    usage,
                    routed_model_name: response.provider_model.clone(),
                    finish_reason: response.finish_reason,
                    thinking_content: thinking,
                });
            }

            let content = response
                .content
                .unwrap_or_else(|| "I'm not sure how to respond to that.".to_string());

            // Some models (e.g. GLM-4.7) emit tool calls as XML tags in content
            // instead of using the structured tool_calls field. Try to recover
            // them before giving up and returning plain text.
            let recovered = recover_tool_calls_from_content(&content, &routing.available_tools);
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
                    routed_model_name: response.provider_model.clone(),
                    finish_reason: response.finish_reason,
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
                routed_model_name: response.provider_model.clone(),
                finish_reason: response.finish_reason,
                thinking_content: thinking,
            })
        } else {
            // No tools, use simple completion
            let mut request = CompletionRequest::new(messages)
                .with_context_documents(context.context_documents.clone())
                .with_max_tokens(max_output_tokens)
                .with_temperature(0.7)
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();
            self.mark_request_metadata(&mut request.metadata);

            let response = self.llm.complete(request).await?;
            let usage = TokenUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            };

            // Record cost for text-only completion LLM call (feeds Cost Dashboard).
            self.record_cost(
                &usage,
                response.provider_model.as_deref(),
                response.cost_usd,
            )
            .await;

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
                routed_model_name: response.provider_model.clone(),
                finish_reason: response.finish_reason,
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
        let routing = self.resolve_tool_routing_decision(context);

        let mut messages = vec![self.system_message(system_prompt)];
        if let Some(note) =
            self.tool_unavailable_note_message(routing.unavailable_instruction.as_deref())
        {
            messages.push(note);
        }
        messages.extend(context.messages.clone());

        // Pre-prompt context diagnostics (same as non-streaming)
        {
            let msg_count = messages.len();
            let char_count: usize = messages.iter().map(|m| m.estimated_chars()).sum();
            let tool_count = routing.available_tools.len();
            let tool_def_chars: usize = routing
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

        let effective_tools = routing.available_tools.clone();

        // Use streaming completion
        let max_output_tokens = context.max_output_tokens.unwrap_or(4096);

        let mut stream = if !effective_tools.is_empty() {
            let mut request = ToolCompletionRequest::new(messages, effective_tools)
                .with_context_documents(context.context_documents.clone())
                .with_max_tokens(max_output_tokens)
                .with_temperature(0.7)
                .with_tool_choice(routing.tool_choice)
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();
            self.mark_request_metadata(&mut request.metadata);
            self.llm.complete_stream_with_tools(request).await?
        } else {
            let mut request = CompletionRequest::new(messages)
                .with_context_documents(context.context_documents.clone())
                .with_max_tokens(max_output_tokens)
                .with_temperature(0.7)
                .set_thinking(context.thinking);
            request.metadata = context.metadata.clone();
            self.mark_request_metadata(&mut request.metadata);
            self.llm.complete_stream(request).await?
        };

        // Consume the stream, accumulating text and forwarding chunks
        let mut accumulated_text = String::new();
        let mut thinking_content: Option<String> = None;
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut final_usage = TokenUsage::default();
        let mut final_finish_reason = crate::llm::FinishReason::Stop;
        let mut final_provider_model: Option<String> = None;

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
                    provider_model,
                    cost_usd,
                    input_tokens,
                    output_tokens,
                    finish_reason: fr,
                } => {
                    final_usage = TokenUsage {
                        input_tokens,
                        output_tokens,
                    };
                    final_finish_reason = fr;

                    // Record cost when stream completes (feeds Cost Dashboard).
                    self.record_cost(&final_usage, provider_model.as_deref(), cost_usd)
                        .await;
                    final_provider_model = provider_model;
                }
            }
        }

        // Some providers emit both a complete ToolCall and ToolCallDelta events
        // for the same logical call. Merge instead of appending blindly so we
        // don't execute the same tool twice.
        tool_calls = merge_streamed_tool_calls(tool_calls, partial_tool_calls);

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
                routed_model_name: final_provider_model,
                finish_reason: final_finish_reason,
                thinking_content,
            });
        }

        // Clean the accumulated text
        let cleaned = clean_response(&accumulated_text);

        // Try to recover tool calls from XML-style content (same as non-streaming)
        if !context.force_text {
            let recovered = recover_tool_calls_from_content(&cleaned, &routing.available_tools);
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
                    routed_model_name: final_provider_model,
                    finish_reason: final_finish_reason,
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
            routed_model_name: final_provider_model,
            finish_reason: final_finish_reason,
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
                .map(compact_tool_card)
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

        // Transparency + delegation guidance based on available tools
        let execution_style_section = self.build_execution_style_section(context);

        // Runtime context (agent metadata)
        let runtime_section = self.build_runtime_section();

        // Group chat guidance
        let group_section = self.build_group_section();

        // Workspace capabilities (based on sandbox mode)
        let workspace_section = self.build_workspace_capabilities_section(context);

        // Model-family-specific guidance
        let model_guidance_section = self.build_model_guidance_section();

        let tools_raw = if context.available_tools.is_empty() {
            "No tools available.".to_string()
        } else {
            context
                .available_tools
                .iter()
                .map(|t| {
                    let short = t.description.split('.').next().unwrap_or(&t.description);
                    let short = if short.len() > 80 {
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
        };

        let identity = if let Some(ref id) = self.workspace_system_prompt {
            match &self.personality_overlay {
                Some(overlay) => format!("{id}\n\n---\n\n{overlay}"),
                None => id.clone(),
            }
        } else if let Some(ref overlay) = self.personality_overlay {
            overlay.clone()
        } else {
            String::new()
        };

        let skills = if let Some(ref skill_ctx) = self.skill_context {
            format!("## Skills\n{}", skill_ctx)
        } else {
            String::new()
        };

        let mut stack = PromptStack::new();
        stack.push_section(
            "Tooling",
            format!(
                "{tools_raw}\nCall tools when they would help. For multi-step tasks, call independent tools in parallel.\n{execution_style_section}"
            ),
        );
        stack.push_section(
            "Memory",
            "After meaningful interactions, proactively save important learnings to your daily log via `memory_write` (target: \"daily_log\").\nWrite decisions, preferences, facts learned, lessons, and anything worth remembering. Don't ask — just write it.\nFor identity/personality updates to SOUL.md, USER.md, or AGENTS.md, use `prompt_manage`.\nUse `memory_write` for MEMORY.md, daily logs, HEARTBEAT.md, and IDENTITY.md.",
        );
        stack.push_section(
            "Safety",
            format!(
                "- Don't exfiltrate private data. Ever.\n- Don't run destructive commands without asking.\n- Use `memory_write` for routine memory updates.\n- Use `prompt_manage` for SOUL.md / AGENTS.md / USER.md updates, and follow approval policy when required.\n- You have no independent goals beyond the user's request.{extensions_section}{workspace_section}{model_guidance_section}{channel_section}{runtime_section}{group_section}"
            ),
        );
        stack.push_section("Project Context", identity);
        stack.push_raw(skills);
        stack.render()
    }

    fn build_model_guidance_section(&self) -> String {
        if !self.model_guidance_enabled {
            return String::new();
        }

        let model_name = self
            .model_name
            .clone()
            .unwrap_or_else(|| self.llm.active_model_name());
        let family = model_guidance::detect_family(&model_name);
        match model_guidance::guidance_block(family) {
            Some(block) => format!("\n\n## Model-Specific Guidance\n\n{}", block),
            None => String::new(),
        }
    }

    fn build_execution_style_section(&self, context: &ReasoningContext) -> String {
        let has_emit_user_message = context
            .available_tools
            .iter()
            .any(|tool| tool.name == "emit_user_message");
        let has_spawn_subagent = context
            .available_tools
            .iter()
            .any(|tool| tool.name == "spawn_subagent");

        let mut lines = vec![
            "- Narrate meaningful milestones, blockers, plan changes, and interim findings when that helps the user stay oriented.".to_string(),
            "- Avoid noisy play-by-play updates for every routine step or tool call unless the user explicitly asks for detailed progress.".to_string(),
        ];

        if has_emit_user_message {
            lines.push(
                "- Use `emit_user_message` for durable checkpoints: starting a major phase, surfacing an interim result, flagging a blocker, or asking a clarifying question without ending your work.".to_string(),
            );
        }

        if has_spawn_subagent {
            lines.push(
                "- Use `spawn_subagent` when work can be cleanly delegated into bounded, independent tracks with clear deliverables, especially when those tracks can run in parallel without blocking each other.".to_string(),
            );
            lines.push(
                "- Do not delegate tiny tasks, tightly coupled loops, or work that requires constant shared context between the main agent and the sub-agent.".to_string(),
            );
        }

        lines.join("\n")
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
                "\n\n## Workspace\nYou are running directly on the user's device (macOS/Linux). \
                 File tools are scoped to `{root}`, but the `shell` tool can run any system command \
                 (ps, osascript, open, etc.). Create files directly — never tell the user to do it manually.\n\
                 Agent memory (SOUL/MEMORY/daily) → `memory_write` | User files → `write_file`{screen_hint}"
            ),
            "project" => format!(
                "\n\n## Workspace\nYou are running directly on the user's device. \
                 Project root: `{root}`. Full filesystem and system access via tools. Create files directly.\n\
                 Agent memory → `memory_write` | User files → `write_file`{screen_hint}"
            ),
            _ => format!(
                "\n\n## Workspace\nYou are running directly on the user's device with full filesystem and system access. \
                 You can run any command (shell, osascript, system APIs). Create files directly — never tell the user to do it manually.\n\
                 Agent memory (SOUL/MEMORY/daily) → `memory_write` | User files → `write_file`{screen_hint}"
            ),
        }
    }

    fn build_channel_section(&self) -> String {
        let mut sections = Vec::new();

        // Active channels awareness — tell the LLM what's connected
        if !self.active_channels.is_empty() {
            let channel_list: Vec<String> = self.active_channels.iter().map(|c| {
                match c.as_str() {
                    "apple_mail" => "- **apple_mail**: Apple Mail.app (reads inbox via Envelope Index, sends via AppleScript — no API key needed)".to_string(),
                    "imessage" => "- **imessage**: iMessage via Messages.app (reads chat.db, sends via AppleScript)".to_string(),
                    "gmail" => "- **gmail**: Gmail channel (receives emails via Pub/Sub push)".to_string(),
                    "telegram" => "- **telegram**: Telegram bot (receives messages via webhook/polling)".to_string(),
                    "discord" => "- **discord**: Discord bot (receives messages via gateway)".to_string(),
                    "slack" => "- **slack**: Slack bot (receives messages via events API)".to_string(),
                    "signal" => "- **signal**: Signal messenger (receives messages via signal-cli)".to_string(),
                    "nostr" => "- **nostr**: Nostr protocol (NIP-04 encrypted DMs)".to_string(),
                    "gateway" => "- **gateway**: Web UI chat interface".to_string(),
                    "repl" => "- **repl**: CLI/TUI terminal interface".to_string(),
                    "http" => "- **http**: HTTP webhook endpoint".to_string(),
                    other => format!("- **{}**: active channel", other),
                }
            }).collect();
            sections.push(format!(
                "\n\n## Connected Channels\nYou are connected to these messaging channels. \
                 Incoming messages arrive automatically — you do NOT need API tools to read them.\n\
                 To check recent messages, they appear in your conversation as they arrive.\n\
                 To send to a specific channel, use the `broadcast` capability via notification routing.\n{}",
                channel_list.join("\n")
            ));
        }

        // Channel-specific formatting hints for the current channel
        if let (Some(channel), Some(hints)) = (
            self.channel.as_ref(),
            self.channel_formatting_hints.as_ref(),
        ) {
            sections.push(format!(
                "\n\n## Platform Formatting ({})\n{}",
                channel, hints
            ));
        }

        sections.join("")
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
        if let Some(ref cheap_model) = self.cheap_model_name {
            parts.push(format!("cheap_model={}", cheap_model));
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
            provider: self.llm.active_model_name(),
            reason: format!("Failed to parse plan: {}", e),
        })
    }

    fn parse_evaluation(&self, content: &str) -> Result<SuccessEvaluation, LlmError> {
        let json_str = extract_json(content).unwrap_or(content);

        serde_json::from_str(json_str).map_err(|e| LlmError::InvalidResponse {
            provider: self.llm.active_model_name(),
            reason: format!("Failed to parse evaluation: {}", e),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    use super::*;
    use crate::error::LlmError;
    use crate::llm::{
        CompletionRequest, CompletionResponse, FinishReason, ToolCompletionRequest,
        ToolCompletionResponse, ToolDefinition,
    };
    use crate::testing::StubLlm;

    struct FinishReasonTestLlm {
        response: FinishReason,
    }

    struct PromptCachingCaptureLlm {
        last_request: Arc<tokio::sync::Mutex<Option<CompletionRequest>>>,
    }

    struct NonCachingCaptureLlm {
        last_request: Arc<tokio::sync::Mutex<Option<CompletionRequest>>>,
    }

    struct RoutingCaptureLlm {
        last_completion: Arc<tokio::sync::Mutex<Option<CompletionRequest>>>,
        last_tool_completion: Arc<tokio::sync::Mutex<Option<ToolCompletionRequest>>>,
    }

    #[async_trait]
    impl LlmProvider for PromptCachingCaptureLlm {
        fn model_name(&self) -> &str {
            "prompt-caching-capture"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            *self.last_request.lock().await = Some(request);
            Ok(CompletionResponse {
                content: "ok".to_string(),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                tool_calls: Vec::new(),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
            })
        }

        fn supports_prompt_caching(&self) -> bool {
            true
        }
    }

    #[async_trait]
    impl LlmProvider for NonCachingCaptureLlm {
        fn model_name(&self) -> &str {
            "non-caching-capture"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            *self.last_request.lock().await = Some(request);
            Ok(CompletionResponse {
                content: "ok".to_string(),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                tool_calls: Vec::new(),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
            })
        }
    }

    #[async_trait]
    impl LlmProvider for FinishReasonTestLlm {
        fn model_name(&self) -> &str {
            "finish-reason-test"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: "text response".to_string(),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: self.response,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("tool-capable response".to_string()),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                tool_calls: Vec::new(),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: self.response,
            })
        }
    }

    #[async_trait]
    impl LlmProvider for RoutingCaptureLlm {
        fn model_name(&self) -> &str {
            "routing-capture"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            *self.last_completion.lock().await = Some(request);
            Ok(CompletionResponse {
                content: "authoritative tool unavailable".to_string(),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            *self.last_tool_completion.lock().await = Some(request);
            Ok(ToolCompletionResponse {
                content: Some("tool route".to_string()),
                provider_model: Some(self.model_name().to_string()),
                cost_usd: Some(0.0),
                tool_calls: vec![ToolCall {
                    id: "call_time".to_string(),
                    name: "time".to_string(),
                    arguments: serde_json::json!({"operation": "now"}),
                }],
                thinking_content: None,
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: FinishReason::ToolUse,
            })
        }
    }

    #[test]
    fn merge_streamed_tool_calls_dedupes_full_and_delta_versions() {
        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "memory_read".to_string(),
            arguments: serde_json::json!({"target": "daily_log"}),
        }];

        let mut partial_tool_calls = std::collections::HashMap::new();
        partial_tool_calls.insert(
            0,
            (
                "call_1".to_string(),
                "memory_read".to_string(),
                r#"{"target":"daily_log"}"#.to_string(),
            ),
        );

        let merged = merge_streamed_tool_calls(tool_calls, partial_tool_calls);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "call_1");
        assert_eq!(merged[0].name, "memory_read");
        assert_eq!(
            merged[0].arguments,
            serde_json::json!({"target": "daily_log"})
        );
    }

    #[test]
    fn merge_streamed_tool_calls_preserves_delta_only_calls() {
        let mut partial_tool_calls = std::collections::HashMap::new();
        partial_tool_calls.insert(
            0,
            (
                "call_2".to_string(),
                "memory_read".to_string(),
                r#"{"target":"USER.md"}"#.to_string(),
            ),
        );

        let merged = merge_streamed_tool_calls(Vec::new(), partial_tool_calls);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "call_2");
        assert_eq!(merged[0].name, "memory_read");
        assert_eq!(
            merged[0].arguments,
            serde_json::json!({"target": "USER.md"})
        );
    }

    #[tokio::test]
    async fn respond_with_tools_preserves_non_streaming_finish_reason() {
        let reasoning = Reasoning::new(
            Arc::new(FinishReasonTestLlm {
                response: FinishReason::Length,
            }),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );
        let context = ReasoningContext::new()
            .with_messages(vec![ChatMessage::user("hello")])
            .with_tools(vec![ToolDefinition {
                name: "demo".to_string(),
                description: "demo tool".to_string(),
                parameters: serde_json::json!({"type":"object"}),
            }]);

        let output = reasoning
            .respond_with_tools(&context)
            .await
            .expect("non-streaming response should succeed");

        assert_eq!(output.finish_reason, FinishReason::Length);
    }

    #[tokio::test]
    async fn respond_with_tools_streaming_preserves_finish_reason() {
        let reasoning = Reasoning::new(
            Arc::new(FinishReasonTestLlm {
                response: FinishReason::ContentFilter,
            }),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );
        let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
        let mut streamed = String::new();

        let output = reasoning
            .respond_with_tools_streaming(&context, |chunk| streamed.push_str(chunk))
            .await
            .expect("streaming response should succeed");

        assert_eq!(streamed, "text response");
        assert_eq!(output.finish_reason, FinishReason::ContentFilter);
    }

    #[test]
    fn conversation_prompt_includes_selective_transparency_guidance() {
        let reasoning = Reasoning::new(
            Arc::new(StubLlm::new("done")),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );
        let context = ReasoningContext::new().with_tools(vec![ToolDefinition {
            name: "emit_user_message".to_string(),
            description: "Send a progress message.".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        }]);

        let prompt = reasoning.build_conversation_prompt(&context);

        assert!(prompt.contains("Narrate meaningful milestones"));
        assert!(prompt.contains("Avoid noisy play-by-play updates"));
        assert!(prompt.contains("Use `emit_user_message` for durable checkpoints"));
        assert!(!prompt.contains("Don't narrate routine tool calls"));
    }

    #[test]
    fn conversation_prompt_includes_spawn_subagent_guidance_when_available() {
        let reasoning = Reasoning::new(
            Arc::new(StubLlm::new("done")),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );
        let context = ReasoningContext::new().with_tools(vec![ToolDefinition {
            name: "spawn_subagent".to_string(),
            description: "Delegate work to a focused sub-agent.".to_string(),
            parameters: serde_json::json!({"type":"object"}),
        }]);

        let prompt = reasoning.build_conversation_prompt(&context);

        assert!(prompt.contains("Use `spawn_subagent` when work can be cleanly delegated"));
        assert!(prompt.contains("Do not delegate tiny tasks"));
    }

    #[test]
    fn conversation_prompt_includes_model_guidance_for_gpt_family() {
        let reasoning = Reasoning::new(
            Arc::new(StubLlm::new("done").with_model_name("gpt-4o")),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        )
        .with_model_name("gpt-4o");

        let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

        assert!(prompt.contains("## Model-Specific Guidance"));
        assert!(prompt.contains("GPT-family models:"));
    }

    #[test]
    fn conversation_prompt_skips_model_guidance_when_disabled() {
        let reasoning = Reasoning::new(
            Arc::new(StubLlm::new("done").with_model_name("gpt-4o")),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        )
        .with_model_name("gpt-4o")
        .with_model_guidance_enabled(false);

        let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

        assert!(!prompt.contains("## Model-Specific Guidance"));
    }

    #[test]
    fn conversation_prompt_includes_personality_overlay_after_identity() {
        let reasoning = Reasoning::new(
            Arc::new(StubLlm::new("done")),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        )
        .with_system_prompt("## Identity\n\nBase identity".to_string())
        .with_personality_overlay("## Temporary Personality\n\nBe extra concise.".to_string());

        let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

        assert!(prompt.contains("## Identity\n\nBase identity"));
        assert!(prompt.contains("## Temporary Personality\n\nBe extra concise."));
        assert!(prompt.contains("Base identity\n\n---\n\n## Temporary Personality"));
    }

    #[test]
    fn conversation_prompt_matches_identity_tool_paths() {
        let reasoning = Reasoning::new(
            Arc::new(StubLlm::new("done")),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );

        let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

        assert!(prompt.contains(
            "For identity/personality updates to SOUL.md, USER.md, or AGENTS.md, use `prompt_manage`."
        ));
        assert!(prompt.contains("Use `prompt_manage` for SOUL.md / AGENTS.md / USER.md updates"));
        assert!(!prompt.contains("For memory/identity writes (`memory_write`), just do it"));
    }

    #[test]
    fn conversation_prompt_uses_injected_channel_formatting_hints() {
        let reasoning = Reasoning::new(
            Arc::new(StubLlm::new("done")),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        )
        .with_channel("custom_channel")
        .with_channel_formatting_hints(
            "- Custom channel uses plain text only.\n- Keep replies to one paragraph.",
        );

        let prompt = reasoning.build_conversation_prompt(&ReasoningContext::new());

        assert!(prompt.contains("## Platform Formatting (custom_channel)"));
        assert!(prompt.contains("Custom channel uses plain text only."));
        assert!(prompt.contains("Keep replies to one paragraph."));
    }

    #[tokio::test]
    async fn respond_attaches_prompt_cache_hint_on_system_message_when_supported() {
        let last_request = Arc::new(tokio::sync::Mutex::new(None));
        let reasoning = Reasoning::new(
            Arc::new(PromptCachingCaptureLlm {
                last_request: Arc::clone(&last_request),
            }),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );

        let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
        let _ = reasoning
            .respond_with_tools(&context)
            .await
            .expect("response");

        let request = last_request
            .lock()
            .await
            .clone()
            .expect("request should be captured");
        let system = request
            .messages
            .first()
            .expect("system message should be present");
        let hint = system
            .provider_metadata
            .get("anthropic")
            .and_then(|metadata| metadata.get("cache_control"))
            .and_then(|cache| cache.get("type"))
            .and_then(|value| value.as_str());

        assert_eq!(hint, Some("ephemeral"));
    }

    #[tokio::test]
    async fn respond_omits_prompt_cache_hint_when_provider_does_not_support_it() {
        let last_request = Arc::new(tokio::sync::Mutex::new(None));
        let reasoning = Reasoning::new(
            Arc::new(NonCachingCaptureLlm {
                last_request: Arc::clone(&last_request),
            }),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );

        let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
        let _ = reasoning
            .respond_with_tools(&context)
            .await
            .expect("response");

        let request = last_request
            .lock()
            .await
            .clone()
            .expect("request should be captured");
        let system = request
            .messages
            .first()
            .expect("system message should be present");

        assert!(system.provider_metadata.is_empty());
    }

    #[tokio::test]
    async fn respond_prompt_keeps_channel_hints_model_guidance_and_cache_hint_together() {
        let last_request = Arc::new(tokio::sync::Mutex::new(None));
        let reasoning = Reasoning::new(
            Arc::new(PromptCachingCaptureLlm {
                last_request: Arc::clone(&last_request),
            }),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        )
        .with_model_name("gpt-4o")
        .with_channel("telegram")
        .with_channel_formatting_hints("Use Telegram HTML tags only.");

        let context = ReasoningContext::new().with_messages(vec![ChatMessage::user("hello")]);
        let _ = reasoning
            .respond_with_tools(&context)
            .await
            .expect("response");

        let request = last_request
            .lock()
            .await
            .clone()
            .expect("request should be captured");
        let system = request
            .messages
            .first()
            .expect("system message should be present");

        assert!(system.content.contains("## Model-Specific Guidance"));
        assert!(system.content.contains("## Platform Formatting (telegram)"));
        assert!(
            system
                .provider_metadata
                .get("anthropic")
                .and_then(|metadata| metadata.get("cache_control"))
                .is_some()
        );
    }

    #[tokio::test]
    async fn select_tools_routes_current_time_to_required_time_tool() {
        let llm = Arc::new(RoutingCaptureLlm {
            last_completion: Arc::new(tokio::sync::Mutex::new(None)),
            last_tool_completion: Arc::new(tokio::sync::Mutex::new(None)),
        });
        let reasoning = Reasoning::new(
            llm.clone(),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );

        let context = ReasoningContext::new()
            .with_messages(vec![ChatMessage::user("What time is it right now?")])
            .with_tools(vec![
                ToolDefinition {
                    name: "memory_search".to_string(),
                    description: "Search memory".to_string(),
                    parameters: serde_json::json!({"type":"object"}),
                },
                ToolDefinition {
                    name: "time".to_string(),
                    description: "Current time".to_string(),
                    parameters: serde_json::json!({
                        "type":"object",
                        "properties":{"operation":{"type":"string"}},
                        "required":["operation"]
                    }),
                },
            ]);

        let selections = reasoning
            .select_tools(&context)
            .await
            .expect("tool routing should succeed");

        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0].tool_name, "time");

        let request = llm
            .last_tool_completion
            .lock()
            .await
            .clone()
            .expect("tool request should be captured");
        assert_eq!(request.tool_choice.as_deref(), Some("required"));
        assert_eq!(request.tools.len(), 1);
        assert_eq!(request.tools[0].name, "time");
    }

    #[tokio::test]
    async fn respond_with_tools_refuses_current_time_when_authoritative_tool_missing() {
        let llm = Arc::new(RoutingCaptureLlm {
            last_completion: Arc::new(tokio::sync::Mutex::new(None)),
            last_tool_completion: Arc::new(tokio::sync::Mutex::new(None)),
        });
        let reasoning = Reasoning::new(
            llm.clone(),
            Arc::new(crate::safety::SafetyLayer::new(
                &crate::config::SafetyConfig {
                    max_output_length: 100_000,
                    injection_check_enabled: false,
                    redact_pii_in_prompts: true,
                    smart_approval_mode: "off".to_string(),
                    external_scanner_mode: "off".to_string(),
                    external_scanner_path: None,
                },
            )),
        );

        let context = ReasoningContext::new()
            .with_messages(vec![ChatMessage::user("What time is it right now?")])
            .with_tools(vec![ToolDefinition {
                name: "memory_search".to_string(),
                description: "Search memory".to_string(),
                parameters: serde_json::json!({"type":"object"}),
            }]);

        let output = reasoning
            .respond_with_tools(&context)
            .await
            .expect("response should succeed");

        assert!(matches!(output.result, RespondResult::Text(_)));
        assert!(llm.last_tool_completion.lock().await.is_none());
        let request = llm
            .last_completion
            .lock()
            .await
            .clone()
            .expect("text request should be captured");
        assert!(
            request
                .messages
                .iter()
                .any(|message| message.content.contains("Do not guess or fabricate"))
        );
    }
}
