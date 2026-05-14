//! Core LLM provider traits and transport-agnostic helper types.

#![allow(clippy::too_many_arguments)]

pub mod prompt_stack;
pub mod provider;
pub mod routing_policy;
pub mod smart_routing;
pub mod streaming;
pub mod turn_analysis;

pub use prompt_stack::{PromptLayer, PromptStack};
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata,
    ProviderTokenCapture, Role, StreamChunk, StreamChunkStream, StreamPolicy, StreamSupport,
    ThinkingConfig, TokenCaptureSupport, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition, ToolResult, sanitize_tool_messages,
};
pub use routing_policy::{
    LatencyTracker, ProviderCapabilitiesMetadata, RouteCandidate, RoutingContext, RoutingDecision,
    RoutingPolicy, RoutingRule, RoutingRuleSummary, canonical_latency_key,
};
pub use smart_routing::{SmartRoutingConfig, TaskComplexity, classify_message};
pub use streaming::{
    merge_streamed_tool_calls, native_required_error, normalize_tool_name,
    simulate_stream_from_response,
};
pub use turn_analysis::{AssistantToolPlanDigest, ToolOutcomeDigest, TurnAwareness};
