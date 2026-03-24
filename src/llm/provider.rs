//! LLM provider trait and types.

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::error::LlmError;

/// Configuration for extended thinking / reasoning mode.
///
/// When enabled, models that support it (e.g. Anthropic Claude with extended
/// thinking, OpenAI o-series with reasoning) will output their chain-of-thought
/// reasoning alongside the final response.
#[derive(Debug, Clone, Copy, Default)]
pub enum ThinkingConfig {
    /// Thinking is disabled (default). The model responds normally.
    #[default]
    Disabled,
    /// Thinking is enabled with a token budget for the reasoning phase.
    /// The model may use up to `budget_tokens` for its internal reasoning
    /// before producing the final response.
    Enabled {
        /// Maximum tokens the model can use for thinking/reasoning.
        /// Anthropic: maps to `thinking.budget_tokens`.
        /// OpenAI: maps to `reasoning_effort` (low/medium/high scaled).
        budget_tokens: u32,
    },
}

/// Role in a conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// Tool call ID if this is a tool result message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Name of the tool for tool results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Tool calls made by the assistant (OpenAI protocol requires these
    /// to appear on the assistant message preceding tool result messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatMessage {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }
    }

    /// Create an assistant message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            name: None,
            tool_calls: None,
        }
    }

    /// Create an assistant message that includes tool calls.
    ///
    /// Per the OpenAI protocol, an assistant message with tool_calls must
    /// precede the corresponding tool result messages in the conversation.
    pub fn assistant_with_tool_calls(content: Option<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.unwrap_or_default(),
            tool_call_id: None,
            name: None,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
        }
    }

    /// Create a tool result message.
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            name: Some(name.into()),
            tool_calls: None,
        }
    }

    /// Estimate character count for context size diagnostics.
    ///
    /// Returns a rough count of characters in this message, including content
    /// and any serialized tool calls. Not an exact token count, but useful for
    /// order-of-magnitude context window monitoring.
    pub fn estimated_chars(&self) -> usize {
        let mut chars = self.content.len();
        if let Some(ref calls) = self.tool_calls {
            for tc in calls {
                chars += tc.name.len() + tc.arguments.to_string().len() + 20; // ~20 for JSON overhead
            }
        }
        chars
    }
}

/// Request for a chat completion.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<ChatMessage>,
    /// Optional per-request model override.
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop_sequences: Option<Vec<String>>,
    /// Extended thinking / reasoning configuration.
    pub thinking: ThinkingConfig,
    /// Opaque metadata passed through to the provider (e.g. thread_id for chaining).
    pub metadata: std::collections::HashMap<String, String>,
}

impl CompletionRequest {
    /// Create a new completion request.
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            messages,
            model: None,
            max_tokens: None,
            temperature: None,
            stop_sequences: None,
            thinking: ThinkingConfig::Disabled,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Set model override.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Enable extended thinking with a token budget.
    pub fn with_thinking(mut self, budget_tokens: u32) -> Self {
        self.thinking = ThinkingConfig::Enabled { budget_tokens };
        self
    }

    /// Set the thinking configuration directly.
    pub fn set_thinking(mut self, config: ThinkingConfig) -> Self {
        self.thinking = config;
        self
    }
}

/// Response from a chat completion.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    /// Extended thinking / reasoning content, if the model produced it.
    pub thinking_content: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: FinishReason,
}

/// Why the completion finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Unknown,
}

/// Definition of a tool for the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A tool call requested by the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result of a tool execution to send back to the LLM.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub name: String,
    pub content: String,
    pub is_error: bool,
}

/// Request for a completion with tool use.
#[derive(Debug, Clone)]
pub struct ToolCompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
    /// Optional per-request model override.
    pub model: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// How to handle tool use: "auto", "required", or "none".
    pub tool_choice: Option<String>,
    /// Extended thinking / reasoning configuration.
    pub thinking: ThinkingConfig,
    /// Opaque metadata passed through to the provider (e.g. thread_id for chaining).
    pub metadata: std::collections::HashMap<String, String>,
}

