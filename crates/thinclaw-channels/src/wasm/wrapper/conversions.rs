//! WIT type conversions, message normalization, and `HttpResponse`.
//!
//! Bridges between the generated WIT component-model types
//! ([`wit_channel`](super::wit_channel)) and the host-side channel types,
//! normalizes WASM-emitted messages into [`IncomingMessage`]s, and owns the
//! public [`HttpResponse`] type returned by webhook callbacks.

use std::collections::HashMap;

use crate::manager::{IncomingEvent, normalize_incoming_event, parse_slash_command};
use crate::wasm::host::EmittedMessage;
use crate::wasm::schema::ChannelConfig;
use thinclaw_channels_core::{IncomingMessage, OutgoingResponse, StatusUpdate};

use super::wit_channel;

/// Convert WIT-generated ChannelConfig to our internal type.
pub(super) fn convert_channel_config(wit: wit_channel::ChannelConfig) -> ChannelConfig {
    ChannelConfig {
        display_name: wit.display_name,
        http_endpoints: wit
            .http_endpoints
            .into_iter()
            .map(|ep| crate::wasm::schema::HttpEndpointConfigSchema {
                path: ep.path,
                methods: ep.methods,
                require_secret: ep.require_secret,
            })
            .collect(),
        poll: wit.poll.map(|p| crate::wasm::schema::PollConfigSchema {
            interval_ms: p.interval_ms,
            enabled: p.enabled,
        }),
    }
}

/// Convert WIT-generated OutgoingHttpResponse to our HttpResponse type.
pub(super) fn convert_http_response(wit: wit_channel::OutgoingHttpResponse) -> HttpResponse {
    let headers = serde_json::from_str(&wit.headers_json).unwrap_or_default();
    HttpResponse {
        status: wit.status,
        headers,
        body: wit.body,
    }
}

