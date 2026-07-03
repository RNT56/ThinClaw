//! Generic adapter that bridges rig-core's `CompletionModel` trait to ThinClaw's `LlmProvider`.
//!
//! This lets us use any rig-core provider (OpenAI, Anthropic, Ollama, etc.) as an
//! `Arc<dyn LlmProvider>` without changing any of the agent, reasoning, or tool code.

use async_trait::async_trait;
use rig::OneOrMany;
use rig::completion::{
    AssistantContent, CompletionModel, CompletionRequest as RigRequest,
    ToolDefinition as RigToolDefinition, Usage as RigUsage,
};
use rig::message::{
    Message as RigMessage, ReasoningContent, ToolChoice as RigToolChoice, ToolFunction,
    ToolResult as RigToolResult, ToolResultContent, UserContent,
};
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as JsonValue;

use std::collections::HashSet;

use crate::costs;
use thinclaw_llm_core::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider,
    ProviderTokenCapture, StreamChunk, StreamChunkStream, StreamPolicy, StreamSupport,
    ThinkingConfig, TokenCaptureSupport, ToolCall as IronToolCall, ToolCompletionRequest,
    ToolCompletionResponse, ToolDefinition as IronToolDefinition,
};
use thinclaw_llm_core::streaming::{normalize_tool_name, simulate_stream_from_response};
use thinclaw_types::error::LlmError;

/// Adapter that wraps a rig-core `CompletionModel` and implements `LlmProvider`.
pub struct RigAdapter<M: CompletionModel + 'static> {
    model: M,
    model_name: String,
    input_cost: Decimal,
    output_cost: Decimal,
    prompt_caching: bool,
    stream_support: StreamSupport,
    token_capture_support: TokenCaptureSupport,
    token_capture_params: Option<JsonValue>,
    provider_label: String,
}

impl<M: CompletionModel + 'static> RigAdapter<M> {
    /// Create a new adapter wrapping the given rig-core model.
    pub fn new(model: M, model_name: impl Into<String>) -> Self {
        Self::new_with_prompt_caching(model, model_name, false)
    }

    /// Create a new adapter and record whether the wrapped model enables
    /// provider-side prompt caching.
    pub fn new_with_prompt_caching(
        model: M,
        model_name: impl Into<String>,
        prompt_caching: bool,
    ) -> Self {
        Self::new_with_prompt_caching_and_stream_support(
            model,
            model_name,
            prompt_caching,
            StreamSupport::Native,
        )
    }

    /// Create a new adapter with an explicit streaming capability.
    pub fn new_with_stream_support(
        model: M,
        model_name: impl Into<String>,
        stream_support: StreamSupport,
    ) -> Self {
        Self::new_with_prompt_caching_and_stream_support(model, model_name, false, stream_support)
    }

    /// Create a new adapter with explicit prompt-caching and streaming metadata.
    pub fn new_with_prompt_caching_and_stream_support(
        model: M,
        model_name: impl Into<String>,
        prompt_caching: bool,
        stream_support: StreamSupport,
    ) -> Self {
        let name = model_name.into();
        let (input_cost, output_cost) =
            costs::model_cost(&name).unwrap_or_else(costs::default_cost);
        Self {
            model,
            model_name: name,
            input_cost,
            output_cost,
            prompt_caching,
            stream_support,
            token_capture_support: TokenCaptureSupport::UNSUPPORTED,
            token_capture_params: None,
            provider_label: "rig".to_string(),
        }
    }

    /// Attach a stable provider label for telemetry and trajectory artifacts.
    pub fn with_provider_label(mut self, provider_label: impl Into<String>) -> Self {
        self.provider_label = provider_label.into();
        self
    }

    /// Declare provider-native exact token/logprob support and the
    /// provider-specific request parameters needed to ask for that data.
    pub fn with_token_capture(
        mut self,
        support: TokenCaptureSupport,
        request_params: Option<JsonValue>,
    ) -> Self {
        self.token_capture_support = support;
        self.token_capture_params = request_params;
        self
    }

    fn streaming_policy_error(&self) -> LlmError {
        thinclaw_llm_core::streaming::native_required_error(
            self.model_name.clone(),
            self.active_model_name(),
        )
    }

    async fn simulated_stream_from_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let resp = self.complete(request).await?;
        Ok(simulate_stream_from_response(
            resp.content,
            resp.provider_model,
            resp.cost_usd,
            resp.thinking_content,
            vec![],
            resp.input_tokens,
            resp.output_tokens,
            resp.finish_reason,
            resp.token_capture,
        ))
    }

    async fn simulated_stream_from_tool_completion(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        let resp = self.complete_with_tools(request).await?;
        Ok(simulate_stream_from_response(
            resp.content.unwrap_or_default(),
            resp.provider_model,
            resp.cost_usd,
            resp.thinking_content,
            resp.tool_calls,
            resp.input_tokens,
            resp.output_tokens,
            resp.finish_reason,
            resp.token_capture,
        ))
    }
}

fn requested_model_matches_active_model(requested_model: &str, active_model: &str) -> bool {
    let requested_model = requested_model.trim();
    if requested_model == active_model {
        return true;
    }

    requested_model
        .rsplit_once('/')
        .map(|(_, model)| model == active_model)
        .unwrap_or(false)
}

// -- Type conversion helpers --

/// Normalize a JSON Schema for OpenAI strict mode compliance.
///
/// OpenAI strict function calling requires:
/// - Every object must have `"additionalProperties": false`
/// - `"required"` must list ALL property keys
/// - Optional fields use `"type": ["<original>", "null"]` instead of being omitted from `required`
/// - Nested objects and array items are recursively normalized
///
/// This is applied as a clone-and-transform at the provider boundary so the
/// original tool definitions remain unchanged for other providers.
fn normalize_schema_strict(schema: &JsonValue) -> JsonValue {
    let mut schema = schema.clone();
    normalize_schema_recursive(&mut schema);
    schema
}

