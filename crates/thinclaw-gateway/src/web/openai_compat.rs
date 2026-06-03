//! Root-independent OpenAI-compatible gateway DTOs and conversions.

use axum::{Json, http::StatusCode};
use serde::{Deserialize, Serialize};
use thinclaw_llm_core::{
    ChatMessage, CompletionResponse, FinishReason, Role, ToolCall, ToolCompletionResponse,
    ToolDefinition,
};

pub const MAX_MODEL_NAME_BYTES: usize = 256;
pub const OPENAI_RATE_LIMIT_MESSAGE: &str = "Rate limit exceeded. Please try again later.";
pub const OPENAI_LLM_PROVIDER_NOT_CONFIGURED_MESSAGE: &str = "LLM provider not configured";
pub const OPENAI_MESSAGES_EMPTY_MESSAGE: &str = "messages must not be empty";

#[derive(Debug, Deserialize)]
pub struct OpenAiChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub tools: Option<Vec<OpenAiTool>>,
    #[serde(default)]
    pub tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    pub stop: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiToolCall>>,
    /// Extended thinking content from the model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiTool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: OpenAiFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiFunction {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: OpenAiToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiToolCallFunction {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAiChatResponse {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAiChoice>,
    pub usage: OpenAiUsage,
}

#[derive(Debug, Serialize)]
pub struct OpenAiChoice {
    pub index: u32,
    pub message: OpenAiMessage,
    pub finish_reason: String,
}

