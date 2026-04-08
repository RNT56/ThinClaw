// Telegram API types have fields reserved for future use (entities, reply threading, etc.)
#![allow(dead_code)]

//! Telegram Bot API channel for ThinClaw.
//!
//! This WASM component implements the channel interface for handling Telegram
//! webhooks and sending messages back via the Bot API.
//!
//! # Features
//!
//! - Webhook-based message receiving
//! - Private chat (DM) support
//! - Group chat support with @mention triggering
//! - Reply threading support
//! - User name extraction
//!
//! # Security
//!
//! - Bot token is injected by host during HTTP requests
//! - WASM never sees raw credentials
//! - Optional webhook secret validation by host

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, PollConfig, StatusType, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// Telegram API Types
// ============================================================================

/// Telegram Update object (webhook payload).
/// https://core.telegram.org/bots/api#update
#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    /// Unique update identifier.
    update_id: i64,

    /// New incoming message.
    message: Option<TelegramMessage>,

    /// Edited message.
    edited_message: Option<TelegramMessage>,

    /// Channel post (we ignore these for now).
    channel_post: Option<TelegramMessage>,
}

/// Telegram Message object.
/// https://core.telegram.org/bots/api#message
#[derive(Debug, Deserialize)]
struct TelegramMessage {
    /// Unique message identifier.
    message_id: i64,

    /// Sender (empty for channel posts).
    from: Option<TelegramUser>,

    /// Chat the message belongs to.
    chat: TelegramChat,

    /// Forum topic thread ID (for supergroups with Topics enabled).
    /// https://core.telegram.org/bots/api#message → message_thread_id
    message_thread_id: Option<i64>,

    /// Message text.
    text: Option<String>,

    /// Caption for media (photo, video, document, etc.).
    #[serde(default)]
    caption: Option<String>,

    /// Original message if this is a reply.
    reply_to_message: Option<Box<TelegramMessage>>,

    /// Bot command entities (for /commands).
    entities: Option<Vec<MessageEntity>>,

    /// Photo message — array of PhotoSize, sorted by size (last = largest).
    #[serde(default)]
    photo: Option<Vec<PhotoSize>>,

    /// Voice message (OGG/Opus audio).
    voice: Option<TelegramVoice>,

    /// Audio file sent as a music file.
    audio: Option<TelegramAudio>,

    /// General file/document.
    document: Option<TelegramDocument>,

    /// Video message.
    video: Option<TelegramVideo>,

    /// Video note (round video message).
    video_note: Option<TelegramVideoNote>,

    /// Sticker.
    sticker: Option<TelegramSticker>,
}

/// Telegram PhotoSize object.
/// https://core.telegram.org/bots/api#photosize
#[derive(Debug, Deserialize)]
struct PhotoSize {
    file_id: String,
    file_unique_id: String,
    width: i64,
    height: i64,
    file_size: Option<i64>,
}

/// Telegram Voice object.
/// https://core.telegram.org/bots/api#voice
#[derive(Debug, Deserialize)]
struct TelegramVoice {
    file_id: String,
    file_unique_id: String,
    duration: i64,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Audio object.
/// https://core.telegram.org/bots/api#audio
#[derive(Debug, Deserialize)]
struct TelegramAudio {
    file_id: String,
    file_unique_id: String,
    duration: i64,
    performer: Option<String>,
    title: Option<String>,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Document object.
/// https://core.telegram.org/bots/api#document
#[derive(Debug, Deserialize)]
struct TelegramDocument {
    file_id: String,
    file_unique_id: String,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram Video object.
/// https://core.telegram.org/bots/api#video
#[derive(Debug, Deserialize)]
struct TelegramVideo {
    file_id: String,
    file_unique_id: String,
    width: i64,
    height: i64,
    duration: i64,
    file_name: Option<String>,
    mime_type: Option<String>,
    file_size: Option<i64>,
}

/// Telegram VideoNote object.
/// https://core.telegram.org/bots/api#videonote
#[derive(Debug, Deserialize)]
struct TelegramVideoNote {
    file_id: String,
    file_unique_id: String,
    length: i64,
    duration: i64,
    file_size: Option<i64>,
}

/// Telegram Sticker object (partial).
/// https://core.telegram.org/bots/api#sticker
#[derive(Debug, Deserialize)]
struct TelegramSticker {
    file_id: String,
    file_unique_id: String,
    width: i64,
    height: i64,
    is_animated: Option<bool>,
    is_video: Option<bool>,
    file_size: Option<i64>,
}

/// Telegram File object (response from getFile).
/// https://core.telegram.org/bots/api#file
#[derive(Debug, Deserialize)]
struct TelegramFile {
    file_id: String,
    file_unique_id: String,
    file_size: Option<i64>,
    file_path: Option<String>,
}

/// Telegram User object.
/// https://core.telegram.org/bots/api#user
#[derive(Debug, Deserialize)]
struct TelegramUser {
    /// Unique user identifier.
    id: i64,

    /// True if this is a bot.
    is_bot: bool,

    /// User's first name.
    first_name: String,

    /// User's last name.
    last_name: Option<String>,

    /// Username (without @).
    username: Option<String>,
}

/// Telegram Chat object.
/// https://core.telegram.org/bots/api#chat
#[derive(Debug, Deserialize)]
struct TelegramChat {
    /// Unique chat identifier.
    id: i64,

    /// Type of chat: private, group, supergroup, or channel.
    #[serde(rename = "type")]
    chat_type: String,

    /// Title for groups/channels.
    title: Option<String>,

    /// Username for private chats.
    username: Option<String>,
}

/// Message entity (for parsing @mentions, commands, etc.).
/// https://core.telegram.org/bots/api#messageentity
#[derive(Debug, Deserialize)]
struct MessageEntity {
    /// Type: mention, bot_command, etc.
    #[serde(rename = "type")]
    entity_type: String,

    /// Offset in UTF-16 code units.
    offset: i64,

    /// Length in UTF-16 code units.
    length: i64,