/// Convert a StatusUpdate + metadata into the WIT StatusUpdate type.
fn truncate_status_text(input: &str, max_chars: usize) -> String {
    let mut iter = input.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

pub(super) fn status_to_wit(
    status: &StatusUpdate,
    metadata: &serde_json::Value,
) -> wit_channel::StatusUpdate {
    let metadata_json = serde_json::to_string(metadata).unwrap_or_default();

    match status {
        StatusUpdate::Thinking(msg) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: msg.clone(),
            metadata_json,
        },
        StatusUpdate::ToolStarted { name, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolStarted,
            message: format!("Tool started: {}", name),
            metadata_json,
        },
        StatusUpdate::ToolCompleted { name, success, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolCompleted,
            message: format!(
                "Tool completed: {} ({})",
                name,
                if *success { "ok" } else { "failed" }
            ),
            metadata_json,
        },
        StatusUpdate::ToolResult { name, preview, .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ToolResult,
            message: format!(
                "Tool result: {}\n{}",
                name,
                truncate_status_text(preview, 280)
            ),
            metadata_json,
        },
        StatusUpdate::StreamChunk(chunk) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: chunk.clone(),
            metadata_json,
        },
        StatusUpdate::Status(msg) => {
            // Map well-known status strings to WIT types (case-insensitive
            // to stay consistent with is_terminal_text_status and the
            // Telegram-side classify_status_update).
            let trimmed = msg.trim();
            let status_type = if trimmed.eq_ignore_ascii_case("done") {
                wit_channel::StatusType::Done
            } else if trimmed.eq_ignore_ascii_case("interrupted") {
                wit_channel::StatusType::Interrupted
            } else {
                wit_channel::StatusType::Status
            };
            wit_channel::StatusUpdate {
                status: status_type,
                message: msg.clone(),
                metadata_json,
            }
        }
        StatusUpdate::Plan { entries } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[plan] {}",
                serde_json::to_string(entries).unwrap_or_default()
            ),
            metadata_json,
        },
        StatusUpdate::ContextCompactionStarted { used, limit } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!("Compacting context ({used}/{limit} tokens) and retrying"),
            metadata_json,
        },
        StatusUpdate::AdvisorConsultationStarted { .. } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: "Consulting the advisor lane".to_string(),
            metadata_json,
        },
        StatusUpdate::SelfRepairStarted {
            repair_type,
            target_id,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!("Self-repair: {repair_type} {target_id}"),
            metadata_json,
        },
        StatusUpdate::SelfRepairCompleted {
            repair_type,
            target_id,
            success,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "Self-repair {}: {repair_type} {target_id}",
                if *success { "succeeded" } else { "failed" }
            ),
            metadata_json,
        },
        StatusUpdate::Usage {
            input_tokens,
            output_tokens,
            cost_usd,
            model,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[usage] {} input + {} output tokens{}{}",
                input_tokens,
                output_tokens,
                cost_usd
                    .map(|cost| format!(", ${cost:.6}"))
                    .unwrap_or_default(),
                model
                    .as_deref()
                    .map(|model| format!(" ({model})"))
                    .unwrap_or_default()
            ),
            metadata_json,
        },
        StatusUpdate::ApprovalNeeded {
            request_id,
            tool_name,
            description,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::ApprovalNeeded,
            message: format!(
                "Approval needed for tool '{}'. {}\nRequest ID: {}\nReply with: yes (or /approve), no (or /deny), or always (or /always).",
                tool_name, description, request_id
            ),
            metadata_json,
        },
        StatusUpdate::JobStarted {
            job_id,
            title,
            browse_url,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::JobStarted,
            message: format!("Job started: {} ({})\n{}", title, job_id, browse_url),
            metadata_json,
        },
        StatusUpdate::AuthRequired {
            extension_name,
            instructions,
            auth_url,
            setup_url,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthRequired,
            message: {
                let mut lines = vec![format!("Authentication required for {}.", extension_name)];
                if let Some(text) = instructions
                    && !text.trim().is_empty()
                {
                    lines.push(text.trim().to_string());
                }
                if let Some(url) = auth_url {
                    lines.push(format!("Auth URL: {}", url));
                }
                if let Some(url) = setup_url {
                    lines.push(format!("Setup URL: {}", url));
                }
                lines.join("\n")
            },
            metadata_json,
        },
        StatusUpdate::AuthCompleted {
            extension_name,
            success,
            message,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::AuthCompleted,
            message: format!(
                "Authentication {} for {}. {}",
                if *success { "completed" } else { "failed" },
                extension_name,
                message
            ),
            metadata_json,
        },
        StatusUpdate::CredentialPrompt {
            secret_name,
            reason,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!("Credential needed ({secret_name}): {reason}"),
            metadata_json,
        },
        StatusUpdate::Error { message, code } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[error{}] {}",
                code.as_ref().map(|c| format!(": {c}")).unwrap_or_default(),
                message
            ),
            metadata_json,
        },
        StatusUpdate::CanvasAction(action) => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[canvas] {}",
                serde_json::to_string(action).unwrap_or_default()
            ),
            metadata_json,
        },
        StatusUpdate::AgentMessage {
            content,
            message_type,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!("[agent_message:{}] {}", message_type, content),
            metadata_json,
        },
        StatusUpdate::LifecycleStart { run_id } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Thinking,
            message: format!("lifecycle:start:{}", run_id),
            metadata_json,
        },
        StatusUpdate::LifecycleEnd { run_id, phase } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Done,
            message: format!("lifecycle:end:{}:{}", phase, run_id),
            metadata_json,
        },
        StatusUpdate::SubagentSpawned {
            agent_id,
            name,
            task,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[subagent:spawned:{}] {}",
                agent_id,
                serde_json::to_string(&serde_json::json!({
                    "name": name,
                    "task": task,
                }))
                .unwrap_or_else(|_| format!("{} - {}", name, task))
            ),
            metadata_json,
        },
        StatusUpdate::SubagentProgress {
            agent_id,
            message,
            category,
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[subagent:progress:{}:{}] {}",
                agent_id,
                category,
                serde_json::to_string(&serde_json::json!({
                    "message": message,
                }))
                .unwrap_or_else(|_| message.clone())
            ),
            metadata_json,
        },
        StatusUpdate::SubagentCompleted {
            agent_id,
            name,
            success,
            response,
            duration_ms,
            iterations,
            ..
        } => wit_channel::StatusUpdate {
            status: wit_channel::StatusType::Status,
            message: format!(
                "[subagent:{}:{}] {}",
                if *success { "completed" } else { "failed" },
                agent_id,
                serde_json::to_string(&serde_json::json!({
                    "name": name,
                    "success": success,
                    "response": response,
                    "duration_ms": duration_ms,
                    "iterations": iterations,
                }))
                .unwrap_or_else(|_| format!(
                    "{} ({:.1}s)",
                    name,
                    *duration_ms as f64 / 1000.0
                ))
            ),
            metadata_json,
        },
    }
}