#[derive(Debug, Serialize)]
pub struct OpenAiUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Serialize)]
pub struct OpenAiChatChunk {
    pub id: String,
    pub object: &'static str,
    pub created: u64,
    pub model: String,
    pub choices: Vec<OpenAiChunkChoice>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiChunkChoice {
    pub index: u32,
    pub delta: OpenAiDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiToolCallDelta>>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiToolCallDelta {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub call_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<OpenAiToolCallFunctionDelta>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiToolCallFunctionDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct OpenAiErrorResponse {
    pub error: OpenAiErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct OpenAiErrorDetail {
    pub message: String,
    #[serde(rename = "type")]
    pub error_type: String,
    pub param: Option<String>,
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiChatRequestPlan {
    pub has_tools: bool,
    pub stream: bool,
    pub requested_model: String,
    pub tool_choice: Option<String>,
    pub stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiErrorKind {
    Authentication,
    RateLimit,
    InvalidRequest,
    Server,
}

impl OpenAiErrorKind {
    pub const fn error_type(self) -> &'static str {
        match self {
            Self::Authentication => "authentication_error",
            Self::RateLimit => "rate_limit_error",
            Self::InvalidRequest => "invalid_request_error",
            Self::Server => "server_error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenAiErrorMapping {
    pub status: StatusCode,
    pub kind: OpenAiErrorKind,
    pub code: Option<&'static str>,
}

impl OpenAiErrorMapping {
    pub const fn new(status: StatusCode, kind: OpenAiErrorKind) -> Self {
        Self {
            status,
            kind,
            code: None,
        }
    }

    pub const fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    pub fn response(self, message: impl Into<String>) -> OpenAiErrorResponse {
        build_openai_error_response(message, self.kind, self.code)
    }

    pub fn into_axum_response(
        self,
        message: impl Into<String>,
    ) -> (StatusCode, Json<OpenAiErrorResponse>) {
        (self.status, Json(self.response(message)))
    }
}

pub fn build_openai_error_response(
    message: impl Into<String>,
    kind: OpenAiErrorKind,
    code: Option<&str>,
) -> OpenAiErrorResponse {
    OpenAiErrorResponse {
        error: OpenAiErrorDetail {
            message: message.into(),
            error_type: kind.error_type().to_string(),
            param: None,
            code: code.map(str::to_string),
        },
    }
}

pub fn openai_error(
    status: StatusCode,
    message: impl Into<String>,
    kind: OpenAiErrorKind,
) -> (StatusCode, Json<OpenAiErrorResponse>) {
    OpenAiErrorMapping::new(status, kind).into_axum_response(message)
}

pub fn openai_rate_limit_error() -> (StatusCode, Json<OpenAiErrorResponse>) {
    openai_error(
        StatusCode::TOO_MANY_REQUESTS,
        OPENAI_RATE_LIMIT_MESSAGE,
        OpenAiErrorKind::RateLimit,
    )
}

pub fn openai_llm_provider_not_configured_error() -> (StatusCode, Json<OpenAiErrorResponse>) {
    openai_error(
        StatusCode::SERVICE_UNAVAILABLE,
        OPENAI_LLM_PROVIDER_NOT_CONFIGURED_MESSAGE,
        OpenAiErrorKind::Server,
    )
}

pub fn validate_openai_chat_request(req: &OpenAiChatRequest) -> Result<(), String> {
    if req.messages.is_empty() {
        return Err(OPENAI_MESSAGES_EMPTY_MESSAGE.to_string());
    }
    validate_model_name(&req.model)
}

pub fn plan_openai_chat_request(req: &OpenAiChatRequest) -> Result<OpenAiChatRequestPlan, String> {
    validate_openai_chat_request(req)?;

    Ok(OpenAiChatRequestPlan {
        has_tools: req.tools.as_ref().is_some_and(|tools| !tools.is_empty()),
        stream: req.stream.unwrap_or(false),
        requested_model: req.model.clone(),
        tool_choice: req.tool_choice.as_ref().and_then(normalize_tool_choice),
        stop_sequences: req.stop.as_ref().and_then(parse_stop),
    })
}

pub fn chat_completion_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4().simple())
}

pub fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn parse_role(s: &str) -> Result<Role, String> {
    match s {
        "system" => Ok(Role::System),
        "user" => Ok(Role::User),
        "assistant" => Ok(Role::Assistant),
        "tool" => Ok(Role::Tool),
        _ => Err(format!("Unknown role: '{s}'")),
    }
}

pub fn convert_messages(messages: &[OpenAiMessage]) -> Result<Vec<ChatMessage>, String> {
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let role = parse_role(&m.role).map_err(|e| format!("messages[{i}]: {e}"))?;
            match role {
                Role::Tool => {
                    let tool_call_id = m.tool_call_id.as_deref().ok_or_else(|| {
                        format!("messages[{i}]: tool message requires 'tool_call_id'")
                    })?;
                    let name = m
                        .name
                        .as_deref()
                        .ok_or_else(|| format!("messages[{i}]: tool message requires 'name'"))?;
                    Ok(ChatMessage::tool_result(
                        tool_call_id,
                        name,
                        m.content.as_deref().unwrap_or(""),
                    ))
                }
                Role::Assistant => {
                    if let Some(ref tcs) = m.tool_calls {
                        let calls: Vec<ToolCall> = tcs
                            .iter()
                            .map(|tc| ToolCall {
                                id: tc.id.clone(),
                                name: tc.function.name.clone(),
                                arguments: serde_json::from_str(&tc.function.arguments)
                                    .unwrap_or(serde_json::Value::Object(Default::default())),
                            })
                            .collect();
                        Ok(ChatMessage::assistant_with_tool_calls(
                            m.content.clone(),
                            calls,
                        ))
                    } else {
                        Ok(ChatMessage::assistant(m.content.as_deref().unwrap_or("")))
                    }
                }
                _ => Ok(ChatMessage {
                    role,
                    content: m.content.as_deref().unwrap_or("").to_string(),
                    tool_call_id: None,
                    name: m.name.clone(),
                    tool_calls: None,
                    provider_metadata: std::collections::HashMap::new(),
                    attachments: Vec::new(),
                }),
            }
        })
        .collect()
}

pub fn convert_tools(tools: &[OpenAiTool]) -> Vec<ToolDefinition> {
    tools
        .iter()
        .filter(|t| t.tool_type == "function")
        .map(|t| ToolDefinition {
            name: t.function.name.clone(),
            description: t.function.description.clone().unwrap_or_default(),
            parameters: t
                .function
                .parameters
                .clone()
                .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
        })
        .collect()
}

pub fn convert_tool_calls_to_openai(calls: &[ToolCall]) -> Vec<OpenAiToolCall> {
    calls
        .iter()
        .map(|tc| OpenAiToolCall {
            id: tc.id.clone(),
            call_type: "function".to_string(),
            function: OpenAiToolCallFunction {
                name: tc.name.clone(),
                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
            },
        })
        .collect()
}

pub fn finish_reason_str(reason: FinishReason) -> String {
    match reason {
        FinishReason::Stop => "stop".to_string(),
        FinishReason::Length => "length".to_string(),
        FinishReason::ToolUse => "tool_calls".to_string(),
        FinishReason::ContentFilter => "content_filter".to_string(),
        FinishReason::Unknown => "stop".to_string(),
    }
}

pub fn build_completion_chat_response(
    id: String,
    created: u64,
    model: String,
    response: CompletionResponse,
) -> OpenAiChatResponse {
    OpenAiChatResponse {
        id,
        object: "chat.completion",
        created,
        model,
        choices: vec![OpenAiChoice {
            index: 0,
            message: OpenAiMessage {
                role: "assistant".to_string(),
                content: Some(response.content),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_content: response.thinking_content,
            },
            finish_reason: finish_reason_str(response.finish_reason),
        }],
        usage: OpenAiUsage {
            prompt_tokens: response.input_tokens,
            completion_tokens: response.output_tokens,
            total_tokens: response.input_tokens + response.output_tokens,
        },
    }
}

pub fn build_tool_completion_chat_response(
    id: String,
    created: u64,
    model: String,
    response: ToolCompletionResponse,
) -> OpenAiChatResponse {
    let tool_calls_openai = if response.tool_calls.is_empty() {
        None
    } else {
        Some(convert_tool_calls_to_openai(&response.tool_calls))
    };

    OpenAiChatResponse {
        id,
        object: "chat.completion",
        created,
        model,
        choices: vec![OpenAiChoice {
            index: 0,
            message: OpenAiMessage {
                role: "assistant".to_string(),
                content: response.content,
                name: None,
                tool_call_id: None,
                tool_calls: tool_calls_openai,
                reasoning_content: response.thinking_content,
            },
            finish_reason: finish_reason_str(response.finish_reason),
        }],
        usage: OpenAiUsage {
            prompt_tokens: response.input_tokens,
            completion_tokens: response.output_tokens,
            total_tokens: response.input_tokens + response.output_tokens,
        },
    }
}

pub fn build_role_chunk(id: &str, created: u64, model: &str) -> OpenAiChatChunk {
    OpenAiChatChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![OpenAiChunkChoice {
            index: 0,
            delta: OpenAiDelta {
                role: Some("assistant".to_string()),
                content: None,
                tool_calls: None,
            },
            finish_reason: None,
        }],
    }
}

pub fn build_text_chunk(id: &str, created: u64, model: &str, text: String) -> OpenAiChatChunk {
    OpenAiChatChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![OpenAiChunkChoice {
            index: 0,
            delta: OpenAiDelta {
                role: None,
                content: Some(text),
                tool_calls: None,
            },
            finish_reason: None,
        }],
    }
}

pub fn build_tool_call_chunk(
    id: &str,
    created: u64,
    model: &str,
    call: ToolCall,
) -> OpenAiChatChunk {
    let delta = OpenAiToolCallDelta {
        index: 0,
        id: Some(call.id),
        call_type: Some("function".to_string()),
        function: Some(OpenAiToolCallFunctionDelta {
            name: Some(call.name),
            arguments: Some(serde_json::to_string(&call.arguments).unwrap_or_default()),
        }),
    };

    OpenAiChatChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![OpenAiChunkChoice {
            index: 0,
            delta: OpenAiDelta {
                role: None,
                content: None,
                tool_calls: Some(vec![delta]),
            },
            finish_reason: None,
        }],
    }
}