    /// For "mention" type, the mentioned user.
    user: Option<TelegramUser>,
}

/// Telegram API response wrapper.
#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    /// True if the request was successful.
    ok: bool,

    /// Error description if not ok.
    description: Option<String>,

    /// Result on success.
    result: Option<T>,
}

/// Response from sendMessage.
#[derive(Debug, Deserialize)]
struct SentMessage {
    message_id: i64,
}

/// Workspace path for storing polling state.
const POLLING_STATE_PATH: &str = "state/last_update_id";

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";

/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";

/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";

/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "telegram";

/// Workspace path for persisting bot_username for mention detection in groups.
const BOT_USERNAME_PATH: &str = "state/bot_username";

/// Workspace path for persisting respond_to_all_group_messages flag.
const RESPOND_TO_ALL_GROUP_PATH: &str = "state/respond_to_all_group_messages";

/// Telegram limits messages to 4096 UTF-8 characters.
/// https://core.telegram.org/bots/api#sendmessage
const TELEGRAM_MAX_MESSAGE_LENGTH: usize = 4096;

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
#[derive(Debug, Serialize, Deserialize)]
struct TelegramMessageMetadata {
    /// Chat ID where the message was received.
    chat_id: i64,

    /// Original message ID (for reply_to_message_id).
    message_id: i64,

    /// User ID who sent the message.
    user_id: i64,

    /// Whether this is a private (DM) chat.
    is_private: bool,

    /// Forum topic thread ID (for supergroups with Topics enabled).
    /// When present, all replies must include this to target the correct topic.
    #[serde(default)]
    message_thread_id: Option<i64>,
}

/// Channel configuration injected by host.
///
/// The host injects runtime values like tunnel_url and webhook_secret.
/// The channel doesn't need to know about polling vs webhook mode - it just
/// checks if tunnel_url is set to determine behavior.
#[derive(Debug, Deserialize)]
struct TelegramConfig {
    /// Bot username (without @) for mention detection in groups.
    #[serde(default)]
    bot_username: Option<String>,

    /// Telegram user ID of the bot owner. When set, only messages from this
    /// user are processed. All others are silently dropped.
    #[serde(default)]
    owner_id: Option<i64>,

    /// DM policy: "pairing" (default), "allowlist", or "open".
    #[serde(default)]
    dm_policy: Option<String>,

    /// Allowed sender IDs/usernames from config (merged with pairing-approved store).
    #[serde(default)]
    allow_from: Option<Vec<String>>,

    /// Whether to respond to all group messages (not just mentions).
    #[serde(default)]
    respond_to_all_group_messages: bool,

    /// Public tunnel URL for webhook mode (injected by host from global settings).
    /// When set, webhook mode is enabled and polling is disabled.
    #[serde(default)]
    tunnel_url: Option<String>,

    /// Secret token for webhook validation (injected by host from secrets store).
    /// Telegram will include this in the X-Telegram-Bot-Api-Secret-Token header.
    #[serde(default)]
    webhook_secret: Option<String>,
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct TelegramChannel;

#[derive(Debug, Clone, PartialEq, Eq)]
enum TelegramStatusAction {
    Typing,
    Notify(String),
}

const TELEGRAM_STATUS_MAX_CHARS: usize = 600;

fn truncate_status_message(input: &str, max_chars: usize) -> String {
    let mut iter = input.chars();
    let truncated: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        format!("{}...", truncated)
    } else {
        truncated
    }
}

fn status_message_for_user(update: &StatusUpdate) -> Option<String> {
    let message = update.message.trim();
    if message.is_empty() {
        None
    } else {
        Some(truncate_status_message(message, TELEGRAM_STATUS_MAX_CHARS))
    }
}

fn get_updates_url(offset: i64, timeout_secs: u32) -> String {
    format!(
        "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getUpdates?offset={}&timeout={}&allowed_updates=[\"message\",\"edited_message\"]",
        offset, timeout_secs
    )
}

fn classify_status_update(update: &StatusUpdate) -> Option<TelegramStatusAction> {
    match update.status {
        StatusType::Thinking => Some(TelegramStatusAction::Typing),
        StatusType::Done | StatusType::Interrupted => None,
        // Tool telemetry can be noisy in chat; keep it as typing-only UX.
        StatusType::ToolStarted | StatusType::ToolCompleted | StatusType::ToolResult => None,
        StatusType::Status => {
            let msg = update.message.trim();
            if msg.eq_ignore_ascii_case("Done")
                || msg.eq_ignore_ascii_case("Interrupted")
                || msg.eq_ignore_ascii_case("Awaiting approval")
                || msg.eq_ignore_ascii_case("Rejected")
            {
                None
            } else {
                status_message_for_user(update).map(TelegramStatusAction::Notify)
            }
        }
        StatusType::ApprovalNeeded
        | StatusType::JobStarted
        | StatusType::AuthRequired
        | StatusType::AuthCompleted => {
            status_message_for_user(update).map(TelegramStatusAction::Notify)
        }
    }
}

impl Guest for TelegramChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Telegram channel config: {}", config_json),
        );

        let config: TelegramConfig = serde_json::from_str(&config_json)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        channel_host::log(channel_host::LogLevel::Info, "Telegram channel starting");