/// Clone a WIT StatusUpdate (the generated type doesn't derive Clone).
pub(super) fn clone_wit_status_update(
    update: &wit_channel::StatusUpdate,
) -> wit_channel::StatusUpdate {
    wit_channel::StatusUpdate {
        status: match update.status {
            wit_channel::StatusType::Thinking => wit_channel::StatusType::Thinking,
            wit_channel::StatusType::Done => wit_channel::StatusType::Done,
            wit_channel::StatusType::Interrupted => wit_channel::StatusType::Interrupted,
            wit_channel::StatusType::ToolStarted => wit_channel::StatusType::ToolStarted,
            wit_channel::StatusType::ToolCompleted => wit_channel::StatusType::ToolCompleted,
            wit_channel::StatusType::ToolResult => wit_channel::StatusType::ToolResult,
            wit_channel::StatusType::ApprovalNeeded => wit_channel::StatusType::ApprovalNeeded,
            wit_channel::StatusType::Status => wit_channel::StatusType::Status,
            wit_channel::StatusType::JobStarted => wit_channel::StatusType::JobStarted,
            wit_channel::StatusType::AuthRequired => wit_channel::StatusType::AuthRequired,
            wit_channel::StatusType::AuthCompleted => wit_channel::StatusType::AuthCompleted,
        },
        message: update.message.clone(),
        metadata_json: update.metadata_json.clone(),
    }
}

pub(super) fn emitted_message_to_incoming_message(
    channel_name: &str,
    emitted: EmittedMessage,
) -> IncomingMessage {
    let parsed_metadata = serde_json::from_str::<serde_json::Value>(&emitted.metadata_json)
        .unwrap_or(serde_json::Value::Null);
    let legacy_thread_id = emitted.thread_id.clone();

    let mut msg = wasm_emitted_incoming_event(channel_name, &emitted, &parsed_metadata)
        .map(normalize_incoming_event)
        .unwrap_or_else(|| {
            let mut msg = IncomingMessage::new(channel_name, &emitted.user_id, &emitted.content);
            if let Some(thread_id) = emitted.thread_id.clone() {
                msg = msg.with_thread(thread_id);
            }
            msg
        });

    if let Some(name) = emitted.user_name {
        msg = msg.with_user_name(name);
    }

    let mut metadata = metadata_object(&parsed_metadata, "package_metadata");
    for (key, value) in metadata_object(&msg.metadata, "normalized_metadata") {
        let collision_key = match key.as_str() {
            "chat_id" if metadata.contains_key("chat_id") => Some("canonical_chat_id"),
            "chat_type" if metadata.contains_key("chat_type") => Some("canonical_chat_type"),
            _ => None,
        };
        if let Some(collision_key) = collision_key {
            metadata.insert(collision_key.to_string(), value);
        } else {
            metadata.insert(key, value);
        }
    }
    if let Some(legacy_thread_id) = legacy_thread_id.as_deref() {
        add_legacy_thread_aliases(&mut metadata, channel_name, legacy_thread_id);
        metadata.insert(
            "package_thread_id".to_string(),
            serde_json::Value::String(legacy_thread_id.to_string()),
        );
    }
    if let Some(command) = parse_slash_command(&msg.content) {
        metadata.insert(
            "slash_command".to_string(),
            serde_json::json!({
                "command": command.command,
                "args": command.args,
            }),
        );
    }
    if !metadata.contains_key("conversation_kind")
        && let Some(chat_type) = metadata.get("chat_type").and_then(|value| value.as_str())
    {
        let conversation_kind = if chat_type == "dm" { "direct" } else { "group" };
        metadata.insert(
            "conversation_kind".to_string(),
            serde_json::Value::String(conversation_kind.to_string()),
        );
    }
    msg = msg.with_metadata(serde_json::Value::Object(metadata));

    for att in &emitted.attachments {
        msg.attachments.push(att.to_media_content());
    }

    msg
}

