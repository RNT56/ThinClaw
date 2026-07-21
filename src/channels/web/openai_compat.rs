//! OpenAI-compatible HTTP API (`/v1/chat/completions`, `/v1/models`).
//!
//! This module provides a direct LLM proxy through the web gateway so any
//! standard OpenAI client library can use ThinClaw as a backend by simply
//! changing the `base_url`.

use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use thinclaw_gateway::web::identity::{DeviceContext, GatewayRequestIdentity};
use thinclaw_gateway::web::openai_compat::{
    OpenAiChatRequest, OpenAiChatRequestPlan, OpenAiErrorKind, OpenAiErrorMapping,
    OpenAiErrorResponse, build_completion_chat_response, build_finish_chunk, build_models_response,
    build_role_chunk, build_text_chunk, build_tool_call_chunk, build_tool_call_delta_chunk,
    build_tool_completion_chat_response, chat_completion_id, convert_messages, convert_tools,
    openai_error, openai_llm_provider_not_configured_error, openai_rate_limit_error,
    plan_openai_chat_request, unix_timestamp,
};
#[cfg(test)]
use thinclaw_gateway::web::openai_compat::{
    OpenAiChatResponse, OpenAiChoice, OpenAiFunction, OpenAiMessage, OpenAiTool, OpenAiUsage,
    convert_tool_calls_to_openai, finish_reason_str, normalize_tool_choice, parse_role, parse_stop,
    validate_model_name,
};
#[cfg(test)]
use thinclaw_llm_core::{Role, ToolCall};

use crate::llm::{CompletionRequest, FinishReason, ToolCompletionRequest};

use super::server::GatewayState;

fn map_llm_error(err: crate::error::LlmError) -> (StatusCode, Json<OpenAiErrorResponse>) {
    let mapping = match &err {
        crate::error::LlmError::AuthFailed { .. }
        | crate::error::LlmError::SessionExpired { .. } => {
            OpenAiErrorMapping::new(StatusCode::UNAUTHORIZED, OpenAiErrorKind::Authentication)
                .with_code("auth_error")
        }
        crate::error::LlmError::RateLimited { .. } => {
            OpenAiErrorMapping::new(StatusCode::TOO_MANY_REQUESTS, OpenAiErrorKind::RateLimit)
                .with_code("rate_limit")
        }
        crate::error::LlmError::ContextLengthExceeded { .. } => {
            OpenAiErrorMapping::new(StatusCode::BAD_REQUEST, OpenAiErrorKind::InvalidRequest)
                .with_code("context_length_exceeded")
        }
        crate::error::LlmError::ModelNotAvailable { .. } => {
            OpenAiErrorMapping::new(StatusCode::NOT_FOUND, OpenAiErrorKind::InvalidRequest)
                .with_code("model_not_found")
        }
        _ => OpenAiErrorMapping::new(StatusCode::INTERNAL_SERVER_ERROR, OpenAiErrorKind::Server)
            .with_code("internal_error"),
    };

    mapping.into_axum_response(err.to_string())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn chat_completions_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    device_ctx: Option<axum::Extension<DeviceContext>>,
    Json(req): Json<OpenAiChatRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<OpenAiErrorResponse>)> {
    let llm = state
        .llm_provider
        .as_ref()
        .ok_or_else(openai_llm_provider_not_configured_error)?;

    let plan = plan_openai_chat_request(&req)
        .map_err(|e| openai_error(StatusCode::BAD_REQUEST, e, OpenAiErrorKind::InvalidRequest))?;

    let rate_limit_key = request_identity.rate_limit_key(device_ctx.as_ref().map(|ctx| &ctx.0));
    if !state.chat_rate_limiter.check_for(&rate_limit_key) {
        return Err(openai_rate_limit_error());
    }

    if plan.stream {
        return handle_streaming(llm.clone(), req, plan)
            .await
            .map(IntoResponse::into_response);
    }

    // --- Non-streaming path ---

    let messages = convert_messages(&req.messages)
        .map_err(|e| openai_error(StatusCode::BAD_REQUEST, e, OpenAiErrorKind::InvalidRequest))?;
    let id = chat_completion_id();
    let created = unix_timestamp();

    if plan.has_tools {
        let tools = convert_tools(req.tools.as_deref().unwrap_or(&[]));
        let mut tool_req = ToolCompletionRequest::new(messages, tools).with_model(req.model);
        if let Some(t) = req.temperature {
            tool_req = tool_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            tool_req = tool_req.with_max_tokens(mt);
        }
        if let Some(choice) = plan.tool_choice {
            tool_req = tool_req.with_tool_choice(choice);
        }

        let resp = llm
            .complete_with_tools(tool_req)
            .await
            .map_err(map_llm_error)?;
        let model_name = llm.effective_model_name(Some(plan.requested_model.as_str()));
        let response = build_tool_completion_chat_response(id, created, model_name, resp);

        Ok(Json(response).into_response())
    } else {
        let mut comp_req = CompletionRequest::new(messages).with_model(req.model);
        if let Some(t) = req.temperature {
            comp_req = comp_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            comp_req = comp_req.with_max_tokens(mt);
        }
        comp_req.stop_sequences = plan.stop_sequences;

        let resp = llm.complete(comp_req).await.map_err(map_llm_error)?;
        let model_name = llm.effective_model_name(Some(plan.requested_model.as_str()));
        let response = build_completion_chat_response(id, created, model_name, resp);

        Ok(Json(response).into_response())
    }
}