fn normalize_schema_recursive(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    // Recurse into combinators: anyOf, oneOf, allOf
    for key in &["anyOf", "oneOf", "allOf"] {
        if let Some(JsonValue::Array(variants)) = obj.get_mut(*key) {
            for variant in variants.iter_mut() {
                normalize_schema_recursive(variant);
            }
        }
    }

    // Recurse into array items
    if let Some(items) = obj.get_mut("items") {
        normalize_schema_recursive(items);
    }

    // Recurse into `not`, `if`, `then`, `else`
    for key in &["not", "if", "then", "else"] {
        if let Some(sub) = obj.get_mut(*key) {
            normalize_schema_recursive(sub);
        }
    }

    // Only apply object-level normalization if this schema has "properties"
    // (explicit object schema) or type == "object"
    let is_object = obj
        .get("type")
        .and_then(|t| t.as_str())
        .map(|t| t == "object")
        .unwrap_or(false);
    let has_properties = obj.contains_key("properties");

    if !is_object && !has_properties {
        return;
    }

    // Ensure "type": "object" is present
    if !obj.contains_key("type") && has_properties {
        obj.insert("type".to_string(), JsonValue::String("object".to_string()));
    }

    // Force additionalProperties: false (overwrite any existing value)
    obj.insert("additionalProperties".to_string(), JsonValue::Bool(false));

    // Ensure "properties" exists
    if !obj.contains_key("properties") {
        obj.insert(
            "properties".to_string(),
            JsonValue::Object(serde_json::Map::new()),
        );
    }

    // Collect current required set
    let current_required: std::collections::HashSet<String> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Get all property keys (sorted for deterministic output)
    let all_keys: Vec<String> = obj
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| {
            let mut keys: Vec<String> = props.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default();

    // For properties NOT in the original required list, make them nullable
    if let Some(JsonValue::Object(props)) = obj.get_mut("properties") {
        for key in &all_keys {
            // Recurse into each property's schema FIRST (before make_nullable,
            // which may change the type to an array and prevent object detection)
            if let Some(prop_schema) = props.get_mut(key) {
                normalize_schema_recursive(prop_schema);
            }
            // Then make originally-optional properties nullable
            if !current_required.contains(key)
                && let Some(prop_schema) = props.get_mut(key)
            {
                make_nullable(prop_schema);
            }
        }
    }

    // Set required to ALL property keys
    let required_value: Vec<JsonValue> = all_keys.into_iter().map(JsonValue::String).collect();
    obj.insert("required".to_string(), JsonValue::Array(required_value));
}

/// Make a property schema nullable for OpenAI strict mode.
///
/// If it has a simple `"type": "<T>"`, converts to `"type": ["<T>", "null"]`.
/// If it already has an array type, adds "null" if not present.
/// Otherwise, wraps with `anyOf: [<existing>, {"type": "null"}]`.
fn make_nullable(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    if let Some(type_val) = obj.get("type").cloned() {
        match type_val {
            // "type": "string" → "type": ["string", "null"]
            JsonValue::String(ref t) if t != "null" => {
                obj.insert("type".to_string(), serde_json::json!([t, "null"]));
            }
            // "type": ["string", "integer"] → add "null" if missing
            JsonValue::Array(ref arr) => {
                let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
                if !has_null {
                    let mut new_arr = arr.clone();
                    new_arr.push(JsonValue::String("null".to_string()));
                    obj.insert("type".to_string(), JsonValue::Array(new_arr));
                }
            }
            _ => {}
        }
    } else {
        // No "type" key — wrap with anyOf including null
        // (handles enum-only, $ref, or combinator schemas)
        let existing = JsonValue::Object(obj.clone());
        obj.clear();
        obj.insert(
            "anyOf".to_string(),
            serde_json::json!([existing, {"type": "null"}]),
        );
    }
}

/// Convert ThinClaw messages to rig-core format.
///
/// Returns `(preamble, chat_history, cache_hint_requested)` where preamble is
/// extracted from System messages, chat_history contains non-system messages,
/// and `cache_hint_requested` indicates a provider metadata cache-control hint
/// was found on at least one system message.
///
/// When a user message carries image attachments, the message is converted
/// to a multimodal `UserContent` with both image and text parts. This is
/// provider-agnostic: rig-core handles the format conversion for OpenAI,
/// Anthropic, Gemini, Ollama, etc.
fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<RigMessage>, bool) {
    let mut preamble: Option<String> = None;
    let mut history = Vec::new();
    let mut cache_hint_requested = false;

    for msg in messages {
        match msg.role {
            thinclaw_llm_core::Role::System => {
                if has_anthropic_ephemeral_cache_hint(msg) {
                    cache_hint_requested = true;
                }
                // Concatenate system messages into preamble
                match preamble {
                    Some(ref mut p) => {
                        p.push('\n');
                        p.push_str(&msg.content);
                    }
                    None => preamble = Some(msg.content.clone()),
                }
            }
            thinclaw_llm_core::Role::User => {
                // Check for multimodal attachments (images, audio, video)
                let multimodal_attachments: Vec<_> = msg
                    .attachments
                    .iter()
                    .filter(|a| {
                        matches!(
                            a.media_type,
                            thinclaw_types::MediaType::Image
                                | thinclaw_types::MediaType::Audio
                                | thinclaw_types::MediaType::Video
                        )
                    })
                    .collect();

                if multimodal_attachments.is_empty() {
                    // Text-only user message
                    history.push(RigMessage::user(&msg.content));
                } else {
                    // Multimodal: media + text
                    let mut parts: Vec<UserContent> = Vec::new();

                    for att in &multimodal_attachments {
                        match att.media_type {
                            thinclaw_types::MediaType::Image => {
                                let media_type = mime_to_rig_image_type(&att.mime_type);
                                parts.push(UserContent::image_base64(
                                    att.to_base64(),
                                    media_type,
                                    Some(rig::message::ImageDetail::Auto),
                                ));
                            }
                            thinclaw_types::MediaType::Audio => {
                                let media_type = mime_to_rig_audio_type(&att.mime_type);
                                parts.push(UserContent::audio(att.to_base64(), media_type));
                            }
                            thinclaw_types::MediaType::Video => {
                                let media_type = mime_to_rig_video_type(&att.mime_type);
                                parts.push(UserContent::Video(rig::message::Video {
                                    data: rig::message::DocumentSourceKind::Base64(att.to_base64()),
                                    media_type,
                                    additional_params: None,
                                }));
                            }
                            _ => {} // Filtered out above
                        }
                    }

                    // Add text content (may be empty for media-only messages)
                    if !msg.content.is_empty() {
                        parts.push(UserContent::text(&msg.content));
                    }

                    if let Ok(many) = OneOrMany::many(parts) {
                        history.push(RigMessage::User { content: many });
                    } else {
                        // Fallback: text-only
                        history.push(RigMessage::user(&msg.content));
                    }

                    tracing::debug!(
                        media_count = multimodal_attachments.len(),
                        types = ?multimodal_attachments.iter().map(|a| a.media_type.to_string()).collect::<Vec<_>>(),
                        "Built multimodal user message with media attachments"
                    );
                }
            }
            thinclaw_llm_core::Role::Assistant => {
                if let Some(ref tool_calls) = msg.tool_calls {
                    // Assistant message with tool calls
                    let mut contents: Vec<AssistantContent> = Vec::new();
                    if !msg.content.is_empty() {
                        contents.push(AssistantContent::text(&msg.content));
                    }
                    for (idx, tc) in tool_calls.iter().enumerate() {
                        let tool_call_id =
                            normalized_tool_call_id(Some(tc.id.as_str()), history.len() + idx);
                        contents.push(AssistantContent::ToolCall(
                            rig::message::ToolCall::new(
                                tool_call_id.clone(),
                                ToolFunction::new(tc.name.clone(), tc.arguments.clone()),
                            )
                            .with_call_id(tool_call_id),
                        ));
                    }
                    if let Ok(many) = OneOrMany::many(contents) {
                        history.push(RigMessage::Assistant {
                            id: None,
                            content: many,
                        });
                    } else {
                        // Shouldn't happen but fall back to text
                        history.push(RigMessage::assistant(&msg.content));
                    }
                } else {
                    history.push(RigMessage::assistant(&msg.content));
                }
            }
            thinclaw_llm_core::Role::Tool => {
                // Tool result message: wrap as User { ToolResult }
                let tool_id = normalized_tool_call_id(msg.tool_call_id.as_deref(), history.len());
                history.push(RigMessage::User {
                    content: OneOrMany::one(UserContent::ToolResult(RigToolResult {
                        id: tool_id.clone(),
                        call_id: Some(tool_id),
                        content: OneOrMany::one(ToolResultContent::text(&msg.content)),
                    })),
                });
            }
        }
    }

    (preamble, history, cache_hint_requested)
}