fn wasm_emitted_incoming_event(
    channel_name: &str,
    emitted: &EmittedMessage,
    metadata: &serde_json::Value,
) -> Option<IncomingEvent> {
    let chat_type = wasm_emitted_chat_type(channel_name, metadata);
    let chat_id = wasm_emitted_chat_id(channel_name, &chat_type, emitted, metadata)?;

    Some(IncomingEvent {
        platform: channel_name.to_string(),
        chat_type,
        chat_id,
        user_id: emitted.user_id.clone(),
        user_name: emitted.user_name.clone(),
        text: emitted.content.clone(),
        metadata: metadata.clone(),
    })
}

fn wasm_emitted_chat_type(channel_name: &str, metadata: &serde_json::Value) -> String {
    if let Some(chat_type) = metadata_string(metadata, "chat_type") {
        return normalize_chat_type(&chat_type);
    }
    if let Some(kind) = metadata_string(metadata, "conversation_kind") {
        return normalize_chat_type(&kind);
    }
    if let Some(is_private) = metadata_bool(metadata, "is_private") {
        return if is_private { "dm" } else { "group" }.to_string();
    }
    if metadata_bool(metadata, "is_group").unwrap_or(false) {
        return "group".to_string();
    }
    match channel_name {
        "whatsapp" => "dm".to_string(),
        "discord" | "slack" => "group".to_string(),
        _ => "chat".to_string(),
    }
}

fn normalize_chat_type(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "direct" | "private" | "dm" => "dm".to_string(),
        "group" | "supergroup" | "channel" | "room" => "group".to_string(),
        "" => "chat".to_string(),
        other => other.to_string(),
    }
}

