//! lib: dispatch.

use super::*;

/// Send a message via the Telegram Bot API.
///
/// Returns the sent message_id on success. When `parse_mode` is set and
/// Telegram returns a 400 "can't parse entities" error, returns
/// `SendError::ParseEntities` so the caller can retry without formatting.
pub(crate) fn send_message(
    chat_id: i64,
    text: &str,
    reply_to_message_id: Option<i64>,
    parse_mode: Option<&str>,
    message_thread_id: Option<i64>,
) -> Result<i64, SendError> {
    let mut payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });

    if let Some(message_id) = reply_to_message_id {
        // Skip reply_to_message_id when it's 0 — this happens for broadcast
        // (proactive) messages where there's no original message to reply to.
        // Telegram rejects message_id 0 with "message to reply not found".
        if message_id > 0 {
            payload["reply_to_message_id"] = serde_json::Value::Number(message_id.into());
        }
    }

    if let Some(mode) = parse_mode {
        payload["parse_mode"] = serde_json::Value::String(mode.to_string());
    }

    // Thread targeting for forum topics
    if let Some(thread_id) = message_thread_id {
        payload["message_thread_id"] = serde_json::json!(thread_id);
    }

    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| SendError::Other(format!("Failed to serialize payload: {}", e)))?;

    let headers = serde_json::json!({ "Content-Type": "application/json" });

    let result = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendMessage",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(http_response) => {
            if http_response.status == 400 {
                let body_str = String::from_utf8_lossy(&http_response.body);
                if body_str.contains("can't parse entities") {
                    return Err(SendError::ParseEntities(body_str.to_string()));
                }
                // Telegram returns this when the forum topic was deleted by the user.
                if body_str.contains("message thread not found")
                    || body_str.contains("TOPIC_ID_INVALID")
                {
                    return Err(SendError::ThreadNotFound(body_str.to_string()));
                }
                return Err(SendError::Other(format!(
                    "Telegram API returned 400: {}",
                    body_str
                )));
            }

            if http_response.status != 200 {
                let body_str = String::from_utf8_lossy(&http_response.body);
                return Err(SendError::Other(format!(
                    "Telegram API returned status {}: {}",
                    http_response.status, body_str
                )));
            }

            let api_response: TelegramApiResponse<SentMessage> =
                serde_json::from_slice(&http_response.body)
                    .map_err(|e| SendError::Other(format!("Failed to parse response: {}", e)))?;

            if !api_response.ok {
                return Err(SendError::Other(format!(
                    "Telegram API error: {}",
                    api_response
                        .description
                        .unwrap_or_else(|| "unknown".to_string())
                )));
            }

            Ok(api_response.result.map(|r| r.message_id).unwrap_or(0))
        }
        Err(e) => Err(SendError::Other(format!("HTTP request failed: {}", e))),
    }
}

// ============================================================================
// Message Splitting
// ============================================================================

/// Split a message into chunks that fit within a character limit.
///
/// Tries to split at paragraph boundaries (`\n\n`), then line boundaries (`\n`),
/// then at the last space. Falls back to hard splitting at the char limit.
pub(crate) fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.chars().count() <= max_len {
        return vec![text.to_string()];
    }

    // Compute the byte index at which the `max_chars`-th character begins.
    //
    // Slicing `&text[..byte_index]` is then guaranteed to land on a UTF-8
    // char boundary, so a multibyte character that straddles `max_chars`
    // cannot trigger a panic.
    fn byte_index_for_char_limit(text: &str, max_chars: usize) -> usize {
        text.char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(text.len())
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.chars().count() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find the best split point within max_len characters
        let search_end = byte_index_for_char_limit(remaining, max_len);
        let search_area = &remaining[..search_end];

        // Priority 1: split at a paragraph break (\n\n)
        let split_at = search_area
            .rfind("\n\n")
            .map(|pos| pos + 1) // include first \n
            // Priority 2: split at a line break
            .or_else(|| search_area.rfind('\n'))
            // Priority 3: split at a space
            .or_else(|| search_area.rfind(' '))
            // Fallback: hard split at the char-boundary byte index for max_len
            .unwrap_or(search_end);

        if split_at == 0 {
            // Safety valve: avoid infinite loop on a leading oversized token
            chunks.push(search_area.to_string());
            remaining = remaining[search_end..].trim_start();
            continue;
        }

        chunks.push(remaining[..split_at].trim_end().to_string());
        remaining = remaining[split_at..].trim_start();
    }

    // Filter out empty chunks
    chunks.retain(|c| !c.is_empty());
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }

    chunks
}