fn has_anthropic_ephemeral_cache_hint(message: &ChatMessage) -> bool {
    message
        .provider_metadata
        .get("anthropic")
        .and_then(|metadata| metadata.get("cache_control"))
        .and_then(|cache| cache.get("type"))
        .and_then(|value| value.as_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("ephemeral"))
}

/// Map a MIME type string to rig-core's `ImageMediaType`.
///
/// Returns `None` for unrecognized types (rig will still try to handle them).
fn mime_to_rig_image_type(mime: &str) -> Option<rig::message::ImageMediaType> {
    match mime.to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => Some(rig::message::ImageMediaType::JPEG),
        "image/png" => Some(rig::message::ImageMediaType::PNG),
        "image/gif" => Some(rig::message::ImageMediaType::GIF),
        "image/webp" => Some(rig::message::ImageMediaType::WEBP),
        "image/heic" => Some(rig::message::ImageMediaType::HEIC),
        "image/heif" => Some(rig::message::ImageMediaType::HEIF),
        "image/svg+xml" => Some(rig::message::ImageMediaType::SVG),
        _ => None,
    }
}

fn mime_to_rig_audio_type(mime: &str) -> Option<rig::message::AudioMediaType> {
    match mime.to_ascii_lowercase().as_str() {
        "audio/wav" | "audio/x-wav" => Some(rig::message::AudioMediaType::WAV),
        "audio/mpeg" | "audio/mp3" => Some(rig::message::AudioMediaType::MP3),
        "audio/aiff" => Some(rig::message::AudioMediaType::AIFF),
        "audio/aac" | "audio/mp4" | "audio/m4a" => Some(rig::message::AudioMediaType::AAC),
        "audio/ogg" | "audio/opus" => Some(rig::message::AudioMediaType::OGG),
        "audio/flac" => Some(rig::message::AudioMediaType::FLAC),
        _ => None,
    }
}

fn mime_to_rig_video_type(mime: &str) -> Option<rig::message::VideoMediaType> {
    match mime.to_ascii_lowercase().as_str() {
        "video/x-msvideo" => Some(rig::message::VideoMediaType::AVI),
        "video/mp4" => Some(rig::message::VideoMediaType::MP4),
        "video/mpeg" => Some(rig::message::VideoMediaType::MPEG),
        _ => None,
    }
}

/// Responses-style providers require a non-empty tool call ID.
fn normalized_tool_call_id(raw: Option<&str>, seed: usize) -> String {
    match raw.map(str::trim).filter(|id| !id.is_empty()) {
        Some(id) => id.to_string(),
        None => format!("generated_tool_call_{seed}"),
    }
}

/// Convert ThinClaw tool definitions to rig-core format.
///
/// Applies OpenAI strict-mode schema normalization to ensure all tool
/// parameter schemas comply with OpenAI's function calling requirements.
fn convert_tools(tools: &[IronToolDefinition]) -> Vec<RigToolDefinition> {
    tools
        .iter()
        .map(|t| RigToolDefinition {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: normalize_schema_strict(&t.parameters),
        })
        .collect()
}

/// Convert ThinClaw tool_choice string to rig-core ToolChoice.
fn convert_tool_choice(choice: Option<&str>) -> Option<RigToolChoice> {
    match choice.map(|s| s.to_lowercase()).as_deref() {
        Some("auto") => Some(RigToolChoice::Auto),
        Some("required") => Some(RigToolChoice::Required),
        Some("none") => Some(RigToolChoice::None),
        _ => None,
    }
}

