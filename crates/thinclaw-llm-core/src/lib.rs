//! Core LLM provider traits and transport-agnostic helper types.

#![allow(clippy::too_many_arguments)]

pub mod provider;
pub mod streaming;

pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    ProviderTokenCapture, Role, StreamChunk, StreamChunkStream, StreamPolicy, StreamSupport,
    ThinkingConfig, TokenCaptureSupport, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition, ToolResult, sanitize_tool_messages,
};
pub use streaming::{
    merge_streamed_tool_calls, native_required_error, normalize_tool_name,
    simulate_stream_from_response,
};