// ============================================================================
// Markdown → Telegram HTML Conversion
// ============================================================================

/// Convert standard Markdown (as emitted by LLMs) to Telegram-safe HTML.
///
/// Delegates to the canonical host-side implementation via the WIT boundary
/// to ensure identical formatting between the streaming (`send_draft`) and
/// non-streaming (`on_respond`) paths.
pub(crate) fn markdown_to_telegram_html(md: &str) -> String {
    channel_host::markdown_to_telegram_html(md)
}

/// Escape HTML special characters for Telegram.
///
/// Used by non-converter call sites (e.g., pairing replies) that embed
/// user-supplied text in HTML-formatted messages.
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn write_workspace_state(path: &str, value: &str) {
    if let Err(error) = channel_host::workspace_write(path, value) {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!(
                "Failed to persist Telegram transport state at {}: {}",
                path, error
            ),
        );
    }
}

pub(crate) fn clear_workspace_state(path: &str) {
    write_workspace_state(path, "");
}

pub(crate) fn now_millis_string() -> String {
    channel_host::now_millis().to_string()
}

pub(crate) fn set_transport_error(message: &str) {
    write_workspace_state(LAST_TRANSPORT_ERROR_PATH, message);
}

pub(crate) fn clear_transport_error() {
    clear_workspace_state(LAST_TRANSPORT_ERROR_PATH);
}

pub(crate) fn read_bool_workspace_state(path: &str) -> Option<bool> {
    channel_host::workspace_read(path).and_then(|raw| match raw.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

pub(crate) fn probe_private_topic_settings() {
    let headers = serde_json::json!({}).to_string();
    let response = match channel_host::http_request(
        "GET",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/getMe",
        &headers,
        None,
        None,
    ) {
        Ok(response) => response,
        Err(error) => {
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!(
                    "Failed to probe Telegram bot topic settings via getMe: {}",
                    error
                ),
            );
            return;
        }
    };

    if response.status != 200 {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!(
                "Telegram getMe returned {} while probing topic settings: {}",
                response.status,
                String::from_utf8_lossy(&response.body)
            ),
        );
        return;
    }

    let api_response: TelegramApiResponse<TelegramUser> =
        match serde_json::from_slice(&response.body) {
            Ok(value) => value,
            Err(error) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to parse Telegram getMe response: {}", error),
                );
                return;
            }
        };

    if !api_response.ok {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!(
                "Telegram getMe probe failed: {}",
                api_response
                    .description
                    .unwrap_or_else(|| "unknown error".to_string())
            ),
        );
        return;
    }

    if let Some(bot) = api_response.result {
        if let Some(enabled) = bot.has_topics_enabled {
            write_workspace_state(
                PRIVATE_TOPICS_ENABLED_PATH,
                if enabled { "true" } else { "false" },
            );
        } else {
            clear_workspace_state(PRIVATE_TOPICS_ENABLED_PATH);
        }

        if let Some(allowed) = bot.allows_users_to_create_topics {
            write_workspace_state(
                PRIVATE_TOPICS_ALLOW_USER_CREATE_PATH,
                if allowed { "true" } else { "false" },
            );
        } else {
            clear_workspace_state(PRIVATE_TOPICS_ALLOW_USER_CREATE_PATH);
        }

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Telegram private topics probe: enabled={:?}, users_can_create={:?}",
                bot.has_topics_enabled, bot.allows_users_to_create_topics
            ),
        );
    }
}

pub(crate) fn read_managed_private_topic_registry() -> ManagedPrivateTopicRegistry {
    channel_host::workspace_read(MANAGED_PRIVATE_TOPICS_PATH)
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub(crate) fn write_managed_private_topic_registry(registry: &ManagedPrivateTopicRegistry) {
    match serde_json::to_string(registry) {
        Ok(serialized) => write_workspace_state(MANAGED_PRIVATE_TOPICS_PATH, &serialized),
        Err(error) => channel_host::log(
            channel_host::LogLevel::Warn,
            &format!(
                "Failed to serialize managed Telegram topic registry: {}",
                error
            ),
        ),
    }
}

pub(crate) fn managed_private_topic_id(
    state: &ManagedPrivateTopicState,
    kind: ManagedPrivateTopicKind,
) -> Option<i64> {
    match kind {
        ManagedPrivateTopicKind::Onboarding => state.onboarding_thread_id,
        ManagedPrivateTopicKind::General => state.general_thread_id,
    }
}

pub(crate) fn set_managed_private_topic_id(
    state: &mut ManagedPrivateTopicState,
    kind: ManagedPrivateTopicKind,
    thread_id: Option<i64>,
) {
    match kind {
        ManagedPrivateTopicKind::Onboarding => state.onboarding_thread_id = thread_id,
        ManagedPrivateTopicKind::General => state.general_thread_id = thread_id,
    }
}

/// Invalidate a persisted managed topic thread_id (e.g. because the topic was
/// deleted by the user). The next call to `ensure_managed_private_topic` will
/// re-create it.
pub(crate) fn invalidate_managed_private_topic(chat_id: i64, kind: ManagedPrivateTopicKind) {
    let mut registry = read_managed_private_topic_registry();
    let chat_key = chat_id.to_string();
    if let Some(state) = registry.chats.get_mut(&chat_key) {
        set_managed_private_topic_id(state, kind, None);
        write_managed_private_topic_registry(&registry);
        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "Invalidated stale managed Telegram topic {:?} for chat {} (will recreate on next use)",
                kind, chat_id
            ),
        );
    }
}