/// Extract text, tool calls, thinking content, and finish reason from a rig-core response.
fn extract_response(
    choice: &OneOrMany<AssistantContent>,
    _usage: &RigUsage,
) -> (
    Option<String>,
    Vec<IronToolCall>,
    Option<String>,
    FinishReason,
) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<IronToolCall> = Vec::new();
    let mut thinking_parts: Vec<String> = Vec::new();

    for content in choice.iter() {
        match content {
            AssistantContent::Text(t) if !t.text.is_empty() => {
                text_parts.push(t.text.clone());
            }
            AssistantContent::ToolCall(tc) => {
                let safe_id = if tc.id.trim().is_empty() {
                    format!("call_{}", uuid::Uuid::new_v4().simple())
                } else {
                    tc.id.clone()
                };
                tool_calls.push(IronToolCall {
                    id: safe_id,
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                });
            }
            AssistantContent::Reasoning(reasoning) => {
                // Capture extended thinking / reasoning content. Encrypted
                // and redacted blocks are opaque payloads, not display text.
                for part in &reasoning.content {
                    let text = match part {
                        ReasoningContent::Text { text, .. } => text,
                        ReasoningContent::Summary(text) => text,
                        _ => continue,
                    };
                    if !text.is_empty() {
                        thinking_parts.push(text.clone());
                    }
                }
            }
            _ => {}
        }
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let thinking = if thinking_parts.is_empty() {
        None
    } else {
        Some(thinking_parts.join("\n"))
    };

    let finish = finish_reason_for_tool_use(!tool_calls.is_empty());

    (text, tool_calls, thinking, finish)
}

/// Saturate u64 to u32 for token counts.
fn saturate_u32(val: u64) -> u32 {
    val.min(u32::MAX as u64) as u32
}

/// Derive the terminal finish reason for a completion from whether any tool
/// call was produced. Shared by the non-streaming and streaming paths so the
/// two agree: a turn that emitted any tool call ends with
/// [`FinishReason::ToolUse`], otherwise [`FinishReason::Stop`].
fn finish_reason_for_tool_use(saw_tool_call: bool) -> FinishReason {
    if saw_tool_call {
        FinishReason::ToolUse
    } else {
        FinishReason::Stop
    }
}

/// Build a rig-core CompletionRequest from our internal types.
#[allow(clippy::too_many_arguments)]
fn build_rig_request(
    preamble: Option<String>,
    mut history: Vec<RigMessage>,
    context_documents: Vec<String>,
    tools: Vec<RigToolDefinition>,
    tool_choice: Option<RigToolChoice>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    additional_params: Option<JsonValue>,
) -> Result<RigRequest, LlmError> {
    // rig-core requires at least one message in chat_history
    if history.is_empty() {
        history.push(RigMessage::user("Hello"));
    }

    let chat_history = OneOrMany::many(history).map_err(|e| LlmError::RequestFailed {
        provider: "rig".to_string(),
        reason: format!("Failed to build chat history: {}", e),
    })?;
    let preamble = merge_ephemeral_context_into_preamble(preamble, context_documents);

    Ok(RigRequest {
        // The target model is carried by the provider client we build the
        // request against, matching pre-0.33 behavior where the request had
        // no model field.
        model: None,
        output_schema: None,
        preamble,
        chat_history,
        documents: Vec::new(),
        tools,
        temperature: temperature.map(|t| t as f64),
        max_tokens: max_tokens.map(|t| t as u64),
        tool_choice,
        additional_params,
    })
}

fn merge_ephemeral_context_into_preamble(
    preamble: Option<String>,
    context_documents: Vec<String>,
) -> Option<String> {
    let ephemeral_context = context_documents
        .into_iter()
        .filter_map(|text| {
            let text = text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(text)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    match (
        preamble
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        ephemeral_context.is_empty(),
    ) {
        (Some(preamble), true) => Some(preamble),
        (Some(preamble), false) => Some(format!("{preamble}\n\n{ephemeral_context}")),
        (None, false) => Some(ephemeral_context),
        (None, true) => None,
    }
}

/// Convert a ThinkingConfig into provider-specific additional_params JSON.
///
/// For Anthropic: `{ "thinking": { "type": "enabled", "budget_tokens": N } }`
/// This is passed through rig-core's `additional_params` on the completion request,
/// and the Anthropic provider implementation in rig maps it to the API correctly.
fn thinking_config_to_params(config: &ThinkingConfig) -> Option<JsonValue> {
    match config {
        ThinkingConfig::Disabled => None,
        ThinkingConfig::Enabled { budget_tokens } => Some(serde_json::json!({
            "thinking": {
                "type": "enabled",
                "budget_tokens": budget_tokens
            }
        })),
    }
}

fn merge_additional_params(base: Option<JsonValue>, extra: Option<JsonValue>) -> Option<JsonValue> {
    match (base, extra) {
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => Some(value),
        (Some(JsonValue::Object(mut base)), Some(JsonValue::Object(extra))) => {
            for (key, value) in extra {
                base.insert(key, value);
            }
            Some(JsonValue::Object(base))
        }
        (Some(base), Some(extra)) => Some(serde_json::json!({
            "base": base,
            "token_capture": extra,
        })),
    }
}

fn json_to_u32(value: &JsonValue) -> Option<u32> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .or_else(|| value.as_i64().and_then(|n| u32::try_from(n).ok()))
}

fn json_to_f32(value: &JsonValue) -> Option<f32> {
    value.as_f64().filter(|v| v.is_finite()).map(|v| v as f32)
}

fn json_to_f64(value: &JsonValue) -> Option<f64> {
    value.as_f64().filter(|v| v.is_finite())
}

fn nested<'a>(value: &'a JsonValue, path: &[&str]) -> Option<&'a JsonValue> {
    let mut cur = value;
    for part in path {
        cur = if let Ok(index) = part.parse::<usize>() {
            cur.get(index)?
        } else {
            cur.get(*part)?
        };
    }
    Some(cur)
}

