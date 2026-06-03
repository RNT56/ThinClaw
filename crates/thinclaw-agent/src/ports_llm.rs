//! Agent-owned LLM and runtime-status ports.
//!
//! These DTOs are intentionally root-independent so dispatcher extraction can
//! depend on `thinclaw-agent` without importing the root `thinclaw` crate.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thinclaw_llm_core::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, ModelMetadata,
    ProviderTokenCapture, StreamChunk, StreamPolicy, StreamSupport, ThinkingConfig,
    TokenCaptureSupport, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};
use thinclaw_types::error::LlmError;

/// Receiver used for host-mediated LLM streaming.
pub type PortableLlmStream = tokio::sync::mpsc::Receiver<Result<PortableLlmStreamEvent, LlmError>>;

/// Serializable mirror of `thinclaw_llm_core::ThinkingConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PortableThinkingConfig {
    #[default]
    Disabled,
    Enabled {
        budget_tokens: u32,
    },
}

impl From<ThinkingConfig> for PortableThinkingConfig {
    fn from(value: ThinkingConfig) -> Self {
        match value {
            ThinkingConfig::Disabled => Self::Disabled,
            ThinkingConfig::Enabled { budget_tokens } => Self::Enabled { budget_tokens },
        }
    }
}

impl From<PortableThinkingConfig> for ThinkingConfig {
    fn from(value: PortableThinkingConfig) -> Self {
        match value {
            PortableThinkingConfig::Disabled => Self::Disabled,
            PortableThinkingConfig::Enabled { budget_tokens } => Self::Enabled { budget_tokens },
        }
    }
}

/// Serializable mirror of `thinclaw_llm_core::StreamPolicy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortableStreamPolicy {
    PreferNative,
    #[default]
    AllowSimulated,
    RequireNative,
}

impl From<StreamPolicy> for PortableStreamPolicy {
    fn from(value: StreamPolicy) -> Self {
        match value {
            StreamPolicy::PreferNative => Self::PreferNative,
            StreamPolicy::AllowSimulated => Self::AllowSimulated,
            StreamPolicy::RequireNative => Self::RequireNative,
        }
    }
}

impl From<PortableStreamPolicy> for StreamPolicy {
    fn from(value: PortableStreamPolicy) -> Self {
        match value {
            PortableStreamPolicy::PreferNative => Self::PreferNative,
            PortableStreamPolicy::AllowSimulated => Self::AllowSimulated,
            PortableStreamPolicy::RequireNative => Self::RequireNative,
        }
    }
}

/// Serializable mirror of `thinclaw_llm_core::FinishReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortableFinishReason {
    #[default]
    Stop,
    Length,
    ToolUse,
    ContentFilter,
    Unknown,
}

impl From<FinishReason> for PortableFinishReason {
    fn from(value: FinishReason) -> Self {
        match value {
            FinishReason::Stop => Self::Stop,
            FinishReason::Length => Self::Length,
            FinishReason::ToolUse => Self::ToolUse,
            FinishReason::ContentFilter => Self::ContentFilter,
            FinishReason::Unknown => Self::Unknown,
        }
    }
}

impl From<PortableFinishReason> for FinishReason {
    fn from(value: PortableFinishReason) -> Self {
        match value {
            PortableFinishReason::Stop => Self::Stop,
            PortableFinishReason::Length => Self::Length,
            PortableFinishReason::ToolUse => Self::ToolUse,
            PortableFinishReason::ContentFilter => Self::ContentFilter,
            PortableFinishReason::Unknown => Self::Unknown,
        }
    }
}

/// Portable chat completion request for host-mediated LLM calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortableCompletionRequest {
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_documents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub thinking: PortableThinkingConfig,
    #[serde(default)]
    pub stream_policy: PortableStreamPolicy,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl From<PortableCompletionRequest> for CompletionRequest {
    fn from(value: PortableCompletionRequest) -> Self {
        let mut request = CompletionRequest::new(value.messages);
        request.context_documents = value.context_documents;
        request.model = value.model;
        request.max_tokens = value.max_tokens;
        request.temperature = value.temperature;
        request.stop_sequences = value.stop_sequences;
        request.thinking = value.thinking.into();
        request.stream_policy = value.stream_policy.into();
        request.metadata = value.metadata;
        request
    }
}