        if let Some(ref username) = config.bot_username {
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Bot username: @{}", username),
            );
        }

        // Persist owner_id so subsequent callbacks (on_http_request, on_poll) can read it
        if let Some(owner_id) = config.owner_id {
            if let Err(e) = channel_host::workspace_write(OWNER_ID_PATH, &owner_id.to_string()) {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to persist owner_id: {}", e),
                );
            }
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            // Clear any stale owner_id from a previous config
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
            channel_host::log(
                channel_host::LogLevel::Warn,
                "No owner_id configured, bot is open to all users",
            );
        }

        // Persist dm_policy and allow_from for DM pairing in handle_message
        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing").to_string();
        let _ = channel_host::workspace_write(DM_POLICY_PATH, &dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // Persist bot_username and respond_to_all_group_messages for group handling
        let _ = channel_host::workspace_write(
            BOT_USERNAME_PATH,
            &config.bot_username.unwrap_or_default(),
        );
        let _ = channel_host::workspace_write(
            RESPOND_TO_ALL_GROUP_PATH,
            &config.respond_to_all_group_messages.to_string(),
        );

        // Mode is determined by whether the host injected a tunnel_url
        // If tunnel is configured, use webhooks. Otherwise, use polling.
        let mut webhook_mode = config.tunnel_url.is_some();

        if webhook_mode {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook mode enabled (tunnel configured)",
            );

            // Register webhook with Telegram API
            if let Some(ref tunnel_url) = config.tunnel_url {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Registering webhook: {}/webhook/telegram", tunnel_url),
                );

                if let Err(e) = register_webhook(tunnel_url, config.webhook_secret.as_deref()) {
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("Failed to register webhook: {} — falling back to polling mode", e),
                    );
                    // Fall back to polling mode — delete any stale webhook so
                    // getUpdates works, then flip the mode flag.
                    let _ = delete_webhook();
                    webhook_mode = false;
                }
            }
        } else {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Polling mode enabled (no tunnel configured)",
            );

            // Delete any existing webhook before polling
            // Telegram doesn't allow getUpdates while a webhook is active
            if let Err(e) = delete_webhook() {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to delete webhook (may not exist): {}", e),
                );
            }
        }

        // Configure polling only if not in webhook mode
        let poll = if !webhook_mode {
            Some(PollConfig {
                interval_ms: 5000, // 5 seconds between poll ticks
                enabled: true,
            })
        } else {
            None
        };

        // Webhook secret validation is handled by the host
        let require_secret = config.webhook_secret.is_some();

        Ok(ChannelConfig {
            display_name: "Telegram".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/telegram".to_string(),
                methods: vec!["POST".to_string()],
                require_secret,
            }],
            poll,
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        // Check if webhook secret validation passed (if required)
        // The host validates X-Telegram-Bot-Api-Secret-Token header and sets secret_validated
        // If require_secret was true in config but validation failed, secret_validated will be false
        if !req.secret_validated {
            // This means require_secret was set but the secret didn't match
            // We still check the field even though the host should have already rejected invalid requests
            // This is defense in depth
            channel_host::log(
                channel_host::LogLevel::Warn,
                "Webhook request with invalid or missing secret token",
            );
            // Return 401 but Telegram will keep retrying, so this is just for logging
            // In practice, the host should reject these before they reach us
        }

        // Parse the request body as UTF-8
        let body_str = match std::str::from_utf8(&req.body) {
            Ok(s) => s,
            Err(_) => {
                return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
            }
        };

        // Parse as Telegram Update
        let update: TelegramUpdate = match serde_json::from_str(body_str) {
            Ok(u) => u,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("Failed to parse Telegram update: {}", e),
                );
                // Still return 200 to prevent Telegram from retrying
                return json_response(200, serde_json::json!({"ok": true}));
            }
        };

        // Handle the update
        handle_update(update);

        // Always respond 200 quickly (Telegram expects fast responses)
        json_response(200, serde_json::json!({"ok": true}))
    }

    fn on_poll() {
        // Read last offset from workspace storage
        let offset = match channel_host::workspace_read(POLLING_STATE_PATH) {
            Some(s) => s.parse::<i64>().unwrap_or(0),
            None => 0,
        };

        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Polling getUpdates with offset {}", offset),
        );

        let headers_json = serde_json::json!({}).to_string();
        // The WASM host enforces a 30s callback_timeout on on_poll.
        // Use a 20s long-poll so the full cycle (WASM startup + HTTP + processing)
        // fits comfortably within the 30s window.  Previous value of 30s caused
        // the callback to ALWAYS time out, leaving orphaned HTTP requests and
        // triggering 409 conflicts on subsequent getUpdates calls.
        let primary_url = get_updates_url(offset, 20);

        // 25s HTTP timeout outlives Telegram's 20s server-side long-poll.
        let result = match channel_host::http_request(
            "GET",
            &primary_url,
            &headers_json,
            None,
            Some(25_000),
        ) {
            Ok(response) => Ok(response),
            Err(primary_err) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "getUpdates request failed ({}), retrying once immediately",
                        primary_err
                    ),
                );

                let retry_url = get_updates_url(offset, 3);
                channel_host::http_request("GET", &retry_url, &headers_json, None, Some(8_000))
                    .map_err(|retry_err| {
                        format!("primary error: {}; retry error: {}", primary_err, retry_err)
                    })
            }
        };

        match result {
            Ok(response) => {
                if response.status != 200 {
                    let body_str = String::from_utf8_lossy(&response.body);
                    channel_host::log(
                        channel_host::LogLevel::Error,
                        &format!("getUpdates returned {}: {}", response.status, body_str),
                    );
                    return;
                }

                // Parse response
                let api_response: Result<TelegramApiResponse<Vec<TelegramUpdate>>, _> =
                    serde_json::from_slice(&response.body);

                match api_response {
                    Ok(resp) if resp.ok => {
                        if let Some(updates) = resp.result {
                            let mut new_offset = offset;

                            for update in updates {
                                // Track highest update_id for next poll
                                if update.update_id >= new_offset {
                                    new_offset = update.update_id + 1;
                                }

                                // Process the update (emits messages)
                                handle_update(update);
                            }

                            // Save new offset if it changed
                            if new_offset != offset {
                                if let Err(e) = channel_host::workspace_write(
                                    POLLING_STATE_PATH,
                                    &new_offset.to_string(),
                                ) {
                                    channel_host::log(
                                        channel_host::LogLevel::Error,
                                        &format!("Failed to save polling offset: {}", e),
                                    );
                                }
                            }
                        }
                    }
                    Ok(resp) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!(
                                "Telegram API error: {}",
                                resp.description.unwrap_or_else(|| "unknown".to_string())
                            ),
                        );
                    }
                    Err(e) => {
                        channel_host::log(
                            channel_host::LogLevel::Error,
                            &format!("Failed to parse getUpdates response: {}", e),
                        );
                    }
                }
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Error,
                    &format!("getUpdates request failed: {}", e),
                );
            }
        }
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        let metadata: TelegramMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "on_respond: chat_id={}, message_thread_id={:?}, is_private={}, metadata_json={}",
                metadata.chat_id, metadata.message_thread_id, metadata.is_private, response.metadata_json
            ),
        );

        // Convert standard Markdown (from LLM output) to Telegram-safe HTML
        let html_content = markdown_to_telegram_html(&response.content);
        let chunks = split_message(&html_content, TELEGRAM_MAX_MESSAGE_LENGTH);

        for (i, chunk) in chunks.iter().enumerate() {
            // Unconditionally set reply_to = None. If we reply_to a message directly, Telegram
            // clients implicitly group it into a sub-thread 'Replies' view which isolates the
            // conversation from the main flow. Passing None keeps it in the active chat/topic.
            let reply_to = None;

            // Try sending with HTML first; fall back to plain text if Telegram
            // can't parse the entities.
            let result = send_message(
                metadata.chat_id,
                chunk,
                reply_to,
                Some("HTML"),
                metadata.message_thread_id,
            );

            match result {
                Ok(msg_id) => {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!(
                            "Sent message chunk {}/{} to chat {}: message_id={}",
                            i + 1,
                            chunks.len(),
                            metadata.chat_id,
                            msg_id
                        ),
                    );
                }
                Err(SendError::ParseEntities(detail)) => {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("HTML parse failed ({}), retrying as plain text", detail),
                    );
                    // Fall back to original plain-text content for this chunk
                    let plain_chunks = split_message(&response.content, TELEGRAM_MAX_MESSAGE_LENGTH);
                    let plain_chunk = plain_chunks.get(i).map(|s| s.as_str()).unwrap_or(chunk);
                    send_message(
                        metadata.chat_id,
                        plain_chunk,
                        reply_to,
                        None,
                        metadata.message_thread_id,
                    )
                    .map_err(|e| format!("Plain-text retry also failed: {}", e))?;
                }
                Err(e) => return Err(e.to_string()),
            }
        }

        Ok(())
    }

    fn on_status(update: StatusUpdate) {
        let action = match classify_status_update(&update) {
            Some(action) => action,
            None => return,
        };

        // Parse chat_id from metadata
        let metadata: TelegramMessageMetadata = match serde_json::from_str(&update.metadata_json) {
            Ok(m) => m,
            Err(_) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    "on_status: no valid Telegram metadata, skipping status update",
                );
                return;
            }
        };

        match action {
            TelegramStatusAction::Typing => {
                // POST /sendChatAction with action "typing"
                let mut payload = serde_json::json!({
                    "chat_id": metadata.chat_id,
                    "action": "typing"
                });

                if let Some(thread_id) = metadata.message_thread_id {
                    payload["message_thread_id"] = serde_json::json!(thread_id);
                }

                let payload_bytes = match serde_json::to_vec(&payload) {
                    Ok(b) => b,
                    Err(_) => return,
                };

                let headers = serde_json::json!({
                    "Content-Type": "application/json"
                });

                let result = channel_host::http_request(
                    "POST",
                    "https://api.telegram.org/bot{TELEGRAM_BOT_TOKEN}/sendChatAction",
                    &headers.to_string(),
                    Some(&payload_bytes),
                    None,
                );

                if let Err(e) = result {
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!("sendChatAction failed: {}", e),
                    );
                }
            }
            TelegramStatusAction::Notify(prompt) => {
                // Send user-visible status updates for actionable events.
                if let Err(first_err) =
                    send_message(metadata.chat_id, &prompt, Some(metadata.message_id), None, metadata.message_thread_id)
                {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "Failed to send status reply ({}), retrying without reply context",
                            first_err
                        ),
                    );

                    if let Err(retry_err) = send_message(metadata.chat_id, &prompt, None, None, metadata.message_thread_id) {
                        channel_host::log(
                            channel_host::LogLevel::Debug,
                            &format!(
                                "Failed to send status message without reply context: {}",
                                retry_err
                            ),
                        );
                    }
                }
            }
        }
    }

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "Telegram channel shutting down",
        );
    }
}