impl ToolCompletionRequest {
    /// Create a new tool completion request.
    pub fn new(messages: Vec<ChatMessage>, tools: Vec<ToolDefinition>) -> Self {
        Self {
            messages,
            tools,
            model: None,
            max_tokens: None,
            temperature: None,
            tool_choice: None,
            thinking: ThinkingConfig::Disabled,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Set model override.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set max tokens.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Set temperature.
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Set tool choice mode.
    pub fn with_tool_choice(mut self, choice: impl Into<String>) -> Self {
        self.tool_choice = Some(choice.into());
        self
    }

    /// Enable extended thinking with a token budget.
    pub fn with_thinking(mut self, budget_tokens: u32) -> Self {
        self.thinking = ThinkingConfig::Enabled { budget_tokens };
        self
    }

    /// Set the thinking configuration directly.
    pub fn set_thinking(mut self, config: ThinkingConfig) -> Self {
        self.thinking = config;
        self
    }
}

/// Response from a completion with potential tool calls.
#[derive(Debug, Clone)]
pub struct ToolCompletionResponse {
    /// Text content (may be empty if tool calls are present).
    pub content: Option<String>,
    /// Tool calls requested by the model.
    pub tool_calls: Vec<ToolCall>,
    /// Extended thinking / reasoning content, if the model produced it.
    pub thinking_content: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: FinishReason,
}

/// Metadata about a model returned by the provider's API.
#[derive(Debug, Clone)]
pub struct ModelMetadata {
    pub id: String,
    /// Total context window size in tokens.
    pub context_length: Option<u32>,
}

/// A single chunk in a streaming LLM response.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// A text delta (token or word boundary chunk).
    Text(String),
    /// Extended thinking / reasoning delta.
    ReasoningDelta(String),
    /// A complete tool call (accumulated from deltas).
    ToolCall(ToolCall),
    /// A partial tool call delta — name or argument fragment.
    ToolCallDelta {
        /// The tool call index (0-based) within this response.
        index: u32,
        /// Tool call ID (may be empty for deltas after the first).
        id: String,
        /// Name delta (first delta typically carries the full name).
        name: Option<String>,
        /// Arguments delta (JSON string fragment).
        arguments_delta: Option<String>,
    },
    /// Stream is complete — carries final token counts.
    Done {
        input_tokens: u32,
        output_tokens: u32,
        finish_reason: FinishReason,
    },
}

/// Type alias for a boxed stream of StreamChunks.
pub type StreamChunkStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<StreamChunk, LlmError>> + Send>>;

/// Trait for LLM providers.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the model name.
    fn model_name(&self) -> &str;

    /// Get cost per token (input, output).
    fn cost_per_token(&self) -> (Decimal, Decimal);

    /// Complete a chat conversation.
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;

    /// Complete with tool use support.
    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError>;

    /// Stream a chat completion token-by-token.
    ///
    /// Default implementation calls `complete()` and simulates streaming
    /// by splitting the response into word-boundary chunks.
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let resp = self.complete(request).await?;
        Ok(simulate_stream_from_response(
            resp.content,
            resp.thinking_content,
            vec![],
            resp.input_tokens,
            resp.output_tokens,
            resp.finish_reason,
        ))
    }

    /// Stream a tool completion token-by-token.
    ///
    /// Default implementation calls `complete_with_tools()` and simulates streaming.
    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let resp = self.complete_with_tools(request).await?;
        Ok(simulate_stream_from_response(
            resp.content.unwrap_or_default(),
            resp.thinking_content,
            resp.tool_calls,
            resp.input_tokens,
            resp.output_tokens,
            resp.finish_reason,
        ))
    }

    /// Whether this provider supports native token-level streaming.
    /// Used by the OpenAI-compat endpoint to set the `x-thinclaw-streaming` header.
    fn supports_streaming(&self) -> bool {
        false
    }

    /// List available models from the provider.
    /// Default implementation returns empty list.
    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(Vec::new())
    }

    /// Fetch metadata for the current model (context length, etc.).
    /// Default returns the model name with no size info.
    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.model_name().to_string(),
            context_length: None,
        })
    }

    /// Resolve which model should be reported for a given request.
    ///
    /// Providers that ignore per-request model overrides should override this
    /// and return `active_model_name()`.
    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        requested_model
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_else(|| self.active_model_name())
    }

    /// Get the currently active model name.
    ///
    /// May differ from `model_name()` if the model was switched at runtime
    /// via `set_model()`. Default returns `model_name()`.
    fn active_model_name(&self) -> String {
        self.model_name().to_string()
    }

    /// Switch the active model at runtime. Not all providers support this.
    fn set_model(&self, _model: &str) -> Result<(), LlmError> {
        Err(LlmError::RequestFailed {
            provider: "unknown".to_string(),
            reason: "Runtime model switching not supported by this provider".to_string(),
        })
    }

    /// Calculate cost for a completion.
    fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Decimal {
        let (input_cost, output_cost) = self.cost_per_token();
        input_cost * Decimal::from(input_tokens) + output_cost * Decimal::from(output_tokens)
    }
}