fn extract_provider_cost_usd_from_raw(raw: &JsonValue) -> Option<f64> {
    let candidates = [
        nested(raw, &["usage", "cost"]),
        nested(raw, &["usage", "cost_usd"]),
        nested(raw, &["usage", "total_cost"]),
        nested(raw, &["usage", "total_cost_usd"]),
        nested(raw, &["usage", "totalCost"]),
        nested(raw, &["usage", "totalCostUsd"]),
        raw.get("cost"),
        raw.get("cost_usd"),
        raw.get("total_cost"),
        raw.get("total_cost_usd"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(json_to_f64)
        .filter(|cost| *cost >= 0.0)
}

fn extract_openai_logprobs(raw: &JsonValue) -> Option<(Vec<u32>, Vec<String>, Vec<f32>)> {
    let logprobs = nested(raw, &["choices", "0", "logprobs"]).or_else(|| raw.get("logprobs"))?;

    if let Some(content) = logprobs.get("content").and_then(|v| v.as_array()) {
        let mut token_ids = Vec::new();
        let mut tokens = Vec::new();
        let mut logprobs_out = Vec::new();
        for item in content {
            if let Some(token) = item.get("token").and_then(|v| v.as_str()) {
                tokens.push(token.to_string());
            }
            if let Some(id) = item
                .get("token_id")
                .or_else(|| item.get("tokenId"))
                .or_else(|| item.get("id"))
                .and_then(json_to_u32)
            {
                token_ids.push(id);
            }
            if let Some(logprob) = item
                .get("logprob")
                .or_else(|| item.get("log_probability"))
                .or_else(|| item.get("logProbability"))
                .and_then(json_to_f32)
            {
                logprobs_out.push(logprob);
            }
        }
        if !tokens.is_empty() || !logprobs_out.is_empty() || !token_ids.is_empty() {
            return Some((token_ids, tokens, logprobs_out));
        }
    }

    let tokens = logprobs
        .get("tokens")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let logprobs_out = logprobs
        .get("token_logprobs")
        .or_else(|| logprobs.get("logprobs"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(json_to_f32).collect::<Vec<_>>())
        .unwrap_or_default();
    let token_ids = logprobs
        .get("token_ids")
        .or_else(|| logprobs.get("tokenIds"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(json_to_u32).collect::<Vec<_>>())
        .unwrap_or_default();
    if !tokens.is_empty() || !logprobs_out.is_empty() || !token_ids.is_empty() {
        return Some((token_ids, tokens, logprobs_out));
    }

    None
}

fn extract_gemini_logprobs(raw: &JsonValue) -> Option<(Vec<u32>, Vec<String>, Vec<f32>)> {
    let candidates = raw.get("candidates").and_then(|v| v.as_array())?;
    let first = candidates.first()?;
    let chosen = nested(first, &["logprobsResult", "chosenCandidates"])
        .or_else(|| nested(first, &["logprobs_result", "chosen_candidates"]))?
        .as_array()?;

    let mut tokens = Vec::new();
    let mut logprobs_out = Vec::new();
    for item in chosen {
        if let Some(token) = item.get("token").and_then(|v| v.as_str()) {
            tokens.push(token.to_string());
        }
        if let Some(logprob) = item
            .get("logProbability")
            .or_else(|| item.get("log_probability"))
            .or_else(|| item.get("logprob"))
            .and_then(json_to_f32)
        {
            logprobs_out.push(logprob);
        }
    }
    if !tokens.is_empty() || !logprobs_out.is_empty() {
        return Some((Vec::new(), tokens, logprobs_out));
    }
    None
}

fn extract_local_token_array(raw: &JsonValue) -> Option<(Vec<u32>, Vec<String>, Vec<f32>)> {
    let arrays = [
        raw.get("tokens"),
        nested(raw, &["response", "tokens"]),
        nested(raw, &["completion", "tokens"]),
        nested(raw, &["choices", "0", "tokens"]),
    ];
    for maybe_array in arrays.into_iter().flatten() {
        let Some(array) = maybe_array.as_array() else {
            continue;
        };
        let mut token_ids = Vec::new();
        let mut tokens = Vec::new();
        let mut logprobs_out = Vec::new();
        for item in array {
            if let Some(id) = item
                .get("id")
                .or_else(|| item.get("token_id"))
                .or_else(|| item.get("tokenId"))
                .and_then(json_to_u32)
            {
                token_ids.push(id);
            }
            if let Some(token) = item
                .get("text")
                .or_else(|| item.get("token"))
                .or_else(|| item.get("piece"))
                .and_then(|v| v.as_str())
            {
                tokens.push(token.to_string());
            }
            if let Some(logprob) = item
                .get("logprob")
                .or_else(|| item.get("log_probability"))
                .or_else(|| item.get("logProbability"))
                .and_then(json_to_f32)
            {
                logprobs_out.push(logprob);
            }
        }
        if !token_ids.is_empty() || !tokens.is_empty() || !logprobs_out.is_empty() {
            return Some((token_ids, tokens, logprobs_out));
        }
    }
    None
}

fn extract_provider_token_capture_from_raw(
    raw: &JsonValue,
    provider: impl Into<String>,
    model: impl Into<String>,
) -> Option<ProviderTokenCapture> {
    let (token_ids, tokens, logprobs) = extract_openai_logprobs(raw)
        .or_else(|| extract_gemini_logprobs(raw))
        .or_else(|| extract_local_token_array(raw))?;

    let exact_tokens_supported = !tokens.is_empty() || !token_ids.is_empty();
    let logprobs_supported = !logprobs.is_empty();
    if !exact_tokens_supported && !logprobs_supported {
        return None;
    }

    Some(ProviderTokenCapture {
        exact_tokens_supported,
        logprobs_supported,
        token_ids,
        tokens,
        logprobs,
        provider: Some(provider.into()),
        model: Some(model.into()),
    })
}

#[async_trait]
impl<M> LlmProvider for RigAdapter<M>
where
    M: CompletionModel + Send + Sync + 'static,
    M::Response: Send + Sync + Serialize + DeserializeOwned,
{
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (self.input_cost, self.output_cost)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        if let Some(requested_model) = request.model.as_deref()
            && !requested_model_matches_active_model(requested_model, self.model_name.as_str())
        {
            tracing::warn!(
                requested_model = requested_model,
                active_model = %self.model_name,
                "Per-request model override is not supported for this provider; using configured model"
            );
        }

        let mut messages = request.messages;
        thinclaw_llm_core::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history, cache_hint_requested) = convert_messages(&messages);
        if cache_hint_requested && !self.prompt_caching {
            tracing::debug!(
                model = %self.model_name,
                "System message requested prompt caching metadata, but provider-side prompt caching is disabled"
            );
        }
        let thinking_params = thinking_config_to_params(&request.thinking);
        let capture_params = self.token_capture_params.clone();
        let additional_params =
            merge_additional_params(thinking_params.clone(), capture_params.clone());

        if additional_params.is_some() {
            tracing::info!(
                model = %self.model_name,
                "Extended thinking enabled for completion request"
            );
        }

        let context_documents = request.context_documents;
        let rig_req = build_rig_request(
            preamble.clone(),
            history.clone(),
            context_documents.clone(),
            Vec::new(),
            None,
            request.temperature,
            request.max_tokens,
            additional_params,
        )?;

        let response = match self.model.completion(rig_req).await {
            Ok(response) => response,
            Err(error) if capture_params.is_some() => {
                tracing::warn!(
                    provider = %self.provider_label,
                    model = %self.model_name,
                    error = %error,
                    "Provider rejected token/logprob capture parameters; retrying without them"
                );
                let retry_req = build_rig_request(
                    preamble,
                    history,
                    context_documents,
                    Vec::new(),
                    None,
                    request.temperature,
                    request.max_tokens,
                    thinking_params,
                )?;
                self.model
                    .completion(retry_req)
                    .await
                    .map_err(|retry_error| LlmError::RequestFailed {
                        provider: self.model_name.clone(),
                        reason: retry_error.to_string(),
                    })?
            }
            Err(error) => {
                return Err(LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: error.to_string(),
                });
            }
        };

        let (text, _tool_calls, thinking, finish) =
            extract_response(&response.choice, &response.usage);
        let input_tokens = saturate_u32(response.usage.input_tokens);
        let output_tokens = saturate_u32(response.usage.output_tokens);
        let raw_response = serde_json::to_value(&response.raw_response).ok();
        let token_capture = raw_response.as_ref().and_then(|raw| {
            extract_provider_token_capture_from_raw(
                raw,
                self.provider_label.clone(),
                self.model_name.clone(),
            )
        });
        let cost_usd = raw_response
            .as_ref()
            .and_then(extract_provider_cost_usd_from_raw);

        Ok(CompletionResponse {
            content: text.unwrap_or_default(),
            provider_model: Some(self.model_name.clone()),
            cost_usd,
            thinking_content: thinking,
            input_tokens,
            output_tokens,
            finish_reason: finish,
            token_capture,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        if let Some(requested_model) = request.model.as_deref()
            && !requested_model_matches_active_model(requested_model, self.model_name.as_str())
        {
            tracing::warn!(
                requested_model = requested_model,
                active_model = %self.model_name,
                "Per-request model override is not supported for this provider; using configured model"
            );
        }

        let known_tool_names: HashSet<String> =
            request.tools.iter().map(|t| t.name.clone()).collect();

        let mut messages = request.messages;
        thinclaw_llm_core::provider::sanitize_tool_messages(&mut messages);

        // ── Diagnostic: dump message roles BEFORE conversion ──────────
        {
            let summary: Vec<String> = messages
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let tc_info = m
                        .tool_calls
                        .as_ref()
                        .map(|tcs| {
                            tcs.iter()
                                .map(|tc| tc.id.clone())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default();
                    let tcid = m.tool_call_id.as_deref().unwrap_or("");
                    format!(
                        "[{}] {:?} name={} tc_ids=[{}] tool_call_id={}",
                        i,
                        m.role,
                        m.name.as_deref().unwrap_or("-"),
                        tc_info,
                        tcid,
                    )
                })
                .collect();
            tracing::info!(
                msg_count = messages.len(),
                "complete_with_tools: post-sanitize message dump:\n{}",
                summary.join("\n")
            );
        }

        let (preamble, history, cache_hint_requested) = convert_messages(&messages);
        if cache_hint_requested && !self.prompt_caching {
            tracing::debug!(
                model = %self.model_name,
                "System message requested prompt caching metadata, but provider-side prompt caching is disabled"
            );
        }
        let tools = convert_tools(&request.tools);
        let tool_choice = convert_tool_choice(request.tool_choice.as_deref());
        let thinking_params = thinking_config_to_params(&request.thinking);
        let capture_params = self.token_capture_params.clone();
        let additional_params =
            merge_additional_params(thinking_params.clone(), capture_params.clone());

        if additional_params.is_some() {
            tracing::info!(
                model = %self.model_name,
                tools = known_tool_names.len(),
                "Extended thinking enabled for tool completion request"
            );
        }

        let context_documents = request.context_documents;
        let rig_req = build_rig_request(
            preamble.clone(),
            history.clone(),
            context_documents.clone(),
            tools.clone(),
            tool_choice.clone(),
            request.temperature,
            request.max_tokens,
            additional_params,
        )?;

        let response = match self.model.completion(rig_req).await {
            Ok(response) => response,
            Err(error) if capture_params.is_some() => {
                tracing::warn!(
                    provider = %self.provider_label,
                    model = %self.model_name,
                    error = %error,
                    "Provider rejected token/logprob capture parameters for tool completion; retrying without them"
                );
                let retry_req = build_rig_request(
                    preamble,
                    history,
                    context_documents,
                    tools,
                    tool_choice,
                    request.temperature,
                    request.max_tokens,
                    thinking_params,
                )?;
                self.model
                    .completion(retry_req)
                    .await
                    .map_err(|retry_error| LlmError::RequestFailed {
                        provider: self.model_name.clone(),
                        reason: retry_error.to_string(),
                    })?
            }
            Err(error) => {
                return Err(LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: error.to_string(),
                });
            }
        };

        let (text, mut tool_calls, thinking, finish) =
            extract_response(&response.choice, &response.usage);

        // Normalize tool call names: some proxies prepend "proxy_" prefixes.
        for tc in &mut tool_calls {
            let normalized = normalize_tool_name(&tc.name, &known_tool_names);
            if normalized != tc.name {
                tracing::debug!(
                    original = %tc.name,
                    normalized = %normalized,
                    "Normalized tool call name from provider",
                );
                tc.name = normalized;
            }
        }

        let input_tokens = saturate_u32(response.usage.input_tokens);
        let output_tokens = saturate_u32(response.usage.output_tokens);
        let raw_response = serde_json::to_value(&response.raw_response).ok();
        let token_capture = raw_response.as_ref().and_then(|raw| {
            extract_provider_token_capture_from_raw(
                raw,
                self.provider_label.clone(),
                self.model_name.clone(),
            )
        });
        let cost_usd = raw_response
            .as_ref()
            .and_then(extract_provider_cost_usd_from_raw);

        Ok(ToolCompletionResponse {
            content: text,
            provider_model: Some(self.model_name.clone()),
            cost_usd,
            tool_calls,
            thinking_content: thinking,
            input_tokens,
            output_tokens,
            finish_reason: finish,
            token_capture,
        })
    }

    fn active_model_name(&self) -> String {
        self.model_name.clone()
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.active_model_name()
    }

    fn supports_prompt_caching(&self) -> bool {
        self.prompt_caching
    }

    fn set_model(&self, _model: &str) -> Result<(), LlmError> {
        // rig-core models are baked at construction time.
        // Switching requires creating a new adapter.
        Err(LlmError::RequestFailed {
            provider: self.model_name.clone(),
            reason: "Runtime model switching not supported for rig-core providers. \
                     Restart with a different model configured."
                .to_string(),
        })
    }

    fn stream_support(&self) -> StreamSupport {
        self.stream_support
    }

    fn token_capture_support(&self) -> TokenCaptureSupport {
        self.token_capture_support
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        if self.stream_support != StreamSupport::Native {
            return match self.stream_support {
                StreamSupport::Simulated
                    if request.stream_policy != StreamPolicy::RequireNative =>
                {
                    self.simulated_stream_from_completion(request).await
                }
                _ => Err(self.streaming_policy_error()),
            };
        }

        let mut messages = request.messages;
        thinclaw_llm_core::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history, cache_hint_requested) = convert_messages(&messages);
        if cache_hint_requested && !self.prompt_caching {
            tracing::debug!(
                model = %self.model_name,
                "System message requested prompt caching metadata, but provider-side prompt caching is disabled"
            );
        }
        let thinking_params = thinking_config_to_params(&request.thinking);
        let capture_params = self.token_capture_params.clone();
        let additional_params =
            merge_additional_params(thinking_params.clone(), capture_params.clone());

        let context_documents = request.context_documents;
        let rig_req = build_rig_request(
            preamble.clone(),
            history.clone(),
            context_documents.clone(),
            Vec::new(),
            None,
            request.temperature,
            request.max_tokens,
            additional_params,
        )?;

        let streaming_resp = match self.model.stream(rig_req).await {
            Ok(response) => response,
            Err(error) if capture_params.is_some() => {
                tracing::warn!(
                    provider = %self.provider_label,
                    model = %self.model_name,
                    error = %error,
                    "Provider rejected streaming token/logprob capture parameters; retrying without them"
                );
                let retry_req = build_rig_request(
                    preamble,
                    history,
                    context_documents,
                    Vec::new(),
                    None,
                    request.temperature,
                    request.max_tokens,
                    thinking_params,
                )?;
                self.model.stream(retry_req).await.map_err(|retry_error| {
                    LlmError::RequestFailed {
                        provider: self.model_name.clone(),
                        reason: retry_error.to_string(),
                    }
                })?
            }
            Err(error) => {
                return Err(LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: error.to_string(),
                });
            }
        };

        Ok(rig_stream_to_chunks(
            streaming_resp,
            self.model_name.clone(),
            self.input_cost,
            self.output_cost,
            self.token_capture_support,
            self.provider_label.clone(),
        ))
    }

    async fn complete_stream_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<StreamChunkStream, LlmError> {
        if self.stream_support != StreamSupport::Native {
            return match self.stream_support {
                StreamSupport::Simulated
                    if request.stream_policy != StreamPolicy::RequireNative =>
                {
                    self.simulated_stream_from_tool_completion(request).await
                }
                _ => Err(self.streaming_policy_error()),
            };
        }

        let known_tool_names: HashSet<String> =
            request.tools.iter().map(|t| t.name.clone()).collect();

        let mut messages = request.messages;
        thinclaw_llm_core::provider::sanitize_tool_messages(&mut messages);

        // ── Diagnostic: dump message roles BEFORE conversion ──────────
        // Enables root-cause analysis of "messages with role tool must follow
        // tool_calls" 400 errors. Remove once the protocol bug is fixed.
        {
            let summary: Vec<String> = messages
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let tc_info = m
                        .tool_calls
                        .as_ref()
                        .map(|tcs| {
                            tcs.iter()
                                .map(|tc| tc.id.clone())
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default();
                    let tcid = m.tool_call_id.as_deref().unwrap_or("");
                    format!(
                        "[{}] {:?} name={} tc_ids=[{}] tool_call_id={}",
                        i,
                        m.role,
                        m.name.as_deref().unwrap_or("-"),
                        tc_info,
                        tcid,
                    )
                })
                .collect();
            tracing::info!(
                msg_count = messages.len(),
                "complete_stream_with_tools: post-sanitize message dump:\n{}",
                summary.join("\n")
            );
        }

        let (preamble, history, cache_hint_requested) = convert_messages(&messages);
        if cache_hint_requested && !self.prompt_caching {
            tracing::debug!(
                model = %self.model_name,
                "System message requested prompt caching metadata, but provider-side prompt caching is disabled"
            );
        }
        let tools = convert_tools(&request.tools);
        let tool_choice = convert_tool_choice(request.tool_choice.as_deref());
        let thinking_params = thinking_config_to_params(&request.thinking);
        let capture_params = self.token_capture_params.clone();
        let additional_params =
            merge_additional_params(thinking_params.clone(), capture_params.clone());

        let context_documents = request.context_documents;
        let rig_req = build_rig_request(
            preamble.clone(),
            history.clone(),
            context_documents.clone(),
            tools.clone(),
            tool_choice.clone(),
            request.temperature,
            request.max_tokens,
            additional_params,
        )?;

        let streaming_resp = match self.model.stream(rig_req).await {
            Ok(response) => response,
            Err(error) if capture_params.is_some() => {
                tracing::warn!(
                    provider = %self.provider_label,
                    model = %self.model_name,
                    error = %error,
                    "Provider rejected streaming token/logprob capture parameters for tool completion; retrying without them"
                );
                let retry_req = build_rig_request(
                    preamble,
                    history,
                    context_documents,
                    tools,
                    tool_choice,
                    request.temperature,
                    request.max_tokens,
                    thinking_params,
                )?;
                self.model.stream(retry_req).await.map_err(|retry_error| {
                    LlmError::RequestFailed {
                        provider: self.model_name.clone(),
                        reason: retry_error.to_string(),
                    }
                })?
            }
            Err(error) => {
                return Err(LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: error.to_string(),
                });
            }
        };

        Ok(rig_stream_to_chunks_with_normalization(
            streaming_resp,
            self.model_name.clone(),
            self.input_cost,
            self.output_cost,
            known_tool_names,
            self.token_capture_support,
            self.provider_label.clone(),
        ))
    }
}