// ============================================================================
// Send Message Helper
// ============================================================================

/// Errors from send_message, split so callers can match on parse-entity failures.
enum SendError {
    /// Telegram returned 400 with "can't parse entities" (Markdown issue).
    ParseEntities(String),
    /// Any other failure.
    Other(String),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::ParseEntities(detail) => write!(f, "parse entities error: {}", detail),
            SendError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

/// Send a message via the Telegram Bot API.
///
/// Returns the sent message_id on success. When `parse_mode` is set and
/// Telegram returns a 400 "can't parse entities" error, returns
/// `SendError::ParseEntities` so the caller can retry without formatting.
fn send_message(
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
fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        // Find the best split point within max_len characters
        let search_area = &remaining[..max_len];

        // Priority 1: split at a paragraph break (\n\n)
        let split_at = search_area.rfind("\n\n").map(|pos| pos + 1) // include first \n
            // Priority 2: split at a line break
            .or_else(|| search_area.rfind('\n'))
            // Priority 3: split at a space
            .or_else(|| search_area.rfind(' '))
            // Fallback: hard split at max_len (but on a char boundary)
            .unwrap_or_else(|| {
                // Find the last valid char boundary at or before max_len
                let mut boundary = max_len;
                while boundary > 0 && !remaining.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                boundary
            });

        if split_at == 0 {
            // Safety valve: avoid infinite loop
            chunks.push(remaining.to_string());
            break;
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
fn markdown_to_telegram_html(md: &str) -> String {
    channel_host::markdown_to_telegram_html(md)
}

/// Escape HTML special characters for Telegram.
///
/// Used by non-converter call sites (e.g., pairing replies) that embed
/// user-supplied text in HTML-formatted messages.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ============================================================================
// Webhook Management
// ============================================================================

/// Delete any existing webhook with Telegram API.
///
/// Called during on_start() when switching to polling mode.
/// Telegram doesn't allow getUpdates while a webhook is active.
fn delete_webhook() -> Result<(), String> {
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
fn register_webhook(tunnel_url: &str, webhook_secret: Option<&str>) -> Result<(), String> {
    let webhook_url = format!("{}/webhook/telegram", tunnel_url);

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
fn send_pairing_reply(chat_id: i64, code: &str) -> Result<(), String> {
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
fn handle_update(update: TelegramUpdate) {
    // Handle regular messages
    if let Some(message) = update.message {
        handle_message(message);
    }

    // Optionally handle edited messages the same way
    if let Some(message) = update.edited_message {
        handle_message(message);
    }
}

/// Process a single message.
fn handle_message(message: TelegramMessage) {
    // Use text or caption (for media messages)
    let content = message
        .text
        .as_deref()
        .filter(|t| !t.is_empty())
        .or_else(|| message.caption.as_deref().filter(|c| !c.is_empty()))
        .unwrap_or_default()
        .to_string();

    // Collect media descriptors: (file_id, mime_type, filename)
    let media_descriptors = collect_media_descriptors(&message);
    let has_media = !media_descriptors.is_empty();

    // Skip messages with no content AND no media
    if content.is_empty() && !has_media {
        return;
    }

    // Skip messages without a sender (channel posts)
    let from = match message.from {
        Some(f) => f,
        None => return,
    };

    // Skip bot messages to avoid loops
    if from.is_bot {
        return;
    }

    let is_private = message.chat.chat_type == "private";

    // Owner validation: when owner_id is set, only that user can message
    let owner_id_str = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());

    if let Some(ref id_str) = owner_id_str {
        if let Ok(owner_id) = id_str.parse::<i64>() {
            if from.id != owner_id {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Dropping message from non-owner user {} (owner: {})",
                        from.id, owner_id
                    ),
                );
                return;
            }
        }
    } else if is_private {
        // No owner_id: apply dm_policy for private chats
        let dm_policy =
            channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

        if dm_policy != "open" {
            // Build effective allow list: config allow_from + pairing store
            let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default();

            if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
                allowed.extend(store_allowed);
            }

            let id_str = from.id.to_string();
            let username_opt = from.username.as_deref();
            let is_allowed = allowed.contains(&"*".to_string())
                || allowed.contains(&id_str)
                || username_opt.map_or(false, |u| allowed.contains(&u.to_string()));

            if !is_allowed {
                if dm_policy == "pairing" {
                    // Upsert pairing request and send reply
                    let meta = serde_json::json!({
                        "chat_id": message.chat.id,
                        "user_id": from.id,
                        "username": username_opt,
                    })
                    .to_string();

                    match channel_host::pairing_upsert_request(CHANNEL_NAME, &id_str, &meta) {
                        Ok(result) => {
                            channel_host::log(
                                channel_host::LogLevel::Info,
                                &format!(
                                    "Pairing request for user {} (chat {}): code {}",
                                    from.id, message.chat.id, result.code
                                ),
                            );
                            if result.created {
                                let _ = send_pairing_reply(message.chat.id, &result.code);
                            }
                        }
                        Err(e) => {
                            channel_host::log(
                                channel_host::LogLevel::Error,
                                &format!("Pairing upsert failed: {}", e),
                            );
                        }
                    }
                }
                return;
            }
        }
    }

    // For group chats, only respond if bot was mentioned or respond_to_all is enabled
    if !is_private {
        let respond_to_all = channel_host::workspace_read(RESPOND_TO_ALL_GROUP_PATH)
            .as_deref()
            .unwrap_or("false")
            == "true";

        if !respond_to_all {
            let has_command = content.starts_with('/');
            let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH).unwrap_or_default();
            let has_bot_mention = if bot_username.is_empty() {
                content.contains('@')
            } else {
                let mention = format!("@{}", bot_username);
                content.to_lowercase().contains(&mention.to_lowercase())
            };

            // In groups: need command, mention, or direct reply to bot
            if !has_command && !has_bot_mention {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Ignoring group message without mention: {}", content),
                );
                return;
            }
        }
    }