/// Handle streaming responses using real token-level streaming.
///
/// Calls `LlmProvider::complete_stream()` or `complete_stream_with_tools()` to
/// obtain a `StreamChunkStream`, then translates each `StreamChunk` into an
/// SSE event matching the OpenAI Chat Completions streaming format.
///
/// Providers that support native streaming will deliver per-token chunks;
/// those that don't will use the default simulated word-chunking fallback.
async fn handle_streaming(
    llm: Arc<dyn crate::llm::LlmProvider>,
    req: OpenAiChatRequest,
    plan: OpenAiChatRequestPlan,
) -> Result<Response, (StatusCode, Json<OpenAiErrorResponse>)> {
    use crate::llm::StreamChunk;
    use futures::StreamExt;

    let messages = convert_messages(&req.messages)
        .map_err(|e| openai_error(StatusCode::BAD_REQUEST, e, OpenAiErrorKind::InvalidRequest))?;

    let id = chat_completion_id();
    let created = unix_timestamp();
    let is_native = llm.supports_streaming_for_model(Some(req.model.as_str()));

    // Obtain the streaming chunk stream from the provider.
    let chunk_stream = if plan.has_tools {
        let tools = convert_tools(req.tools.as_deref().unwrap_or(&[]));
        let mut tool_req = ToolCompletionRequest::new(messages, tools).with_model(req.model);
        if let Some(t) = req.temperature {
            tool_req = tool_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            tool_req = tool_req.with_max_tokens(mt);
        }
        if let Some(choice) = plan.tool_choice {
            tool_req = tool_req.with_tool_choice(choice);
        }
        llm.complete_stream_with_tools(tool_req)
            .await
            .map_err(map_llm_error)?
    } else {
        let mut comp_req = CompletionRequest::new(messages).with_model(req.model);
        if let Some(t) = req.temperature {
            comp_req = comp_req.with_temperature(t);
        }
        if let Some(mt) = req.max_tokens {
            comp_req = comp_req.with_max_tokens(mt);
        }
        comp_req.stop_sequences = plan.stop_sequences;
        llm.complete_stream(comp_req).await.map_err(map_llm_error)?
    };

    let model_name = llm.effective_model_name(Some(plan.requested_model.as_str()));

    // Build the SSE stream from StreamChunks
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, std::convert::Infallible>>(64);

    tokio::spawn(async move {
        let mut chunk_stream = std::pin::pin!(chunk_stream);

        // Send initial chunk with role
        let role_chunk = build_role_chunk(&id, created, &model_name);
        let data = serde_json::to_string(&role_chunk).unwrap_or_default();
        let _ = tx.send(Ok(Event::default().data(data))).await;

        // Stream each chunk
        while let Some(chunk_result) = chunk_stream.next().await {
            match chunk_result {
                Ok(StreamChunk::Text(text)) => {
                    let chunk = build_text_chunk(&id, created, &model_name, text);
                    let data = serde_json::to_string(&chunk).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        break;
                    }
                }
                Ok(StreamChunk::ReasoningDelta(_reasoning)) => {
                    // Reasoning deltas are not part of the standard OpenAI streaming
                    // format; silently consume them for now.
                    // Future: could emit as custom SSE event or extension field.
                }
                Ok(StreamChunk::ToolCall(tc)) => {
                    let chunk = build_tool_call_chunk(&id, created, &model_name, tc);
                    let data = serde_json::to_string(&chunk).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        break;
                    }
                }
                Ok(StreamChunk::ToolCallDelta {
                    index,
                    id: tc_id,
                    name,
                    arguments_delta,
                }) => {
                    let chunk = build_tool_call_delta_chunk(
                        &id,
                        created,
                        &model_name,
                        index,
                        tc_id,
                        name,
                        arguments_delta,
                    );
                    let data = serde_json::to_string(&chunk).unwrap_or_default();
                    if tx.send(Ok(Event::default().data(data))).await.is_err() {
                        break;
                    }
                }
                Ok(StreamChunk::Done { finish_reason, .. }) => {
                    send_finish_chunk(&tx, &id, created, &model_name, finish_reason).await;
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Stream error during SSE delivery");
                    // Send an error as a finish reason
                    send_finish_chunk(&tx, &id, created, &model_name, FinishReason::Stop).await;
                    break;
                }
            }
        }

        // Send [DONE] sentinel
        let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let sse = Sse::new(stream).keep_alive(KeepAlive::new().text(""));
    let mut response = sse.into_response();
    response.headers_mut().insert(
        "x-thinclaw-streaming",
        HeaderValue::from_static(if is_native { "native" } else { "simulated" }),
    );
    Ok(response)
}