pub(crate) fn edit_forum_topic(
    chat_id: i64,
    message_thread_id: i64,
    name: &str,
) -> Result<(), String> {
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "message_thread_id": message_thread_id,
        "name": name,
    });
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize payload: {}", e))?;
    let headers = serde_json::json!({ "Content-Type": "application/json" });
    let response = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/editForumTopic",
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    )
    .map_err(|e| format!("HTTP request failed: {}", e))?;

    if response.status != 200 {
        let body_str = String::from_utf8_lossy(&response.body);
        return Err(format!(
            "Telegram API returned status {}: {}",
            response.status, body_str
        ));
    }

    let api_response: TelegramApiResponse<bool> = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse response: {}", e))?;
    if api_response.ok {
        Ok(())
    } else {
        Err(api_response
            .description
            .unwrap_or_else(|| "unknown edit topic error".to_string()))
    }
}

pub(crate) fn ensure_managed_private_topic(
    chat_id: i64,
    kind: ManagedPrivateTopicKind,
) -> Option<i64> {
    if matches!(
        read_bool_workspace_state(PRIVATE_TOPICS_ENABLED_PATH),
        Some(false)
    ) {
        return None;
    }

    let mut registry = read_managed_private_topic_registry();
    let chat_key = chat_id.to_string();
    let state = registry.chats.entry(chat_key).or_default();

    // Fast path: if we already have a persisted thread_id for this kind, reuse it.
    // We no longer probe via editForumTopic because Telegram returns HTTP 400
    // ("topic name was not changed") when the name is identical, which the old
    // code misinterpreted as "topic is stale" — causing a new General topic on
    // every startup.
    //
    // If the persisted thread_id turns out to be invalid (topic was deleted by
    // the user), the send_message call will fail and the caller can handle it.
    if let Some(existing_thread_id) = managed_private_topic_id(state, kind) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Reusing persisted managed Telegram topic {:?} for chat {} (thread {})",
                kind, chat_id, existing_thread_id
            ),
        );
        return Some(existing_thread_id);
    }

    // No persisted thread_id — create the topic for the first time.
    match create_forum_topic(chat_id, kind.display_name()) {
        Ok(thread_id) => {
            set_managed_private_topic_id(state, kind, Some(thread_id));
            write_managed_private_topic_registry(&registry);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!(
                    "Created managed Telegram topic '{}' for chat {} (thread {})",
                    kind.display_name(),
                    chat_id,
                    thread_id
                ),
            );
            Some(thread_id)
        }
        Err(error) => {
            write_managed_private_topic_registry(&registry);
            channel_host::log(
                channel_host::LogLevel::Warn,
                &format!(
                    "Failed to create managed Telegram topic '{}' for chat {}: {}",
                    kind.display_name(),
                    chat_id,
                    error
                ),
            );
            None
        }
    }
}

pub(crate) fn resolve_outgoing_message_thread_id(
    metadata: &TelegramMessageMetadata,
    response_thread_id: Option<&str>,
) -> Option<i64> {
    if metadata.is_private {
        if let Some(kind) = ManagedPrivateTopicKind::from_response_thread_id(response_thread_id) {
            return ensure_managed_private_topic(metadata.chat_id, kind)
                .or(metadata.message_thread_id);
        }
    }

    response_thread_id
        .and_then(|thread_id| thread_id.trim().parse::<i64>().ok())
        .filter(|thread_id| *thread_id > 0)
        .or(metadata.message_thread_id)
}