    // Build user display name
    let user_name = if let Some(ref last) = from.last_name {
        format!("{} {}", from.first_name, last)
    } else {
        from.first_name.clone()
    };

    // Build metadata for response routing
    let metadata = TelegramMessageMetadata {
        chat_id: message.chat.id,
        message_id: message.message_id,
        user_id: from.id,
        is_private,
        message_thread_id: message.message_thread_id,
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Download media attachments
    let attachments = download_media_attachments(&media_descriptors);

    // Determine content to emit
    let content_to_emit = if content.is_empty() {
        // Media-only message — provide a default prompt so the agent sees it
        if has_media {
            "[Media received — please analyze the attached content]".to_string()
        } else {
            return; // Should not reach here (checked above)
        }
    } else {
        let bot_username = channel_host::workspace_read(BOT_USERNAME_PATH).unwrap_or_default();
        match content_to_emit_for_agent(
            &content,
            if bot_username.is_empty() {
                None
            } else {
                Some(bot_username.as_str())
            },
        ) {
            Some(value) => value,
            None => return,
        }
    };

    // Emit the message to the agent
    // Use message_thread_id as the thread_id for forum topic threading
    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "handle_message: chat_id={}, message_id={}, message_thread_id={:?}, is_private={}, chat_type={}",
            message.chat.id, message.message_id, message.message_thread_id, is_private, message.chat.chat_type
        ),
    );

    channel_host::emit_message(&EmittedMessage {
        user_id: from.id.to_string(),
        user_name: Some(user_name),
        content: content_to_emit,
        thread_id: message.message_thread_id.map(|id| id.to_string()),
        metadata_json,
        attachments,
    });

    channel_host::log(
        channel_host::LogLevel::Info,
        &format!(
            "Emitted message from user {} in chat {} (thread: {:?}, attachments: {})",
            from.id, message.chat.id, message.message_thread_id, media_descriptors.len()
        ),
    );
}