/// Convert a rig-core StreamingCompletionResponse into our StreamChunkStream.
fn rig_stream_to_chunks<R>(
    streaming_resp: rig::streaming::StreamingCompletionResponse<R>,
    provider_model: String,
    input_cost: Decimal,
    output_cost: Decimal,
    token_capture_support: TokenCaptureSupport,
    provider_label: String,
) -> StreamChunkStream
where
    R: Clone
        + Unpin
        + Send
        + Sync
        + 'static
        + serde::Serialize
        + serde::de::DeserializeOwned
        + rig::completion::GetTokenUsage,
{
    rig_stream_to_chunks_with_normalization(
        streaming_resp,
        provider_model,
        input_cost,
        output_cost,
        HashSet::new(),
        token_capture_support,
        provider_label,
    )
}

/// Convert a rig-core StreamingCompletionResponse into our StreamChunkStream,
/// normalizing tool call names against known tools.
fn rig_stream_to_chunks_with_normalization<R>(
    mut streaming_resp: rig::streaming::StreamingCompletionResponse<R>,
    provider_model: String,
    input_cost: Decimal,
    output_cost: Decimal,
    known_tool_names: HashSet<String>,
    _token_capture_support: TokenCaptureSupport,
    provider_label: String,
) -> StreamChunkStream
where
    R: Clone
        + Unpin
        + Send
        + Sync
        + 'static
        + serde::Serialize
        + serde::de::DeserializeOwned
        + rig::completion::GetTokenUsage,
{
    use futures::StreamExt;
    use rig::streaming::{StreamedAssistantContent, ToolCallDeltaContent};

    // Track tool call indices for ToolCallDelta events. Each distinct tool call
    // ID gets an incrementing index so the downstream accumulator can separate
    // parallel tool calls correctly.
    let mut tc_id_to_index: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    let mut next_tc_index: u32 = 0;
    // Track whether any tool-call activity was seen so the terminal `Done`
    // chunk reports the real finish reason. Mirrors the non-streaming
    // derivation in `rig_message_to_completion_response` (tool calls present
    // => `FinishReason::ToolUse`, else `Stop`).
    let mut saw_tool_call = false;

    let stream = async_stream::stream! {
        while let Some(chunk_result) = streaming_resp.next().await {
            match chunk_result {
                Ok(StreamedAssistantContent::Text(text)) => {
                    if !text.text.is_empty() {
                        yield Ok(StreamChunk::Text(text.text));
                    }
                }
                Ok(StreamedAssistantContent::Reasoning(reasoning)) => {
                    let combined: String = reasoning
                        .content
                        .iter()
                        .filter_map(|part| match part {
                            rig::message::ReasoningContent::Text { text, .. } => {
                                Some(text.as_str())
                            }
                            rig::message::ReasoningContent::Summary(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    if !combined.is_empty() {
                        yield Ok(StreamChunk::ReasoningDelta(combined));
                    }
                }
                Ok(StreamedAssistantContent::ReasoningDelta { reasoning, .. }) => {
                    if !reasoning.is_empty() {
                        yield Ok(StreamChunk::ReasoningDelta(reasoning));
                    }
                }
                Ok(StreamedAssistantContent::ToolCall { tool_call, .. }) => {
                    saw_tool_call = true;
                    let mut name = tool_call.function.name;
                    if !known_tool_names.is_empty() {
                        name = normalize_tool_name(&name, &known_tool_names);
                    }
                    let safe_id = if tool_call.id.trim().is_empty() {
                        format!("call_{}", uuid::Uuid::new_v4().simple())
                    } else {
                        tool_call.id
                    };
                    yield Ok(StreamChunk::ToolCall(IronToolCall {
                        id: safe_id,
                        name,
                        arguments: tool_call.function.arguments,
                    }));
                }
                Ok(StreamedAssistantContent::ToolCallDelta { id, content, .. }) => {
                    saw_tool_call = true;
                    let (name_delta, args_delta) = match content {
                        ToolCallDeltaContent::Name(n) => (Some(n), None),
                        ToolCallDeltaContent::Delta(d) => (None, Some(d)),
                    };
                    // Assign a stable index per tool call ID so the downstream
                    // accumulator can group deltas for parallel tool calls.
                    let index = if id.is_empty() {
                        // Empty ID on continuation deltas — use the last assigned index
                        next_tc_index.saturating_sub(1)
                    } else {
                        *tc_id_to_index.entry(id.clone()).or_insert_with(|| {
                            let idx = next_tc_index;
                            next_tc_index += 1;
                            idx
                        })
                    };
                    yield Ok(StreamChunk::ToolCallDelta {
                        index,
                        id,
                        name: name_delta,
                        arguments_delta: args_delta,
                    });
                }
                Ok(StreamedAssistantContent::Final(resp)) => {
                    // Extract usage if available
                    let usage = resp.token_usage().unwrap_or_default();
                    let input_tokens = saturate_u32(usage.input_tokens);
                    let output_tokens = saturate_u32(usage.output_tokens);
                    let token_capture = serde_json::to_value(&resp)
                        .ok()
                        .and_then(|raw| {
                            extract_provider_token_capture_from_raw(
                                &raw,
                                provider_label.clone(),
                                provider_model.clone(),
                            )
                        });
                    let cost_usd = {
                        use rust_decimal::prelude::ToPrimitive;
                        (input_cost * Decimal::from(input_tokens)
                            + output_cost * Decimal::from(output_tokens))
                            .to_f64()
                            .unwrap_or(0.0)
                    };
                    yield Ok(StreamChunk::Done {
                        provider_model: Some(provider_model.clone()),
                        cost_usd: Some(cost_usd),
                        input_tokens,
                        output_tokens,
                        finish_reason: finish_reason_for_tool_use(saw_tool_call),
                        token_capture,
                    });
                }
                Err(e) => {
                    if e.to_string().contains("aborted") {
                        // Stream was cancelled
                        break;
                    }
                    yield Err(LlmError::RequestFailed {
                        provider: "rig-stream".to_string(),
                        reason: e.to_string(),
                    });
                    break;
                }
            }
        }
    };

    Box::pin(stream)
}

#[cfg(test)]
mod tests;