pub(crate) fn tool_result_deleted_path(update: &StatusUpdate) -> Option<String> {
    if update.status != StatusType::ToolResult {
        return None;
    }

    let (header, body) = update.message.split_once('\n')?;
    if !header
        .trim()
        .eq_ignore_ascii_case("Tool result: memory_delete")
    {
        return None;
    }

    let payload: serde_json::Value = serde_json::from_str(body.trim()).ok()?;
    if !payload
        .get("status")
        .and_then(|value| value.as_str())
        .is_some_and(|status| status.eq_ignore_ascii_case("deleted"))
    {
        return None;
    }

    payload
        .get("path")
        .and_then(|value| value.as_str())
        .map(ToString::to_string)
}

pub(crate) fn should_ensure_general_topic_after_status(
    metadata: &TelegramMessageMetadata,
    update: &StatusUpdate,
) -> bool {
    metadata.is_private
        && tool_result_deleted_path(update).is_some_and(|path| {
            path.trim_start_matches('/')
                .eq_ignore_ascii_case("BOOTSTRAP.md")
        })
}

// ============================================================================
// Webhook Management
// ============================================================================

/// Delete any existing webhook with Telegram API.
///
/// Called during on_start() when switching to polling mode.
/// Telegram doesn't allow getUpdates while a webhook is active.
pub(crate) fn delete_webhook() -> Result<(), String> {
    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    let result = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/deleteWebhook",
        &headers.to_string(),
        None,
        None,
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!("HTTP {}: {}", response.status, body_str));
            }

            let api_response: TelegramApiResponse<bool> = serde_json::from_slice(&response.body)
                .map_err(|e| format!("Failed to parse response: {}", e))?;

            if !api_response.ok {
                return Err(format!(
                    "Telegram API error: {}",
                    api_response
                        .description
                        .unwrap_or_else(|| "unknown".to_string())
                ));
            }

            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook deleted successfully (switching to polling mode)",
            );

            Ok(())
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

/// Register webhook URL with Telegram API.
///
/// Called during on_start() when tunnel_url is configured.
pub(crate) fn register_webhook(
    tunnel_url: &str,
    webhook_secret: Option<&str>,
) -> Result<(), String> {
    let webhook_url = format!("{}/webhook/telegram", tunnel_url.trim_end_matches('/'));

    // Build setWebhook request body
    let mut body = serde_json::json!({
        "url": webhook_url,
        "allowed_updates": ["message", "edited_message"]
    });

    if let Some(secret) = webhook_secret {
        body["secret_token"] = serde_json::Value::String(secret.to_string());
    }

    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("Failed to serialize body: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json"
    });

    // Make HTTP request to Telegram API
    // Note: {TELEGRAM_BOT_TOKEN} is replaced by host with the actual token
    let result = channel_host::http_request(
        "POST",
        "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/setWebhook",
        &headers.to_string(),
        Some(&body_bytes),
        None,
    );

    match result {
        Ok(response) => {
            if response.status != 200 {
                let body_str = String::from_utf8_lossy(&response.body);
                return Err(format!("HTTP {}: {}", response.status, body_str));
            }

            // Parse Telegram API response
            let api_response: TelegramApiResponse<serde_json::Value> =
                serde_json::from_slice(&response.body)
                    .map_err(|e| format!("Failed to parse response: {}", e))?;

            if !api_response.ok {
                return Err(format!(
                    "Telegram API error: {}",
                    api_response
                        .description
                        .unwrap_or_else(|| "unknown".to_string())
                ));
            }

            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Webhook registered successfully: {}", webhook_url),
            );

            Ok(())
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

// ============================================================================
// Pairing Reply
// ============================================================================

/// Send a pairing code message to a chat. Uses HTML formatting for the inline code.
pub(crate) fn send_pairing_reply(chat_id: i64, code: &str) -> Result<(), String> {
    send_message(
        chat_id,
        &format!(
            "To pair with this bot, run: <code>thinclaw pairing approve telegram {}</code>",
            escape_html(code)
        ),
        None,
        Some("HTML"),
        None, // Pairing replies don't target a specific thread
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}

// ============================================================================
// Update Handling
// ============================================================================

/// Process a Telegram update and emit messages if applicable.
pub(crate) fn handle_update(update: TelegramUpdate) {
    if should_ignore_update(update.update_id) {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!(
                "Ignoring Telegram update {} (<= ignore_updates_until_id)",
                update.update_id
            ),
        );
        return;
    }

    write_workspace_state(LAST_INBOUND_AT_PATH, &now_millis_string());
    write_workspace_state(LAST_EMITTED_UPDATE_ID_PATH, &update.update_id.to_string());

    // Handle regular messages
    if let Some(message) = update.message {
        handle_message(message);
    }

    // Optionally handle edited messages the same way
    if let Some(message) = update.edited_message {
        handle_message(message);
    }
}