// ============================================================================
// Media Download Helpers
// ============================================================================

/// Maximum file size we'll attempt to download (20 MB — matches Telegram Bot API limit).
const MAX_DOWNLOAD_SIZE: i64 = 20 * 1024 * 1024;

/// A descriptor for a media file to download.
struct MediaDescriptor {
    file_id: String,
    mime_type: String,
    filename: Option<String>,
}

/// Collect all downloadable media descriptors from the message.
fn collect_media_descriptors(message: &TelegramMessage) -> Vec<MediaDescriptor> {
    let mut descriptors = Vec::new();

    // Photo: take the largest resolution (last element)
    if let Some(ref photos) = message.photo {
        if let Some(largest) = photos.last() {
            // Skip photos that are clearly too large
            if largest.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
                descriptors.push(MediaDescriptor {
                    file_id: largest.file_id.clone(),
                    mime_type: "image/jpeg".to_string(), // Telegram always serves photos as JPEG
                    filename: Some(format!("photo_{}.jpg", largest.file_unique_id)),
                });
            }
        }
    }

    // Voice message (OGG/Opus)
    if let Some(ref voice) = message.voice {
        if voice.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: voice.file_id.clone(),
                mime_type: voice
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "audio/ogg".to_string()),
                filename: Some(format!("voice_{}.ogg", voice.file_unique_id)),
            });
        }
    }

    // Audio file (music)
    if let Some(ref audio) = message.audio {
        if audio.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: audio.file_id.clone(),
                mime_type: audio
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "audio/mpeg".to_string()),
                filename: audio.file_name.clone(),
            });
        }
    }

    // Document (general file)
    if let Some(ref doc) = message.document {
        if doc.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: doc.file_id.clone(),
                mime_type: doc
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".to_string()),
                filename: doc.file_name.clone(),
            });
        }
    }

    // Video
    if let Some(ref video) = message.video {
        if video.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: video.file_id.clone(),
                mime_type: video
                    .mime_type
                    .clone()
                    .unwrap_or_else(|| "video/mp4".to_string()),
                filename: video.file_name.clone(),
            });
        }
    }

    // Video note (round video)
    if let Some(ref vn) = message.video_note {
        if vn.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: vn.file_id.clone(),
                mime_type: "video/mp4".to_string(),
                filename: Some(format!("video_note_{}.mp4", vn.file_unique_id)),
            });
        }
    }

    // Sticker (as image — skip animated/video stickers)
    if let Some(ref sticker) = message.sticker {
        let is_static = !sticker.is_animated.unwrap_or(false) && !sticker.is_video.unwrap_or(false);
        if is_static && sticker.file_size.unwrap_or(0) <= MAX_DOWNLOAD_SIZE {
            descriptors.push(MediaDescriptor {
                file_id: sticker.file_id.clone(),
                mime_type: "image/webp".to_string(),
                filename: Some(format!("sticker_{}.webp", sticker.file_unique_id)),
            });
        }
    }

    descriptors
}

/// Download media files from Telegram and convert to WIT MediaAttachment format.
fn download_media_attachments(
    descriptors: &[MediaDescriptor],
) -> Vec<near::agent::channel_host::MediaAttachment> {
    use near::agent::channel_host::MediaAttachment;

    let mut attachments = Vec::new();
    let headers_json = serde_json::json!({"Accept": "*/*"}).to_string();

    for desc in descriptors {
        match download_telegram_file(&desc.file_id, &headers_json) {
            Ok(data) => {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Downloaded media: {} ({}, {} bytes)",
                        desc.filename.as_deref().unwrap_or("unnamed"),
                        desc.mime_type,
                        data.len()
                    ),
                );
                attachments.push(MediaAttachment {
                    mime_type: desc.mime_type.clone(),
                    data,
                    filename: desc.filename.clone(),
                });
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!(
                        "Failed to download media {}: {}",
                        desc.filename.as_deref().unwrap_or("unnamed"),
                        e
                    ),
                );
            }
        }
    }

    attachments
}

/// Download a file from Telegram Bot API using getFile + file download.
///
/// Step 1: Call getFile to get the file_path
/// Step 2: Download from https://api.telegram.org/file/bot<token>/<file_path>
///
/// The host injects the bot token into the request automatically.
fn download_telegram_file(file_id: &str, headers_json: &str) -> Result<Vec<u8>, String> {
    // Step 1: getFile API call
    let get_file_url = format!(
        "https://api.telegram.org/bot{{TELEGRAM_BOT_TOKEN}}/getFile?file_id={}",
        file_id
    );

    let response = channel_host::http_request("GET", &get_file_url, headers_json, None, Some(10_000))
        .map_err(|e| format!("getFile HTTP failed: {}", e))?;

    if response.status != 200 {
        return Err(format!("getFile returned HTTP {}", response.status));
    }

    let api_response: TelegramApiResponse<TelegramFile> = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse getFile response: {}", e))?;

    if !api_response.ok {
        return Err(format!(
            "getFile API error: {}",
            api_response.description.unwrap_or_else(|| "unknown".to_string())
        ));
    }

    let file = api_response
        .result
        .ok_or_else(|| "getFile returned no result".to_string())?;

    let file_path = file
        .file_path
        .ok_or_else(|| "getFile returned no file_path".to_string())?;

    // Step 2: Download the actual file binary
    let download_url = format!(
        "https://api.telegram.org/file/bot{{TELEGRAM_BOT_TOKEN}}/{}",
        file_path
    );

    let download_response =
        channel_host::http_request("GET", &download_url, headers_json, None, Some(30_000))
            .map_err(|e| format!("File download HTTP failed: {}", e))?;

    if download_response.status != 200 {
        return Err(format!(
            "File download returned HTTP {}",
            download_response.status
        ));
    }

    if download_response.body.is_empty() {
        return Err("File download returned empty body".to_string());
    }

    Ok(download_response.body)
}