impl From<CompletionRequest> for PortableCompletionRequest {
    fn from(value: CompletionRequest) -> Self {
        Self {
            messages: value.messages,
            context_documents: value.context_documents,
            model: value.model,
            max_tokens: value.max_tokens,
            temperature: value.temperature,
            stop_sequences: value.stop_sequences,
            thinking: value.thinking.into(),
            stream_policy: value.stream_policy.into(),
            metadata: value.metadata,
        }
    }
}

/// Portable chat completion request that allows the model to request tools.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortableToolCompletionRequest {
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_documents: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    #[serde(default)]
    pub thinking: PortableThinkingConfig,
    #[serde(default)]
    pub stream_policy: PortableStreamPolicy,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl From<PortableToolCompletionRequest> for ToolCompletionRequest {
    fn from(value: PortableToolCompletionRequest) -> Self {
        let mut request = ToolCompletionRequest::new(value.messages, value.tools);
        request.context_documents = value.context_documents;
        request.model = value.model;
        request.max_tokens = value.max_tokens;
        request.temperature = value.temperature;
        request.tool_choice = value.tool_choice;
        request.thinking = value.thinking.into();
        request.stream_policy = value.stream_policy.into();
        request.metadata = value.metadata;
        request
    }
}

impl From<ToolCompletionRequest> for PortableToolCompletionRequest {
    fn from(value: ToolCompletionRequest) -> Self {
        Self {
            messages: value.messages,
            context_documents: value.context_documents,
            tools: value.tools,
            model: value.model,
            max_tokens: value.max_tokens,
            temperature: value.temperature,
            tool_choice: value.tool_choice,
            thinking: value.thinking.into(),
            stream_policy: value.stream_policy.into(),
            metadata: value.metadata,
        }
    }
}

/// Portable chat completion response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortableCompletionResponse {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_content: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: PortableFinishReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_capture: Option<ProviderTokenCapture>,
}

impl From<CompletionResponse> for PortableCompletionResponse {
    fn from(value: CompletionResponse) -> Self {
        Self {
            content: value.content,
            provider_model: value.provider_model,
            cost_usd: value.cost_usd,
            thinking_content: value.thinking_content,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            finish_reason: value.finish_reason.into(),
            token_capture: value.token_capture,
        }
    }
}

impl From<PortableCompletionResponse> for CompletionResponse {
    fn from(value: PortableCompletionResponse) -> Self {
        Self {
            content: value.content,
            provider_model: value.provider_model,
            cost_usd: value.cost_usd,
            thinking_content: value.thinking_content,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            finish_reason: value.finish_reason.into(),
            token_capture: value.token_capture,
        }
    }
}

/// Portable completion response that may contain model-requested tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortableToolCompletionResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_content: Option<String>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub finish_reason: PortableFinishReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_capture: Option<ProviderTokenCapture>,
}

impl From<ToolCompletionResponse> for PortableToolCompletionResponse {
    fn from(value: ToolCompletionResponse) -> Self {
        Self {
            content: value.content,
            provider_model: value.provider_model,
            cost_usd: value.cost_usd,
            tool_calls: value.tool_calls,
            thinking_content: value.thinking_content,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            finish_reason: value.finish_reason.into(),
            token_capture: value.token_capture,
        }
    }
}

impl From<PortableToolCompletionResponse> for ToolCompletionResponse {
    fn from(value: PortableToolCompletionResponse) -> Self {
        Self {
            content: value.content,
            provider_model: value.provider_model,
            cost_usd: value.cost_usd,
            tool_calls: value.tool_calls,
            thinking_content: value.thinking_content,
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            finish_reason: value.finish_reason.into(),
            token_capture: value.token_capture,
        }
    }
}

/// Portable event emitted by a streaming LLM response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PortableLlmStreamEvent {
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ToolCall {
        tool_call: ToolCall,
    },
    ToolCallDelta {
        index: u32,
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments_delta: Option<String>,
    },
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
        finish_reason: PortableFinishReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_capture: Option<ProviderTokenCapture>,
    },
}