fn wasm_emitted_chat_id(
    channel_name: &str,
    chat_type: &str,
    emitted: &EmittedMessage,
    metadata: &serde_json::Value,
) -> Option<String> {
    match channel_name {
        "telegram" => {
            let chat_id = metadata_string(metadata, "chat_id")?;
            if chat_type == "group"
                && let Some(thread_id) = metadata_string(metadata, "message_thread_id")
            {
                return Some(format!("{chat_id}:topic:{thread_id}"));
            }
            Some(chat_id)
        }
        "slack" => {
            let channel = metadata_string(metadata, "channel")
                .or_else(|| metadata_string(metadata, "channel_id"))?;
            metadata_string(metadata, "thread_ts")
                .filter(|thread_ts| !thread_ts.is_empty())
                .map(|thread_ts| format!("{channel}:thread:{thread_ts}"))
                .or(Some(channel))
        }
        "whatsapp" => metadata_string(metadata, "sender_phone")
            .or_else(|| metadata_string(metadata, "chat_id"))
            .or_else(|| metadata_string(metadata, "phone_number_id")),
        "discord" => metadata_string(metadata, "thread_id")
            .or_else(|| metadata_string(metadata, "channel_id"))
            .or_else(|| emitted.thread_id.clone()),
        _ => metadata_string(metadata, "chat_id")
            .or_else(|| metadata_string(metadata, "channel_id"))
            .or_else(|| metadata_string(metadata, "conversation_id"))
            .or_else(|| emitted.thread_id.clone()),
    }
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    let value = metadata.get(key)?;
    match value {
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn metadata_bool(metadata: &serde_json::Value, key: &str) -> Option<bool> {
    match metadata.get(key)? {
        serde_json::Value::Bool(value) => Some(*value),
        serde_json::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn add_legacy_thread_aliases(
    metadata: &mut serde_json::Map<String, serde_json::Value>,
    channel_name: &str,
    legacy_thread_id: &str,
) {
    let legacy_thread_id = legacy_thread_id.trim();
    if legacy_thread_id.is_empty() {
        return;
    }

    let aliases = metadata
        .entry("legacy_session_key_aliases".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let Some(alias_values) = aliases.as_array_mut() else {
        return;
    };

    for alias in [
        legacy_thread_id.to_string(),
        format!("{channel_name}:{legacy_thread_id}"),
        format!("agent:main:{channel_name}:{legacy_thread_id}"),
    ] {
        let value = serde_json::Value::String(alias);
        if !alias_values.contains(&value) {
            alias_values.push(value);
        }
    }
}

/// HTTP response from a WASM channel callback.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response headers.
    pub headers: HashMap<String, String>,
    /// Response body.
    pub body: Vec<u8>,
}

pub(super) fn metadata_object(
    value: &serde_json::Value,
    fallback_key: &str,
) -> serde_json::Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map.clone(),
        serde_json::Value::Null => serde_json::Map::new(),
        other => {
            let mut map = serde_json::Map::new();
            map.insert(fallback_key.to_string(), other.clone());
            map
        }
    }
}

fn serialize_response_attachments(
    attachments: &[thinclaw_media::MediaContent],
) -> Option<serde_json::Value> {
    if attachments.is_empty() {
        return None;
    }

    use base64::Engine;

    Some(serde_json::Value::Array(
        attachments
            .iter()
            .map(|attachment| {
                serde_json::json!({
                    "mime_type": attachment.mime_type,
                    "filename": attachment.filename,
                    "data": base64::engine::general_purpose::STANDARD.encode(&attachment.data),
                })
            })
            .collect(),
    ))
}

pub(super) fn merged_response_metadata(
    original_metadata: &serde_json::Value,
    response: &OutgoingResponse,
) -> serde_json::Value {
    let mut merged = metadata_object(original_metadata, "original_metadata");

    for (key, value) in metadata_object(&response.metadata, "response_metadata") {
        merged.insert(key, value);
    }

    if let Some(serialized_attachments) = serialize_response_attachments(&response.attachments) {
        merged.insert("response_attachments".to_string(), serialized_attachments);
    }

    serde_json::Value::Object(merged)
}

pub(super) fn response_content_for_wasm(channel_name: &str, response: &OutgoingResponse) -> String {
    if response.attachments.is_empty() || wasm_channel_has_media_delivery(channel_name) {
        return response.content.clone();
    }

    let mut content = response.content.clone();
    if !content.is_empty() {
        content.push_str("\n\n");
    }
    content.push_str("Generated media:");
    for attachment in &response.attachments {
        let filename = attachment.filename.as_deref().unwrap_or("generated-media");
        let source = attachment.source_url.as_deref().unwrap_or("stored locally");
        content.push_str(&format!(
            "\n- {} ({} bytes, {}): {}",
            filename,
            attachment.data.len(),
            attachment.mime_type,
            source
        ));
    }
    tracing::info!(
        channel = %channel_name,
        attachment_count = response.attachments.len(),
        "WASM channel using generated media text fallback"
    );
    content
}

fn wasm_channel_has_media_delivery(channel_name: &str) -> bool {
    matches!(channel_name, "telegram" | "whatsapp" | "slack" | "discord")
}

impl HttpResponse {
    /// Create an OK response.
    pub fn ok() -> Self {
        Self {
            status: 200,
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    /// Create a JSON response.
    pub fn json(value: serde_json::Value) -> Self {
        let body = serde_json::to_vec(&value).unwrap_or_default();
        let mut headers = HashMap::new();
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        Self {
            status: 200,
            headers,
            body,
        }
    }

    /// Create an error response.
    pub fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: message.as_bytes().to_vec(),
        }
    }
}

pub(super) fn default_wasm_channel_formatting_hints(channel_name: &str) -> Option<String> {
    match channel_name {
        "telegram" => Some(
            "Prefer Telegram HTML-style formatting for emphasis and links; standard Markdown is also supported as a fallback. Keep code blocks short, avoid markdown tables, and expect long replies to be split into multiple messages."
                .to_string(),
        ),
        "slack" => Some(
            "Use Slack mrkdwn formatting, not GitHub-flavored markdown. Keep replies easy to skim and avoid relying on raw HTML.".to_string(),
        ),
        "whatsapp" => Some(
            "Use WhatsApp-friendly plain text with light emphasis only. Avoid markdown tables, long fenced code blocks, and dense nested structure."
                .to_string(),
        ),
        "discord" => Some(
            "Discord supports markdown and fenced code blocks. Keep long answers readable with short sections and avoid overly wide tables."
                .to_string(),
        ),
        _ => None,
    }
}