/// Simulate a streaming response by word-chunking a completed response.
///
/// Used as the default implementation for providers that don't support
/// native token-level streaming.
fn simulate_stream_from_response(
    content: String,
    thinking_content: Option<String>,
    tool_calls: Vec<ToolCall>,
    input_tokens: u32,
    output_tokens: u32,
    finish_reason: FinishReason,
) -> StreamChunkStream {
    Box::pin(futures::stream::unfold(
        SimState::new(
            content,
            thinking_content,
            tool_calls,
            input_tokens,
            output_tokens,
            finish_reason,
        ),
        |mut state| async move {
            // Phase 1: Emit reasoning deltas
            if let Some(ref mut thinking) = state.thinking {
                if !thinking.is_empty() {
                    let chunk = std::mem::take(thinking);
                    state.thinking = None;
                    return Some((Ok(StreamChunk::ReasoningDelta(chunk)), state));
                }
                state.thinking = None;
            }

            // Phase 2: Emit content word-by-word
            if !state.words.is_empty() {
                let word = state.words.remove(0);
                return Some((Ok(StreamChunk::Text(word)), state));
            }

            // Phase 3: Emit tool calls
            if !state.tool_calls.is_empty() {
                let tc = state.tool_calls.remove(0);
                return Some((Ok(StreamChunk::ToolCall(tc)), state));
            }

            // Phase 4: Done
            if !state.done {
                state.done = true;
                return Some((
                    Ok(StreamChunk::Done {
                        input_tokens: state.input_tokens,
                        output_tokens: state.output_tokens,
                        finish_reason: state.finish_reason,
                    }),
                    state,
                ));
            }

            None
        },
    ))
}

/// Internal state for the simulated stream.
struct SimState {
    thinking: Option<String>,
    words: Vec<String>,
    tool_calls: Vec<ToolCall>,
    input_tokens: u32,
    output_tokens: u32,
    finish_reason: FinishReason,
    done: bool,
}

impl SimState {
    fn new(
        content: String,
        thinking: Option<String>,
        tool_calls: Vec<ToolCall>,
        input_tokens: u32,
        output_tokens: u32,
        finish_reason: FinishReason,
    ) -> Self {
        // Split content into word-boundary chunks, keeping ~20 char groups
        let mut words = Vec::new();
        let mut buf = String::new();
        for word in content.split_inclusive(char::is_whitespace) {
            buf.push_str(word);
            if buf.len() >= 20 {
                words.push(std::mem::take(&mut buf));
            }
        }
        if !buf.is_empty() {
            words.push(buf);
        }

        Self {
            thinking,
            words,
            tool_calls,
            input_tokens,
            output_tokens,
            finish_reason,
            done: false,
        }
    }
}