/// Clean message text by removing bot commands and @mentions at the start.
/// When bot_username is set, only strips that specific mention; otherwise strips any leading @mention.
fn clean_message_text(text: &str, bot_username: Option<&str>) -> String {
    let mut result = text.trim().to_string();

    // Remove leading /command
    if result.starts_with('/') {
        if let Some(space_idx) = result.find(' ') {
            result = result[space_idx..].trim_start().to_string();
        } else {
            // Just a command with no text
            return String::new();
        }
    }

    // Remove leading @mention
    if result.starts_with('@') {
        if let Some(bot) = bot_username {
            let mention = format!("@{}", bot);
            let mention_lower = mention.to_lowercase();
            let result_lower = result.to_lowercase();
            if result_lower.starts_with(&mention_lower) {
                let rest = result[mention.len()..].trim_start();
                if rest.is_empty() {
                    return String::new();
                }
                result = rest.to_string();
            } else if let Some(space_idx) = result.find(' ') {
                // Different leading @mention - only strip if it's the bot
                let first_word = &result[..space_idx];
                if first_word.eq_ignore_ascii_case(&mention) {
                    result = result[space_idx..].trim_start().to_string();
                }
            }
        } else {
            // No bot_username: strip any leading @mention
            if let Some(space_idx) = result.find(' ') {
                result = result[space_idx..].trim_start().to_string();
            } else {
                return String::new();
            }
        }
    }

    result
}

/// Decide which user content should be emitted to the agent loop.
///
/// - `/start` emits a placeholder so the agent can greet the user
/// - bare slash commands are passed through for Submission parsing
/// - empty/mention-only messages are ignored
/// - otherwise cleaned text is emitted
fn content_to_emit_for_agent(content: &str, bot_username: Option<&str>) -> Option<String> {
    let cleaned_text = clean_message_text(content, bot_username);
    let trimmed_content = content.trim();

    if trimmed_content.eq_ignore_ascii_case("/start") {
        return Some("[User started the bot]".to_string());
    }

    if cleaned_text.is_empty() && trimmed_content.starts_with('/') {
        return Some(trimmed_content.to_string());
    }

    if cleaned_text.is_empty() {
        return None;
    }

    Some(cleaned_text)
}

// ============================================================================
// Utilities
// ============================================================================

/// Create a JSON HTTP response.
fn json_response(status: u16, value: serde_json::Value) -> OutgoingHttpResponse {
    let body = serde_json::to_vec(&value).unwrap_or_default();
    let headers = serde_json::json!({"Content-Type": "application/json"});

    OutgoingHttpResponse {
        status,
        headers_json: headers.to_string(),
        body,
    }
}