impl From<StreamChunk> for PortableLlmStreamEvent {
    fn from(value: StreamChunk) -> Self {
        match value {
            StreamChunk::Text(text) => Self::TextDelta { text },
            StreamChunk::ReasoningDelta(text) => Self::ReasoningDelta { text },
            StreamChunk::ToolCall(tool_call) => Self::ToolCall { tool_call },
            StreamChunk::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            } => Self::ToolCallDelta {
                index,
                id,
                name,
                arguments_delta,
            },
            StreamChunk::Done {
                provider_model,
                cost_usd,
                input_tokens,
                output_tokens,
                finish_reason,
                token_capture,
            } => Self::Done {
                provider_model,
                cost_usd,
                input_tokens,
                output_tokens,
                finish_reason: finish_reason.into(),
                token_capture,
            },
        }
    }
}

/// Serializable routing mode used by runtime status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortableRoutingMode {
    #[default]
    PrimaryOnly,
    CheapSplit,
    #[serde(alias = "advisor")]
    AdvisorExecutor,
    Policy,
}

/// Serializable advisor auto-escalation mode used by runtime status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PortableAdvisorAutoEscalationMode {
    ManualOnly,
    RiskOnly,
    #[default]
    RiskAndComplexFinal,
}

/// Portable LLM runtime status needed by dispatcher routing decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PortableLlmRuntimeStatus {
    #[serde(default)]
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default)]
    pub primary_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cheap_model: Option<String>,
    #[serde(default)]
    pub routing_enabled: bool,
    #[serde(default)]
    pub routing_mode: PortableRoutingMode,
    #[serde(default)]
    pub tool_phase_synthesis_enabled: bool,
    #[serde(default)]
    pub tool_phase_primary_thinking_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_chain: Vec<String>,
    #[serde(default)]
    pub advisor_max_calls: u32,
    #[serde(default)]
    pub advisor_auto_escalation_mode: PortableAdvisorAutoEscalationMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_escalation_prompt: Option<String>,
    #[serde(default)]
    pub advisor_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_disabled_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_target: Option<String>,
}

/// Portable model metadata returned by the host LLM runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableModelMetadata {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    #[serde(default)]
    pub stream_support: StreamSupport,
    #[serde(default)]
    pub token_capture_support: TokenCaptureSupport,
    #[serde(default)]
    pub prompt_caching_supported: bool,
}

impl From<ModelMetadata> for PortableModelMetadata {
    fn from(value: ModelMetadata) -> Self {
        Self {
            id: value.id,
            context_length: value.context_length,
            stream_support: StreamSupport::default(),
            token_capture_support: TokenCaptureSupport::default(),
            prompt_caching_supported: false,
        }
    }
}

/// Host LLM completion surface required by the extracted agent dispatcher.
#[async_trait]
pub trait HostLlmCompletionPort: Send + Sync {
    async fn complete(
        &self,
        request: PortableCompletionRequest,
    ) -> Result<PortableCompletionResponse, LlmError>;

    async fn complete_with_tools(
        &self,
        request: PortableToolCompletionRequest,
    ) -> Result<PortableToolCompletionResponse, LlmError>;

    async fn stream_completion(
        &self,
        request: PortableCompletionRequest,
    ) -> Result<PortableLlmStream, LlmError>;

    async fn stream_completion_with_tools(
        &self,
        request: PortableToolCompletionRequest,
    ) -> Result<PortableLlmStream, LlmError>;
}

/// Host LLM runtime diagnostics and model discovery surface.
#[async_trait]
pub trait HostLlmRuntimeStatusPort: Send + Sync {
    fn runtime_status(&self) -> PortableLlmRuntimeStatus;

    async fn list_models(&self) -> Result<Vec<String>, LlmError>;

    async fn model_metadata(
        &self,
        model: Option<String>,
    ) -> Result<PortableModelMetadata, LlmError>;
}

/// Convenience supertrait for hosts that provide both completion and status.
pub trait HostLlmPort: HostLlmCompletionPort + HostLlmRuntimeStatusPort {}