/// Sanitize a message list to ensure tool_use / tool_result integrity.
///
/// LLM APIs (especially Anthropic) require every tool_result to reference a
/// tool_call_id that exists in an immediately preceding assistant message's
/// tool_calls. Orphaned tool_results cause HTTP 400 errors.
///
/// This function:
/// 1. Tracks all tool_call_ids emitted by assistant messages.
/// 2. Rewrites orphaned tool_result messages (whose tool_call_id has no
///    matching assistant tool_call) as user messages so the content is
///    preserved without violating the protocol.
///
/// Call this before sending messages to any LLM provider.
pub fn sanitize_tool_messages(messages: &mut [ChatMessage]) {
    use std::collections::HashSet;

    // Collect all tool_call_ids from assistant messages with tool_calls.
    let mut known_ids: HashSet<String> = HashSet::new();
    for msg in messages.iter() {
        if msg.role == Role::Assistant
            && let Some(ref calls) = msg.tool_calls
        {
            for tc in calls {
                known_ids.insert(tc.id.clone());
            }
        }
    }

    // Rewrite orphaned tool_result messages as user messages.
    for msg in messages.iter_mut() {
        if msg.role != Role::Tool {
            continue;
        }
        let is_orphaned = match &msg.tool_call_id {
            Some(id) => !known_ids.contains(id),
            None => true,
        };
        if is_orphaned {
            let tool_name = msg.name.as_deref().unwrap_or("unknown");
            tracing::debug!(
                tool_call_id = ?msg.tool_call_id,
                tool_name,
                "Rewriting orphaned tool_result as user message",
            );
            msg.role = Role::User;
            msg.content = format!("[Tool `{}` returned: {}]", tool_name, msg.content);
            msg.tool_call_id = None;
            msg.name = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_preserves_valid_pairs() {
        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({}),
        };
        let mut messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_1", "echo", "result"),
        ];
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages[2].role, Role::Tool);
        assert_eq!(messages[2].tool_call_id, Some("call_1".to_string()));
    }

    #[test]
    fn test_sanitize_rewrites_orphaned_tool_result() {
        let mut messages = vec![
            ChatMessage::user("hello"),
            ChatMessage::assistant("I'll use a tool"),
            ChatMessage::tool_result("call_missing", "search", "some result"),
        ];
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages[2].role, Role::User);
        assert!(messages[2].content.contains("[Tool `search` returned:"));
        assert!(messages[2].tool_call_id.is_none());
        assert!(messages[2].name.is_none());
    }

    #[test]
    fn test_sanitize_handles_no_tool_messages() {
        let mut messages = vec![
            ChatMessage::system("prompt"),
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
        ];
        let original_len = messages.len();
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages.len(), original_len);
    }

    #[test]
    fn test_sanitize_multiple_orphaned() {
        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({}),
        };
        let mut messages = vec![
            ChatMessage::user("test"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_1", "echo", "ok"),
            // These are orphaned (call_2 and call_3 have no matching assistant message)
            ChatMessage::tool_result("call_2", "search", "orphan 1"),
            ChatMessage::tool_result("call_3", "http", "orphan 2"),
        ];
        sanitize_tool_messages(&mut messages);
        assert_eq!(messages[2].role, Role::Tool); // call_1 is valid
        assert_eq!(messages[3].role, Role::User); // call_2 orphaned
        assert_eq!(messages[4].role, Role::User); // call_3 orphaned
    }

    #[tokio::test]
    async fn test_simulated_stream_text_only() {
        use futures::StreamExt;

        let stream = simulate_stream_from_response(
            "Hello world, this is a test of streaming.".to_string(),
            None,
            vec![],
            10,
            20,
            FinishReason::Stop,
        );

        let mut chunks: Vec<StreamChunk> = Vec::new();
        let mut stream = std::pin::pin!(stream);
        while let Some(Ok(chunk)) = stream.next().await {
            chunks.push(chunk);
        }

        // Should have text chunks + Done
        assert!(
            chunks.len() >= 2,
            "Expected at least 2 chunks, got {}",
            chunks.len()
        );

        // All but last should be Text
        for chunk in &chunks[..chunks.len() - 1] {
            assert!(
                matches!(chunk, StreamChunk::Text(_)),
                "Expected Text, got {:?}",
                chunk
            );
        }

        // Last should be Done
        match &chunks[chunks.len() - 1] {
            StreamChunk::Done {
                input_tokens,
                output_tokens,
                finish_reason,
            } => {
                assert_eq!(*input_tokens, 10);
                assert_eq!(*output_tokens, 20);
                assert_eq!(*finish_reason, FinishReason::Stop);
            }
            other => panic!("Expected Done, got {:?}", other),
        }

        // Concatenated text should equal original
        let all_text: String = chunks
            .iter()
            .filter_map(|c| match c {
                StreamChunk::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(all_text, "Hello world, this is a test of streaming.");
    }

    #[tokio::test]
    async fn test_simulated_stream_with_reasoning_and_tools() {
        use futures::StreamExt;

        let tc = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
        };
        let stream = simulate_stream_from_response(
            "Result text".to_string(),
            Some("I need to think about this.".to_string()),
            vec![tc],
            5,
            15,
            FinishReason::ToolUse,
        );

        let mut chunks: Vec<StreamChunk> = Vec::new();
        let mut stream = std::pin::pin!(stream);
        while let Some(Ok(chunk)) = stream.next().await {
            chunks.push(chunk);
        }

        // Should be: ReasoningDelta, Text, ToolCall, Done (4 chunks)
        assert!(
            chunks.len() >= 4,
            "Expected at least 4 chunks, got {}",
            chunks.len()
        );

        // First should be reasoning
        assert!(
            matches!(&chunks[0], StreamChunk::ReasoningDelta(r) if r == "I need to think about this."),
            "Expected ReasoningDelta first, got {:?}",
            chunks[0]
        );

        // Tool call should appear before Done
        let has_tool_call = chunks
            .iter()
            .any(|c| matches!(c, StreamChunk::ToolCall(tc) if tc.name == "search"));
        assert!(has_tool_call, "Expected ToolCall chunk");

        // Done should carry ToolUse
        match chunks.last().unwrap() {
            StreamChunk::Done { finish_reason, .. } => {
                assert_eq!(*finish_reason, FinishReason::ToolUse);
            }
            other => panic!("Expected Done last, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_simulated_stream_empty_content() {
        use futures::StreamExt;

        let stream =
            simulate_stream_from_response(String::new(), None, vec![], 0, 0, FinishReason::Stop);

        let mut chunks: Vec<StreamChunk> = Vec::new();
        let mut stream = std::pin::pin!(stream);
        while let Some(Ok(chunk)) = stream.next().await {
            chunks.push(chunk);
        }

        // Just Done
        assert_eq!(chunks.len(), 1);
        assert!(matches!(&chunks[0], StreamChunk::Done { .. }));
    }
}