pub fn build_tool_call_delta_chunk(
    id: &str,
    created: u64,
    model: &str,
    index: u32,
    tool_call_id: String,
    name: Option<String>,
    arguments_delta: Option<String>,
) -> OpenAiChatChunk {
    let delta = OpenAiToolCallDelta {
        index,
        id: if tool_call_id.is_empty() {
            None
        } else {
            Some(tool_call_id)
        },
        call_type: if name.is_some() {
            Some("function".to_string())
        } else {
            None
        },
        function: Some(OpenAiToolCallFunctionDelta {
            name,
            arguments: arguments_delta,
        }),
    };

    OpenAiChatChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![OpenAiChunkChoice {
            index: 0,
            delta: OpenAiDelta {
                role: None,
                content: None,
                tool_calls: Some(vec![delta]),
            },
            finish_reason: None,
        }],
    }
}

pub fn build_finish_chunk(
    id: &str,
    created: u64,
    model: &str,
    reason: FinishReason,
) -> OpenAiChatChunk {
    OpenAiChatChunk {
        id: id.to_string(),
        object: "chat.completion.chunk",
        created,
        model: model.to_string(),
        choices: vec![OpenAiChunkChoice {
            index: 0,
            delta: OpenAiDelta {
                role: None,
                content: None,
                tool_calls: None,
            },
            finish_reason: Some(finish_reason_str(reason)),
        }],
    }
}