impl<T> HostLlmPort for T where T: HostLlmCompletionPort + HostLlmRuntimeStatusPort {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn completion_request_deserializes_defaults() {
        let request: PortableCompletionRequest = serde_json::from_value(json!({
            "messages": [
                {
                    "role": "user",
                    "content": "hello"
                }
            ]
        }))
        .expect("request should deserialize");

        assert_eq!(request.messages.len(), 1);
        assert!(request.context_documents.is_empty());
        assert_eq!(request.thinking, PortableThinkingConfig::Disabled);
        assert_eq!(request.stream_policy, PortableStreamPolicy::AllowSimulated);
        assert!(request.metadata.is_empty());
    }

    #[test]
    fn runtime_status_defaults_are_serializable() {
        let status = PortableLlmRuntimeStatus {
            revision: 42,
            primary_model: "openai/gpt-5".to_string(),
            routing_enabled: true,
            routing_mode: PortableRoutingMode::AdvisorExecutor,
            advisor_ready: true,
            advisor_max_calls: 3,
            ..PortableLlmRuntimeStatus::default()
        };

        let value = serde_json::to_value(&status).expect("status should serialize");

        assert_eq!(value["revision"], 42);
        assert_eq!(value["primary_model"], "openai/gpt-5");
        assert_eq!(value["routing_mode"], "advisor_executor");
        assert_eq!(
            value["advisor_auto_escalation_mode"],
            "risk_and_complex_final"
        );
        assert!(value.get("last_error").is_none());
    }

    #[test]
    fn stream_done_event_uses_portable_finish_reason() {
        let event = PortableLlmStreamEvent::Done {
            provider_model: Some("provider/model".to_string()),
            cost_usd: Some(0.01),
            input_tokens: 11,
            output_tokens: 7,
            finish_reason: PortableFinishReason::Length,
            token_capture: None,
        };

        let value = serde_json::to_value(&event).expect("event should serialize");

        assert_eq!(value["type"], "done");
        assert_eq!(value["provider_model"], "provider/model");
        assert_eq!(value["finish_reason"], "length");
        assert!(value.get("token_capture").is_none());
    }

    struct ObjectSafeLlm;

    #[async_trait::async_trait]
    impl HostLlmCompletionPort for ObjectSafeLlm {
        async fn complete(
            &self,
            _request: PortableCompletionRequest,
        ) -> Result<PortableCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "test".to_string(),
                reason: "not implemented".to_string(),
            })
        }

        async fn complete_with_tools(
            &self,
            _request: PortableToolCompletionRequest,
        ) -> Result<PortableToolCompletionResponse, LlmError> {
            Err(LlmError::RequestFailed {
                provider: "test".to_string(),
                reason: "not implemented".to_string(),
            })
        }

        async fn stream_completion(
            &self,
            _request: PortableCompletionRequest,
        ) -> Result<PortableLlmStream, LlmError> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }

        async fn stream_completion_with_tools(
            &self,
            _request: PortableToolCompletionRequest,
        ) -> Result<PortableLlmStream, LlmError> {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            Ok(rx)
        }
    }

    #[async_trait::async_trait]
    impl HostLlmRuntimeStatusPort for ObjectSafeLlm {
        fn runtime_status(&self) -> PortableLlmRuntimeStatus {
            PortableLlmRuntimeStatus::default()
        }

        async fn list_models(&self) -> Result<Vec<String>, LlmError> {
            Ok(Vec::new())
        }

        async fn model_metadata(
            &self,
            _model: Option<String>,
        ) -> Result<PortableModelMetadata, LlmError> {
            Ok(PortableModelMetadata {
                id: "test".to_string(),
                context_length: None,
                stream_support: StreamSupport::Unsupported,
                token_capture_support: TokenCaptureSupport::UNSUPPORTED,
                prompt_caching_supported: false,
            })
        }
    }

    #[test]
    fn host_llm_traits_are_object_safe() {
        let llm = ObjectSafeLlm;

        let _completion: &dyn HostLlmCompletionPort = &llm;
        let _status: &dyn HostLlmRuntimeStatusPort = &llm;
        let _host: &dyn HostLlmPort = &llm;
    }
}