// Export the component
export!(TelegramChannel);

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_message_text() {
        // Without bot_username: strips any leading @mention
        assert_eq!(clean_message_text("/start hello", None), "hello");
        assert_eq!(clean_message_text("@bot hello world", None), "hello world");
        assert_eq!(clean_message_text("/start", None), "");
        assert_eq!(clean_message_text("@botname", None), "");
        assert_eq!(clean_message_text("just text", None), "just text");
        assert_eq!(clean_message_text("  spaced  ", None), "spaced");

        // With bot_username: only strips @MyBot, not @alice
        assert_eq!(clean_message_text("@MyBot hello", Some("MyBot")), "hello");
        assert_eq!(clean_message_text("@mybot hi", Some("MyBot")), "hi");
        assert_eq!(
            clean_message_text("@alice hello", Some("MyBot")),
            "@alice hello"
        );
        assert_eq!(clean_message_text("@MyBot", Some("MyBot")), "");
    }

    #[test]
    fn test_clean_message_text_bare_commands() {
        // Bare commands return empty (the caller decides what to emit)
        assert_eq!(clean_message_text("/start", None), "");
        assert_eq!(clean_message_text("/interrupt", None), "");
        assert_eq!(clean_message_text("/stop", None), "");
        assert_eq!(clean_message_text("/help", None), "");
        assert_eq!(clean_message_text("/undo", None), "");
        assert_eq!(clean_message_text("/ping", None), "");

        // Commands with args: command prefix stripped, args returned
        assert_eq!(clean_message_text("/start hello", None), "hello");
        assert_eq!(clean_message_text("/help me please", None), "me please");
        assert_eq!(
            clean_message_text("/model claude-opus-4-6", None),
            "claude-opus-4-6"
        );
    }

    /// Tests for the content_to_emit logic in handle_message.
    /// Since handle_message uses WASM host calls, test the extracted decision function.
    #[test]
    fn test_content_to_emit_logic() {
        // /start → welcome placeholder
        assert_eq!(
            content_to_emit_for_agent("/start", None),
            Some("[User started the bot]".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/Start", None),
            Some("[User started the bot]".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("  /start  ", None),
            Some("[User started the bot]".to_string())
        );

        // /start with args → pass args through
        assert_eq!(
            content_to_emit_for_agent("/start hello", None),
            Some("hello".to_string())
        );

        // Control commands → pass through raw so Submission::parse() can match
        assert_eq!(
            content_to_emit_for_agent("/interrupt", None),
            Some("/interrupt".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/stop", None),
            Some("/stop".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/help", None),
            Some("/help".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/undo", None),
            Some("/undo".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/redo", None),
            Some("/redo".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/ping", None),
            Some("/ping".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/tools", None),
            Some("/tools".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/compact", None),
            Some("/compact".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/clear", None),
            Some("/clear".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/version", None),
            Some("/version".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/approve", None),
            Some("/approve".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/always", None),
            Some("/always".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/deny", None),
            Some("/deny".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/yes", None),
            Some("/yes".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("/no", None),
            Some("/no".to_string())
        );

        // Commands with args → cleaned text (command stripped)
        assert_eq!(
            content_to_emit_for_agent("/help me please", None),
            Some("me please".to_string())
        );

        // Plain text → pass through
        assert_eq!(
            content_to_emit_for_agent("hello world", None),
            Some("hello world".to_string())
        );
        assert_eq!(
            content_to_emit_for_agent("just text", None),
            Some("just text".to_string())
        );

        // Empty / whitespace → skip (None)
        assert_eq!(content_to_emit_for_agent("", None), None);
        assert_eq!(content_to_emit_for_agent("   ", None), None);

        // Bare @mention without bot → skip
        assert_eq!(content_to_emit_for_agent("@botname", None), None);

        // With bot username configured: other mentions are preserved.
        assert_eq!(
            content_to_emit_for_agent("@alice hello", Some("MyBot")),
            Some("@alice hello".to_string())
        );
    }

    #[test]
    fn test_config_with_owner_id() {
        let json = r#"{"owner_id": 123456789}"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.owner_id, Some(123456789));
    }

    #[test]
    fn test_config_without_owner_id() {
        let json = r#"{}"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.owner_id, None);
    }

    #[test]
    fn test_config_with_null_owner_id() {
        let json = r#"{"owner_id": null}"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.owner_id, None);
    }

    #[test]
    fn test_config_full() {
        let json = r#"{
            "bot_username": "my_bot",
            "owner_id": 42,
            "respond_to_all_group_messages": true
        }"#;
        let config: TelegramConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.bot_username, Some("my_bot".to_string()));
        assert_eq!(config.owner_id, Some(42));
        assert!(config.respond_to_all_group_messages);
    }

    #[test]
    fn test_parse_update() {
        let json = r#"{
            "update_id": 123,
            "message": {
                "message_id": 456,
                "from": {
                    "id": 789,
                    "is_bot": false,
                    "first_name": "John",
                    "last_name": "Doe"
                },
                "chat": {
                    "id": 789,
                    "type": "private"
                },
                "text": "Hello bot"
            }
        }"#;

        let update: TelegramUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(update.update_id, 123);

        let message = update.message.unwrap();
        assert_eq!(message.message_id, 456);
        assert_eq!(message.text.unwrap(), "Hello bot");

        let from = message.from.unwrap();
        assert_eq!(from.id, 789);
        assert_eq!(from.first_name, "John");
    }

    #[test]
    fn test_parse_message_with_caption() {
        let json = r#"{
            "message_id": 1,
            "from": {"id": 1, "is_bot": false, "first_name": "A"},
            "chat": {"id": 1, "type": "private"},
            "caption": "What's in this image?"
        }"#;
        let msg: TelegramMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.text, None);
        assert_eq!(msg.caption.as_deref(), Some("What's in this image?"));
    }

    #[test]
    fn test_get_updates_url_includes_offset_and_timeout() {
        let url = get_updates_url(444_809_884, 30);
        assert!(url.contains("offset=444809884"));
        assert!(url.contains("timeout=30"));
        assert!(url.contains("allowed_updates=[\"message\",\"edited_message\"]"));
    }

    #[test]
    fn test_classify_status_update_thinking() {
        let update = StatusUpdate {
            status: StatusType::Thinking,
            message: "Thinking...".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Typing)
        );
    }

    #[test]
    fn test_classify_status_update_approval_needed() {
        let update = StatusUpdate {
            status: StatusType::ApprovalNeeded,
            message: "Approval needed for tool 'http_request'".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Approval needed for tool 'http_request'".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_done_ignored() {
        let update = StatusUpdate {
            status: StatusType::Done,
            message: "Done".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_auth_required() {
        let update = StatusUpdate {
            status: StatusType::AuthRequired,
            message: "Authentication required for weather.".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Authentication required for weather.".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_tool_started_ignored() {
        let update = StatusUpdate {
            status: StatusType::ToolStarted,
            message: "Tool started: http_request".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_tool_completed_ignored() {
        let update = StatusUpdate {
            status: StatusType::ToolCompleted,
            message: "Tool completed: http_request (ok)".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_job_started_notify() {
        let update = StatusUpdate {
            status: StatusType::JobStarted,
            message: "Job started: Daily sync".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Job started: Daily sync".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_auth_completed_notify() {
        let update = StatusUpdate {
            status: StatusType::AuthCompleted,
            message: "Authentication completed for weather.".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Authentication completed for weather.".to_string()
            ))
        );
    }

    #[test]
    fn test_classify_status_update_tool_result_ignored() {
        let update = StatusUpdate {
            status: StatusType::ToolResult,
            message: "Tool result: http_request ...".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_awaiting_approval_ignored() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Awaiting approval".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_interrupted_ignored() {
        let update = StatusUpdate {
            status: StatusType::Interrupted,
            message: "Interrupted".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_done_ignored_case_insensitive() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "done".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_interrupted_ignored() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "interrupted".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_rejected_ignored() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Rejected".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(classify_status_update(&update), None);
    }

    #[test]
    fn test_classify_status_update_status_notify() {
        let update = StatusUpdate {
            status: StatusType::Status,
            message: "Context compaction started".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(
            classify_status_update(&update),
            Some(TelegramStatusAction::Notify(
                "Context compaction started".to_string()
            ))
        );
    }

    #[test]
    fn test_status_message_for_user_ignores_blank() {
        let update = StatusUpdate {
            status: StatusType::AuthRequired,
            message: "   ".to_string(),
            metadata_json: "{}".to_string(),
        };

        assert_eq!(status_message_for_user(&update), None);
    }

    #[test]
    fn test_truncate_status_message_appends_ellipsis() {
        let input = "abcdefghijklmnopqrstuvwxyz";
        let output = truncate_status_message(input, 10);
        assert_eq!(output, "abcdefghij...");
    }

    #[test]
    fn test_status_message_for_user_truncates_long_input() {
        let update = StatusUpdate {
            status: StatusType::AuthRequired,
            message: "x".repeat(700),
            metadata_json: "{}".to_string(),
        };

        let msg = status_message_for_user(&update).expect("expected message");
        assert!(msg.len() <= TELEGRAM_STATUS_MAX_CHARS + 3);
        assert!(msg.ends_with("..."));
    }
}