pub fn build_models_response(
    created: u64,
    active_model: String,
    models: Vec<String>,
) -> serde_json::Value {
    let models = if models.is_empty() {
        vec![active_model]
    } else {
        models
    };
    serde_json::json!({
        "object": "list",
        "data": models.into_iter().map(|name| {
            serde_json::json!({
                "id": name,
                "object": "model",
                "created": created,
                "owned_by": "thinclaw"
            })
        }).collect::<Vec<_>>()
    })
}

pub fn normalize_tool_choice(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(obj) => {
            if obj.contains_key("function") {
                Some("required".to_string())
            } else {
                obj.get("type")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            }
        }
        _ => None,
    }
}

pub fn validate_model_name(model: &str) -> Result<(), String> {
    let trimmed = model.trim();

    if trimmed.is_empty() {
        return Err("model must not be empty".to_string());
    }
    if trimmed != model {
        return Err("model must not have leading or trailing whitespace".to_string());
    }
    if model.len() > MAX_MODEL_NAME_BYTES {
        return Err(format!(
            "model must be at most {MAX_MODEL_NAME_BYTES} bytes"
        ));
    }
    if model.chars().any(char::is_control) {
        return Err("model contains control characters".to_string());
    }
    Ok(())
}

/// Extract stop sequences from the flexible OpenAI `stop` field.
pub fn parse_stop(val: &serde_json::Value) -> Option<Vec<String>> {
    match val {
        serde_json::Value::String(s) => Some(vec![s.clone()]),
        serde_json::Value::Array(arr) => {
            let strs: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if strs.is_empty() { None } else { Some(strs) }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_error_kind_uses_openai_type_strings() {
        assert_eq!(
            OpenAiErrorKind::Authentication.error_type(),
            "authentication_error"
        );
        assert_eq!(OpenAiErrorKind::RateLimit.error_type(), "rate_limit_error");
        assert_eq!(
            OpenAiErrorKind::InvalidRequest.error_type(),
            "invalid_request_error"
        );
        assert_eq!(OpenAiErrorKind::Server.error_type(), "server_error");
    }

    #[test]
    fn openai_error_mapping_builds_error_json_without_code() {
        let (status, Json(response)) = openai_error(
            StatusCode::BAD_REQUEST,
            "messages must not be empty",
            OpenAiErrorKind::InvalidRequest,
        );

        let json = serde_json::to_value(response).unwrap();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["message"], "messages must not be empty");
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert!(json["error"]["param"].is_null());
        assert!(json["error"]["code"].is_null());
    }

    #[test]
    fn openai_error_mapping_preserves_status_kind_and_code() {
        let mapping =
            OpenAiErrorMapping::new(StatusCode::NOT_FOUND, OpenAiErrorKind::InvalidRequest)
                .with_code("model_not_found");
        let (status, Json(response)) = mapping.into_axum_response("model is unavailable");

        let json = serde_json::to_value(response).unwrap();
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"]["type"], "invalid_request_error");
        assert_eq!(json["error"]["code"], "model_not_found");
    }

    #[test]
    fn openai_boundary_error_helpers_preserve_statuses_and_messages() {
        let (status, Json(response)) = openai_rate_limit_error();
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(json["error"]["message"], OPENAI_RATE_LIMIT_MESSAGE);
        assert_eq!(json["error"]["type"], "rate_limit_error");

        let (status, Json(response)) = openai_llm_provider_not_configured_error();
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            json["error"]["message"],
            OPENAI_LLM_PROVIDER_NOT_CONFIGURED_MESSAGE
        );
        assert_eq!(json["error"]["type"], "server_error");
    }

    #[test]
    fn chat_completion_id_matches_openai_prefix_and_uuid_payload() {
        let id = chat_completion_id();
        let uuid_payload = id.strip_prefix("chatcmpl-").unwrap();

        assert_eq!(uuid_payload.len(), 32);
        assert!(uuid_payload.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn unix_timestamp_returns_current_epoch_seconds() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let actual = unix_timestamp();
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        assert!(actual >= before);
        assert!(actual <= after);
    }

    #[test]
    fn parse_role_accepts_known_roles() {
        assert_eq!(parse_role("system").unwrap(), Role::System);
        assert_eq!(parse_role("user").unwrap(), Role::User);
        assert_eq!(parse_role("assistant").unwrap(), Role::Assistant);
        assert_eq!(parse_role("tool").unwrap(), Role::Tool);
    }

    #[test]
    fn parse_role_unknown_rejected() {
        let err = parse_role("unknown").unwrap_err();
        assert!(err.contains("Unknown role"));
        assert!(err.contains("unknown"));
    }

    #[test]
    fn finish_reason_strings_match_openai_values() {
        assert_eq!(finish_reason_str(FinishReason::Stop), "stop");
        assert_eq!(finish_reason_str(FinishReason::Length), "length");
        assert_eq!(finish_reason_str(FinishReason::ToolUse), "tool_calls");
        assert_eq!(
            finish_reason_str(FinishReason::ContentFilter),
            "content_filter"
        );
        assert_eq!(finish_reason_str(FinishReason::Unknown), "stop");
    }

    #[test]
    fn convert_messages_basic() {
        let msgs = vec![
            OpenAiMessage {
                role: "system".to_string(),
                content: Some("You are helpful.".to_string()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_content: None,
            },
            OpenAiMessage {
                role: "user".to_string(),
                content: Some("Hello".to_string()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_content: None,
            },
        ];

        let converted = convert_messages(&msgs).unwrap();
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0].role, Role::System);
        assert_eq!(converted[0].content, "You are helpful.");
        assert_eq!(converted[1].role, Role::User);
        assert_eq!(converted[1].content, "Hello");
    }

    #[test]
    fn convert_messages_with_tool_results() {
        let msgs = vec![OpenAiMessage {
            role: "tool".to_string(),
            content: Some("42".to_string()),
            name: Some("calculator".to_string()),
            tool_call_id: Some("call_123".to_string()),
            tool_calls: None,
            reasoning_content: None,
        }];

        let converted = convert_messages(&msgs).unwrap();
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, Role::Tool);
        assert_eq!(converted[0].content, "42");
        assert_eq!(converted[0].tool_call_id.as_deref(), Some("call_123"));
        assert_eq!(converted[0].name.as_deref(), Some("calculator"));
    }

    #[test]
    fn convert_tools_keeps_function_tools() {
        let tools = vec![OpenAiTool {
            tool_type: "function".to_string(),
            function: OpenAiFunction {
                name: "get_weather".to_string(),
                description: Some("Get weather for a location".to_string()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {
                        "location": { "type": "string" }
                    },
                    "required": ["location"]
                })),
            },
        }];

        let converted = convert_tools(&tools);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].name, "get_weather");
        assert_eq!(converted[0].description, "Get weather for a location");
    }

    #[test]
    fn convert_tool_calls_to_openai_function_shape() {
        let calls = vec![ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        }];

        let converted = convert_tool_calls_to_openai(&calls);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].id, "call_abc");
        assert_eq!(converted[0].call_type, "function");
        assert_eq!(converted[0].function.name, "search");
        assert!(converted[0].function.arguments.contains("rust"));
    }

    #[test]
    fn normalize_tool_choice_supports_openai_shapes() {
        assert_eq!(
            normalize_tool_choice(&serde_json::json!("auto")),
            Some("auto".to_string())
        );
        assert_eq!(
            normalize_tool_choice(
                &serde_json::json!({"type": "function", "function": {"name": "foo"}})
            ),
            Some("required".to_string())
        );
        assert_eq!(
            normalize_tool_choice(&serde_json::json!({"type": "none"})),
            Some("none".to_string())
        );
        assert_eq!(normalize_tool_choice(&serde_json::Value::Null), None);
    }

    #[test]
    fn openai_request_deserializes_minimal_body() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}]}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "gpt-4");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.stream, None);
        assert_eq!(req.temperature, None);
    }

    #[test]
    fn openai_request_deserializes_streaming_body() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}],"stream":true,"temperature":0.7}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.stream, Some(true));
        assert_eq!(req.temperature, Some(0.7));
    }

    #[test]
    fn chat_request_plan_validates_and_normalizes_openai_options() {
        let req: OpenAiChatRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hi"}],
            "tools": [{
                "type": "function",
                "function": {"name": "search"}
            }],
            "tool_choice": {"type": "function", "function": {"name": "search"}},
            "stop": ["STOP", "END"]
        }))
        .unwrap();

        let plan = plan_openai_chat_request(&req).unwrap();

        assert!(plan.has_tools);
        assert!(!plan.stream);
        assert_eq!(plan.requested_model, "gpt-4");
        assert_eq!(plan.tool_choice.as_deref(), Some("required"));
        assert_eq!(
            plan.stop_sequences,
            Some(vec!["STOP".to_string(), "END".to_string()])
        );
    }

    #[test]
    fn chat_request_plan_rejects_empty_messages_and_invalid_model() {
        let empty_messages: OpenAiChatRequest = serde_json::from_value(serde_json::json!({
            "model": "gpt-4",
            "messages": []
        }))
        .unwrap();
        assert_eq!(
            plan_openai_chat_request(&empty_messages),
            Err(OPENAI_MESSAGES_EMPTY_MESSAGE.to_string())
        );

        let invalid_model: OpenAiChatRequest = serde_json::from_value(serde_json::json!({
            "model": " gpt-4",
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .unwrap();
        assert!(
            plan_openai_chat_request(&invalid_model)
                .unwrap_err()
                .contains("leading or trailing whitespace")
        );
    }

    #[test]
    fn openai_response_serializes_standard_shape() {
        let resp = OpenAiChatResponse {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion",
            created: 1234567890,
            model: "test-model".to_string(),
            choices: vec![OpenAiChoice {
                index: 0,
                message: OpenAiMessage {
                    role: "assistant".to_string(),
                    content: Some("Hello!".to_string()),
                    name: None,
                    tool_call_id: None,
                    tool_calls: None,
                    reasoning_content: None,
                },
                finish_reason: "stop".to_string(),
            }],
            usage: OpenAiUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            },
        };

        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["object"], "chat.completion");
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert_eq!(json["choices"][0]["message"]["content"], "Hello!");
        assert_eq!(json["usage"]["total_tokens"], 15);
    }

    #[test]
    fn completion_builder_maps_core_response() {
        let response = build_completion_chat_response(
            "chatcmpl-test".to_string(),
            123,
            "test-model".to_string(),
            CompletionResponse {
                content: "Hello".to_string(),
                provider_model: None,
                cost_usd: None,
                thinking_content: Some("reasoning".to_string()),
                input_tokens: 7,
                output_tokens: 3,
                finish_reason: FinishReason::Stop,
                token_capture: None,
            },
        );

        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["id"], "chatcmpl-test");
        assert_eq!(json["model"], "test-model");
        assert_eq!(json["choices"][0]["message"]["content"], "Hello");
        assert_eq!(
            json["choices"][0]["message"]["reasoning_content"],
            "reasoning"
        );
        assert_eq!(json["choices"][0]["finish_reason"], "stop");
        assert_eq!(json["usage"]["total_tokens"], 10);
    }

    #[test]
    fn tool_completion_builder_maps_tool_calls() {
        let response = build_tool_completion_chat_response(
            "chatcmpl-tools".to_string(),
            123,
            "tool-model".to_string(),
            ToolCompletionResponse {
                content: None,
                provider_model: None,
                cost_usd: None,
                tool_calls: vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: serde_json::json!({"q": "rust"}),
                }],
                thinking_content: None,
                input_tokens: 5,
                output_tokens: 6,
                finish_reason: FinishReason::ToolUse,
                token_capture: None,
            },
        );

        let json = serde_json::to_value(response).unwrap();
        assert_eq!(
            json["choices"][0]["message"]["tool_calls"][0]["id"],
            "call_1"
        );
        assert_eq!(
            json["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "search"
        );
        assert_eq!(json["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(json["usage"]["total_tokens"], 11);
    }

    #[test]
    fn stream_chunk_builders_match_openai_chunk_shape() {
        let role = build_role_chunk("chatcmpl-stream", 123, "stream-model");
        let text = build_text_chunk("chatcmpl-stream", 123, "stream-model", "hi".to_string());
        let finish = build_finish_chunk("chatcmpl-stream", 123, "stream-model", FinishReason::Stop);

        let role_json = serde_json::to_value(role).unwrap();
        assert_eq!(role_json["object"], "chat.completion.chunk");
        assert_eq!(role_json["choices"][0]["delta"]["role"], "assistant");

        let text_json = serde_json::to_value(text).unwrap();
        assert_eq!(text_json["choices"][0]["delta"]["content"], "hi");

        let finish_json = serde_json::to_value(finish).unwrap();
        assert_eq!(finish_json["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn model_list_builder_falls_back_to_active_model() {
        let response = build_models_response(123, "active-model".to_string(), Vec::new());
        assert_eq!(response["object"], "list");
        assert_eq!(response["data"][0]["id"], "active-model");
        assert_eq!(response["data"][0]["created"], 123);
    }

    #[test]
    fn openai_message_accepts_null_assistant_content_with_tool_calls() {
        let json = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"test\"}"}}]}"#;
        let msg: OpenAiMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, "assistant");
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn convert_messages_unknown_role_rejected() {
        let msgs = vec![OpenAiMessage {
            role: "moderator".to_string(),
            content: Some("Hi".to_string()),
            name: None,
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }];
        let err = convert_messages(&msgs).unwrap_err();
        assert!(err.contains("messages[0]"));
        assert!(err.contains("Unknown role"));
    }

    #[test]
    fn convert_messages_tool_requires_tool_call_id_and_name() {
        let msgs = vec![OpenAiMessage {
            role: "tool".to_string(),
            content: Some("result".to_string()),
            name: Some("calc".to_string()),
            tool_call_id: None,
            tool_calls: None,
            reasoning_content: None,
        }];
        let err = convert_messages(&msgs).unwrap_err();
        assert!(err.contains("tool_call_id"));

        let msgs = vec![OpenAiMessage {
            role: "tool".to_string(),
            content: Some("result".to_string()),
            name: None,
            tool_call_id: Some("call_1".to_string()),
            tool_calls: None,
            reasoning_content: None,
        }];
        let err = convert_messages(&msgs).unwrap_err();
        assert!(err.contains("'name'"));
    }

    #[test]
    fn parse_stop_supports_string_array_and_null() {
        assert_eq!(
            parse_stop(&serde_json::json!("STOP")),
            Some(vec!["STOP".to_string()])
        );
        assert_eq!(
            parse_stop(&serde_json::json!(["STOP", "END"])),
            Some(vec!["STOP".to_string(), "END".to_string()])
        );
        assert_eq!(parse_stop(&serde_json::Value::Null), None);
    }

    #[test]
    fn validate_model_name_rejects_leading_or_trailing_whitespace() {
        let err = validate_model_name(" gpt-4").unwrap_err();
        assert!(err.contains("leading or trailing whitespace"));

        let err = validate_model_name("gpt-4 ").unwrap_err();
        assert!(err.contains("leading or trailing whitespace"));
    }

    #[test]
    fn validate_model_name_accepts_normal_name() {
        assert!(validate_model_name("gpt-4").is_ok());
    }
}