async fn send_finish_chunk(
    tx: &tokio::sync::mpsc::Sender<Result<Event, std::convert::Infallible>>,
    id: &str,
    created: u64,
    model: &str,
    reason: FinishReason,
) {
    let chunk = build_finish_chunk(id, created, model, reason);
    let data = serde_json::to_string(&chunk).unwrap_or_default();
    let _ = tx.send(Ok(Event::default().data(data))).await;
}

pub async fn models_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<OpenAiErrorResponse>)> {
    let llm = state
        .llm_provider
        .as_ref()
        .ok_or_else(openai_llm_provider_not_configured_error)?;

    let active_model = llm.active_model_name();
    let created = unix_timestamp();

    let models = match llm.list_models().await {
        Ok(names) => build_models_response(created, active_model, names),
        Err(e) => return Err(map_llm_error(e)),
    };

    Ok(Json(models))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_role() {
        assert_eq!(parse_role("system").unwrap(), Role::System);
        assert_eq!(parse_role("user").unwrap(), Role::User);
        assert_eq!(parse_role("assistant").unwrap(), Role::Assistant);
        assert_eq!(parse_role("tool").unwrap(), Role::Tool);
    }

    #[test]
    fn test_parse_role_unknown_rejected() {
        let err = parse_role("unknown").unwrap_err();
        assert!(err.contains("Unknown role"));
        assert!(err.contains("unknown"));
    }

    #[test]
    fn test_finish_reason_str() {
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
    fn test_convert_messages_basic() {
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
    fn test_convert_messages_with_tool_results() {
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
    fn test_convert_tools() {
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
    fn test_convert_tool_calls_to_openai() {
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
    fn test_normalize_tool_choice() {
        // String variant
        let v = serde_json::json!("auto");
        assert_eq!(normalize_tool_choice(&v), Some("auto".to_string()));

        // Object with function
        let v = serde_json::json!({"type": "function", "function": {"name": "foo"}});
        assert_eq!(normalize_tool_choice(&v), Some("required".to_string()));

        // Object with type only
        let v = serde_json::json!({"type": "none"});
        assert_eq!(normalize_tool_choice(&v), Some("none".to_string()));

        // Null
        let v = serde_json::Value::Null;
        assert_eq!(normalize_tool_choice(&v), None);
    }

    #[test]
    fn test_openai_request_deserialize_minimal() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}]}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "gpt-4");
        assert_eq!(req.messages.len(), 1);
        assert_eq!(req.stream, None);
        assert_eq!(req.temperature, None);
    }

    #[test]
    fn test_openai_request_deserialize_streaming() {
        let json = r#"{"model":"gpt-4","messages":[{"role":"user","content":"Hi"}],"stream":true,"temperature":0.7}"#;
        let req: OpenAiChatRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.stream, Some(true));
        assert_eq!(req.temperature, Some(0.7));
    }

    #[test]
    fn test_openai_response_serialize() {
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
    fn test_openai_message_with_null_content() {
        let json = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"search","arguments":"{\"q\":\"test\"}"}}]}"#;
        let msg: OpenAiMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, "assistant");
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_convert_messages_unknown_role_rejected() {
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
    fn test_convert_messages_tool_missing_fields() {
        // Missing tool_call_id
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

        // Missing name
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
    fn test_parse_stop_string() {
        let v = serde_json::json!("STOP");
        assert_eq!(parse_stop(&v), Some(vec!["STOP".to_string()]));
    }

    #[test]
    fn test_parse_stop_array() {
        let v = serde_json::json!(["STOP", "END"]);
        assert_eq!(
            parse_stop(&v),
            Some(vec!["STOP".to_string(), "END".to_string()])
        );
    }

    #[test]
    fn test_parse_stop_null() {
        let v = serde_json::Value::Null;
        assert_eq!(parse_stop(&v), None);
    }

    #[test]
    fn test_validate_model_name_rejects_leading_or_trailing_whitespace() {
        let err = validate_model_name(" gpt-4").unwrap_err();
        assert!(err.contains("leading or trailing whitespace"));

        let err = validate_model_name("gpt-4 ").unwrap_err();
        assert!(err.contains("leading or trailing whitespace"));
    }

    #[test]
    fn test_validate_model_name_accepts_normal_name() {
        assert!(validate_model_name("gpt-4").is_ok());
    }
}
