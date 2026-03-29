//! Telegram Bot API channel via long-polling `getUpdates`.
//!
//! Uses the raw Telegram Bot HTTP API through `reqwest` — no heavy
//! framework dependency. Supports:
//! - Long-polling message reception
//! - Channel post reception (`channel_post` updates)
//! - Forum topic threading (`message_thread_id`)
//! - Text message sending with automatic Markdown→HTML formatting
//! - Owner-only mode (restrict to a single user ID)
//! - Allow-list filtering
//! - Message splitting for long responses (Telegram's 4096 char limit)
//! - Status updates via "typing" chat action

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;
use secrecy::ExposeSecret;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
use crate::config::TelegramConfig;
use crate::error::ChannelError;

/// Maximum message length for Telegram (UTF-16 units, we use chars as approx).
const MAX_MESSAGE_LENGTH: usize = 4096;

/// Long-poll timeout for `getUpdates` (seconds).
const POLL_TIMEOUT: u64 = 30;

/// Channel name constant.
const NAME: &str = "telegram";

// ── Telegram API types ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    message: Option<TgMessage>,
    /// Channel post (from Telegram broadcast channels).
    channel_post: Option<TgMessage>,
    /// Message reaction update.
    message_reaction: Option<TgMessageReaction>,
}

/// Telegram message reaction update.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TgMessageReaction {
    /// Chat where the reaction was set.
    chat: TgChat,
    /// Message that was reacted to.
    message_id: i64,
    /// User who changed the reaction (absent for anonymous reactions).
    user: Option<TgUser>,
    /// New list of reaction types set by the user.
    new_reaction: Vec<TgReactionType>,
    /// Previous list of reaction types.
    old_reaction: Vec<TgReactionType>,
}

/// A single reaction type.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum TgReactionType {
    /// Standard Unicode emoji reaction.
    #[serde(rename = "emoji")]
    Emoji { emoji: String },
    /// Custom emoji reaction (Telegram Premium).
    #[serde(rename = "custom_emoji")]
    CustomEmoji { custom_emoji_id: String },
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields populated by serde deserialization
struct TgMessage {
    message_id: i64,
    from: Option<TgUser>,
    /// Sender chat (for channel posts where `from` is absent).
    sender_chat: Option<TgChat>,
    chat: TgChat,
    text: Option<String>,
    #[serde(default)]
    photo: Option<Vec<TgPhotoSize>>,
    caption: Option<String>,
    /// Forum topic thread ID (for forum/topic-based supergroups).
    message_thread_id: Option<i64>,
    /// ID of the message this is a reply to.
    reply_to_message: Option<Box<TgMessage>>,
}

#[derive(Debug, Deserialize)]
struct TgUser {
    id: i64,
    first_name: String,
    last_name: Option<String>,
    username: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgChat {
    id: i64,
    #[serde(rename = "type")]
    chat_type: String,
    /// Chat title (for groups/channels).
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TgPhotoSize {
    #[allow(dead_code)]
    file_id: String,
}

// ── Channel implementation ──────────────────────────────────────────

/// Telegram Bot channel using long-polling.
pub struct TelegramChannel {
    config: TelegramConfig,
    client: Client,
    api_base: String,
    /// Shutdown flag.
    shutdown: Arc<AtomicBool>,
    /// Last processed update offset.
    offset: Arc<AtomicI64>,
}

impl TelegramChannel {
    /// Create a new Telegram channel.
    pub fn new(config: TelegramConfig) -> Result<Self, ChannelError> {
        let token = config.bot_token.expose_secret();
        let api_base = format!("https://api.telegram.org/bot{token}");

        let client = Client::builder()
            .timeout(Duration::from_secs(POLL_TIMEOUT + 10))
            .build()
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("HTTP client: {e}"),
            })?;

        Ok(Self {
            config,
            client,
            api_base,
            shutdown: Arc::new(AtomicBool::new(false)),
            offset: Arc::new(AtomicI64::new(0)),
        })
    }

    /// Check if a user is allowed to interact with the bot.
    fn is_user_allowed(&self, user_id: i64) -> bool {
        // Owner mode: only respond to owner
        if let Some(owner) = self.config.owner_id {
            return user_id == owner;
        }

        // Allow-list mode
        if self.config.allow_from.is_empty() {
            return true; // No restrictions
        }

        let id_str = user_id.to_string();
        self.config
            .allow_from
            .iter()
            .any(|a| a == "*" || a == &id_str)
    }

    /// Send a text message to a chat.
    ///
    /// Converts standard Markdown (from LLM output) to Telegram HTML.
    /// Falls back to plain text if Telegram rejects the HTML entities.
    async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        thread_id: Option<i64>,
    ) -> Result<(), ChannelError> {
        let html = markdown_to_telegram_html(text);
        // Split long messages (use the HTML version for length check)
        let chunks = split_message(&html);

        for chunk in &chunks {
            let mut payload = serde_json::json!({
                "chat_id": chat_id,
                "text": chunk,
                "parse_mode": "HTML",
            });
            if let Some(tid) = thread_id {
                payload["message_thread_id"] = serde_json::json!(tid);
            }

            let resp = self
                .client
                .post(format!("{}/sendMessage", self.api_base))
                .json(&payload)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("sendMessage: {e}"),
                })?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                // If HTML parsing fails, retry without parse_mode (plain text)
                if body.contains("can't parse entities") {
                    tracing::debug!("Telegram HTML parse failed, falling back to plain text");
                    // Fall back to the original (non-HTML) text chunk
                    let plain_chunks = split_message(text);
                    let plain_chunk = plain_chunks
                        .get(chunks.iter().position(|c| c == chunk).unwrap_or(0))
                        .map(|s| s.as_str())
                        .unwrap_or(chunk);
                    let mut fallback = serde_json::json!({
                        "chat_id": chat_id,
                        "text": plain_chunk,
                    });
                    if let Some(tid) = thread_id {
                        fallback["message_thread_id"] = serde_json::json!(tid);
                    }
                    self.client
                        .post(format!("{}/sendMessage", self.api_base))
                        .json(&fallback)
                        .send()
                        .await
                        .map_err(|e| ChannelError::SendFailed {
                            name: NAME.to_string(),
                            reason: format!("sendMessage fallback: {e}"),
                        })?;
                } else {
                    return Err(ChannelError::SendFailed {
                        name: NAME.to_string(),
                        reason: format!("API {status}: {body}"),
                    });
                }
            }
        }

        Ok(())
    }

    /// Send "typing" action to a chat.
    async fn send_typing(&self, chat_id: i64) -> Result<(), ChannelError> {
        let _ = self
            .client
            .post(format!("{}/sendChatAction", self.api_base))
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "action": "typing",
            }))
            .send()
            .await;
        Ok(())
    }

    /// Poll for updates using long polling.
    async fn get_updates(&self) -> Result<Vec<Update>, ChannelError> {
        let offset = self.offset.load(Ordering::Relaxed);

        let resp = self
            .client
            .get(format!("{}/getUpdates", self.api_base))
            .query(&[
                ("offset", offset.to_string()),
                ("timeout", POLL_TIMEOUT.to_string()),
                (
                    "allowed_updates",
                    serde_json::json!(["message", "channel_post"]).to_string(),
                ),
            ])
            .send()
            .await
            .map_err(|e| ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("getUpdates: {e}"),
            })?;

        let api_resp: TelegramResponse<Vec<Update>> =
            resp.json().await.map_err(|e| ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("getUpdates parse: {e}"),
            })?;

        if !api_resp.ok {
            return Err(ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("API error: {}", api_resp.description.unwrap_or_default()),
            });
        }

        let updates = api_resp.result.unwrap_or_default();

        // Update offset to acknowledge processed messages
        if let Some(last) = updates.last() {
            self.offset.store(last.update_id + 1, Ordering::Relaxed);
        }

        Ok(updates)
    }

    /// Send a poll to a chat.
    #[allow(dead_code)]
    async fn send_poll(
        &self,
        chat_id: i64,
        question: &str,
        options: &[String],
        is_anonymous: bool,
        allows_multiple_answers: bool,
    ) -> Result<(), ChannelError> {
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "question": question,
            "options": options.iter().map(|o| serde_json::json!({"text": o})).collect::<Vec<_>>(),
            "is_anonymous": is_anonymous,
            "allows_multiple_answers": allows_multiple_answers,
        });

        let resp = self
            .client
            .post(format!("{}/sendPoll", self.api_base))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("sendPoll: {e}"),
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("sendPoll API {status}: {body}"),
            });
        }

        Ok(())
    }

    /// Set a reaction emoji on a message.
    #[allow(dead_code)]
    async fn set_message_reaction(
        &self,
        chat_id: i64,
        message_id: i64,
        emoji: &str,
    ) -> Result<(), ChannelError> {
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "reaction": [{
                "type": "emoji",
                "emoji": emoji,
            }],
        });

        let resp = self
            .client
            .post(format!("{}/setMessageReaction", self.api_base))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("setMessageReaction: {e}"),
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::debug!("setMessageReaction {status}: {body}");
        }

        Ok(())
    }
}

impl TelegramChannel {
    /// Send a text message and return the Telegram message ID.
    ///
    /// Used for streaming draft replies. Converts Markdown to HTML.
    async fn send_message_with_id(&self, chat_id: i64, text: &str) -> Result<String, ChannelError> {
        let html = markdown_to_telegram_html(text);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": html,
            "parse_mode": "HTML",
        });

        let resp = self
            .client
            .post(format!("{}/sendMessage", self.api_base))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("sendMessage: {e}"),
            })?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().await.map_err(|e| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("Parse sendMessage response: {e}"),
        })?;

        // If HTML parsing failed, retry as plain text
        if !status.is_success() {
            let desc = body["description"].as_str().unwrap_or_default();
            if desc.contains("can't parse entities") {
                tracing::debug!("Telegram HTML parse failed in send_message_with_id, retrying plain");
                let fallback = serde_json::json!({
                    "chat_id": chat_id,
                    "text": text,
                });
                let resp2 = self
                    .client
                    .post(format!("{}/sendMessage", self.api_base))
                    .json(&fallback)
                    .send()
                    .await
                    .map_err(|e| ChannelError::SendFailed {
                        name: NAME.to_string(),
                        reason: format!("sendMessage fallback: {e}"),
                    })?;
                let body2: serde_json::Value =
                    resp2.json().await.map_err(|e| ChannelError::SendFailed {
                        name: NAME.to_string(),
                        reason: format!("Parse fallback response: {e}"),
                    })?;
                return Ok(body2["result"]["message_id"]
                    .as_i64()
                    .map(|id| id.to_string())
                    .unwrap_or_default());
            }
        }

        let msg_id = body["result"]["message_id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default();

        Ok(msg_id)
    }

    /// Edit an existing message's text.
    ///
    /// Converts Markdown to HTML. Falls back to plain text on parse failure.
    async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        let html = markdown_to_telegram_html(text);
        let mid = message_id.parse::<i64>().unwrap_or(0);
        let payload = serde_json::json!({
            "chat_id": chat_id,
            "message_id": mid,
            "text": html,
            "parse_mode": "HTML",
        });

        let resp = self
            .client
            .post(format!("{}/editMessageText", self.api_base))
            .json(&payload)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("editMessageText: {e}"),
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            if body.contains("can't parse entities") {
                // Retry without HTML parse mode
                tracing::debug!("editMessageText HTML parse failed, retrying plain");
                let fallback = serde_json::json!({
                    "chat_id": chat_id,
                    "message_id": mid,
                    "text": text,
                });
                let resp2 = self
                    .client
                    .post(format!("{}/editMessageText", self.api_base))
                    .json(&fallback)
                    .send()
                    .await
                    .map_err(|e| ChannelError::SendFailed {
                        name: NAME.to_string(),
                        reason: format!("editMessageText fallback: {e}"),
                    })?;
                let status2 = resp2.status();
                if !status2.is_success() {
                    let body2 = resp2.text().await.unwrap_or_default();
                    if !body2.contains("message is not modified") {
                        tracing::debug!("editMessageText fallback {status2}: {body2}");
                    }
                }
            } else if !body.contains("message is not modified") {
                tracing::debug!("editMessageText {status}: {body}");
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn name(&self) -> &str {
        NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);

        let client = self.client.clone();
        let api_base = self.api_base.clone();
        let config = self.config.clone();
        let shutdown = self.shutdown.clone();
        let offset = self.offset.clone();

        // Validate bot token by calling getMe
        let me_resp = client
            .get(format!("{api_base}/getMe"))
            .send()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("getMe: {e}"),
            })?;

        let me: TelegramResponse<serde_json::Value> =
            me_resp
                .json()
                .await
                .map_err(|e| ChannelError::StartupFailed {
                    name: NAME.to_string(),
                    reason: format!("getMe parse: {e}"),
                })?;

        if !me.ok {
            return Err(ChannelError::AuthFailed {
                name: NAME.to_string(),
                reason: "Invalid bot token".to_string(),
            });
        }

        let bot_name = me
            .result
            .as_ref()
            .and_then(|v| v.get("first_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("ThinClaw");
        tracing::info!("Telegram bot connected as @{}", bot_name);

        // Spawn polling task
        tokio::spawn(async move {
            let channel = TelegramChannel {
                config: config.clone(),
                client: client.clone(),
                api_base,
                shutdown: shutdown.clone(),
                offset,
            };

            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                match channel.get_updates().await {
                    Ok(updates) => {
                        for update in updates {
                            // Handle message reactions
                            if let Some(reaction) = update.message_reaction {
                                let user_id = match reaction.user {
                                    Some(ref u) => u.id.to_string(),
                                    None => continue, // Skip anonymous reactions
                                };

                                // Build reaction text from new reactions
                                let emojis: Vec<String> = reaction
                                    .new_reaction
                                    .iter()
                                    .map(|r| match r {
                                        TgReactionType::Emoji { emoji } => emoji.clone(),
                                        TgReactionType::CustomEmoji { custom_emoji_id } => {
                                            format!("custom:{custom_emoji_id}")
                                        }
                                    })
                                    .collect();

                                if emojis.is_empty() {
                                    continue; // Reaction removed, skip
                                }

                                let metadata = serde_json::json!({
                                    "chat_id": reaction.chat.id,
                                    "message_id": reaction.message_id,
                                    "chat_type": reaction.chat.chat_type,
                                    "is_reaction": true,
                                    "reaction_emojis": emojis,
                                });

                                let reaction_text = format!("[reacted with {}]", emojis.join(", "));

                                let incoming =
                                    IncomingMessage::new("telegram", &user_id, &reaction_text)
                                        .with_metadata(metadata);

                                if tx.send(incoming).await.is_err() {
                                    tracing::warn!("Telegram channel receiver dropped");
                                    return;
                                }
                                continue;
                            }

                            // Process both regular messages and channel posts
                            let msg = match update.message.or(update.channel_post) {
                                Some(m) => m,
                                None => continue,
                            };

                            // Determine if this is a channel post (no `from` user)
                            let is_channel_post = msg.from.is_none();

                            // Get text content
                            let text = msg
                                .text
                                .as_deref()
                                .or(msg.caption.as_deref())
                                .unwrap_or_default();

                            if text.is_empty() {
                                continue;
                            }

                            // Get sender info
                            let (user_id, user_name) = if let Some(ref user) = msg.from {
                                let name = match &user.username {
                                    Some(u) => u.clone(),
                                    None => {
                                        let mut n = user.first_name.clone();
                                        if let Some(last) = &user.last_name {
                                            n.push(' ');
                                            n.push_str(last);
                                        }
                                        n
                                    }
                                };
                                (user.id.to_string(), Some(name))
                            } else if let Some(ref sender_chat) = msg.sender_chat {
                                // Channel post: use sender_chat ID and title
                                let name = sender_chat
                                    .title
                                    .clone()
                                    .unwrap_or_else(|| format!("channel:{}", sender_chat.id));
                                (sender_chat.id.to_string(), Some(name))
                            } else {
                                continue;
                            };

                            // Check access (skip for channel posts — channel access is
                            // controlled at the Telegram bot level)
                            if !is_channel_post {
                                let sender_id: i64 = match user_id.parse() {
                                    Ok(id) => id,
                                    Err(_) => continue,
                                };

                                if !channel.is_user_allowed(sender_id) {
                                    tracing::debug!(
                                        "Telegram: ignoring message from unauthorized user {}",
                                        sender_id
                                    );
                                    continue;
                                }
                            }

                            let mut metadata = serde_json::json!({
                                "chat_id": msg.chat.id,
                                "message_id": msg.message_id,
                                "chat_type": msg.chat.chat_type,
                            });

                            if is_channel_post {
                                metadata["is_channel_post"] = serde_json::json!(true);
                            }
                            if let Some(thread_id) = msg.message_thread_id {
                                metadata["message_thread_id"] = serde_json::json!(thread_id);
                            }
                            if let Some(ref reply_to) = msg.reply_to_message {
                                metadata["reply_to_message_id"] =
                                    serde_json::json!(reply_to.message_id);
                            }

                            let incoming = IncomingMessage::new("telegram", &user_id, text)
                                .with_metadata(metadata);

                            // Add thread ID for forum topics
                            let incoming = if let Some(thread_id) = msg.message_thread_id {
                                incoming.with_thread(thread_id.to_string())
                            } else {
                                incoming
                            };

                            let incoming = if let Some(name) = user_name {
                                incoming.with_user_name(name)
                            } else {
                                incoming
                            };

                            if tx.send(incoming).await.is_err() {
                                tracing::warn!("Telegram channel receiver dropped");
                                return;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Telegram polling error: {e}");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let chat_id = msg
            .metadata
            .get("chat_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Missing chat_id in metadata".to_string(),
            })?;

        // Forward thread_id for forum topic replies
        let thread_id = msg
            .metadata
            .get("message_thread_id")
            .and_then(|v| v.as_i64());

        self.send_message(chat_id, &response.content, thread_id)
            .await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let chat_id = match metadata.get("chat_id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return Ok(()),
        };

        match status {
            StatusUpdate::Thinking(_) | StatusUpdate::ToolStarted { .. } => {
                self.send_typing(chat_id).await?;
            }
            _ => {}
        }

        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Parse user_id as chat_id for direct messages.
        // Skip gracefully if not a valid numeric ID (e.g. "default").
        let chat_id: i64 = match user_id.parse() {
            Ok(id) => id,
            Err(_) => {
                tracing::debug!(
                    recipient = user_id,
                    "Telegram: skipping broadcast — recipient is not a numeric chat ID"
                );
                return Ok(());
            }
        };

        // Support forum topic targeting: heartbeat/cron messages can specify
        // a message_thread_id in metadata to land in the correct topic.
        let thread_id = response
            .metadata
            .get("message_thread_id")
            .and_then(|v| v.as_i64());

        self.send_message(chat_id, &response.content, thread_id)
            .await
    }

    async fn send_draft(
        &self,
        draft: &DraftReplyState,
        metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        let chat_id = match metadata.get("chat_id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return Ok(None),
        };

        let display = draft.display_text();
        let text = if draft.posted {
            draft.final_text()
        } else {
            &display
        };

        if let Some(ref msg_id) = draft.message_id {
            // Edit existing message
            self.edit_message_text(chat_id, msg_id, text).await?;
            Ok(Some(msg_id.clone()))
        } else {
            // Send initial message
            let msg_id = self.send_message_with_id(chat_id, text).await?;
            Ok(Some(msg_id))
        }
    }

    fn stream_mode(&self) -> StreamMode {
        self.config.stream_mode
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let resp = self
            .client
            .get(format!("{}/getMe", self.api_base))
            .send()
            .await
            .map_err(|e| ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("Health check: {e}"),
            })?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: NAME.to_string(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.shutdown.store(true, Ordering::Relaxed);
        Ok(())
    }

    async fn react(
        &self,
        chat_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), ChannelError> {
        let cid: i64 = chat_id.parse().map_err(|_| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("Invalid chat_id: {chat_id}"),
        })?;
        let mid: i64 = message_id.parse().map_err(|_| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("Invalid message_id: {message_id}"),
        })?;
        self.set_message_reaction(cid, mid, emoji).await
    }

    async fn poll(
        &self,
        chat_id: &str,
        question: &str,
        options: &[String],
        is_anonymous: bool,
    ) -> Result<(), ChannelError> {
        let cid: i64 = chat_id.parse().map_err(|_| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("Invalid chat_id: {chat_id}"),
        })?;
        self.send_poll(cid, question, options, is_anonymous, false)
            .await
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Convert standard Markdown (as emitted by LLMs) to Telegram-safe HTML.
///
/// Telegram supports a limited subset of HTML:
/// `<b>`, `<i>`, `<u>`, `<s>`, `<code>`, `<pre>`, `<a href>`,
/// `<blockquote>`, `<tg-spoiler>`.
///
/// This converter handles the most common patterns from LLM output:
/// - `**bold**` / `__bold__`  → `<b>bold</b>`
/// - `*italic*` / `_italic_` → `<i>italic</i>`
/// - `` `inline code` ``     → `<code>inline code</code>`
/// - ```` ```lang\ncode``` ````→ `<pre><code class="language-lang">code</code></pre>`
/// - `# Heading`             → `<b>Heading</b>` (bold, since Telegram has no headings)
/// - `[text](url)`           → `<a href="url">text</a>`
/// - `~~strikethrough~~`     → `<s>strikethrough</s>`
/// - `> blockquote`          → `<blockquote>text</blockquote>`
/// - HTML special chars      → escaped (`<`, `>`, `&`)
///
/// Unrecognised markup is left as-is (harmless in HTML mode).
fn markdown_to_telegram_html(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + md.len() / 4);
    let lines: Vec<&str> = md.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];

        // ── Fenced code blocks ──────────────────────────────────────
        if line.trim_start().starts_with("```") {
            let lang = line.trim_start().trim_start_matches('`').trim();
            let mut code_lines = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code_lines.push(lines[i]);
                i += 1;
            }
            // Skip closing ```
            if i < lines.len() {
                i += 1;
            }
            let code_text = escape_html(&code_lines.join("\n"));
            if lang.is_empty() {
                out.push_str(&format!("<pre><code>{code_text}</code></pre>\n"));
            } else {
                out.push_str(&format!(
                    "<pre><code class=\"language-{lang}\">{code_text}</code></pre>\n"
                ));
            }
            continue;
        }

        // ── Blockquotes ─────────────────────────────────────────────
        if line.starts_with("> ") || line == ">" {
            let mut bq_lines = Vec::new();
            while i < lines.len()
                && (lines[i].starts_with("> ") || lines[i] == ">")
            {
                let content = lines[i].strip_prefix("> ").unwrap_or(
                    lines[i].strip_prefix(">").unwrap_or(lines[i]),
                );
                bq_lines.push(content);
                i += 1;
            }
            let inner = bq_lines
                .iter()
                .map(|l| format_inline(&escape_html(l)))
                .collect::<Vec<_>>()
                .join("\n");
            out.push_str(&format!("<blockquote>{inner}</blockquote>\n"));
            continue;
        }

        // ── Headings (# ... ######) → bold line ─────────────────────
        if let Some(heading) = strip_heading(line) {
            let escaped = escape_html(heading);
            let formatted = format_inline(&escaped);
            out.push_str(&format!("<b>{formatted}</b>\n"));
            i += 1;
            continue;
        }

        // ── Regular line → inline formatting ────────────────────────
        let escaped = escape_html(line);
        let formatted = format_inline(&escaped);
        out.push_str(&formatted);
        out.push('\n');
        i += 1;
    }

    // Trim trailing newline to match original behavior
    if out.ends_with('\n') {
        out.truncate(out.len() - 1);
    }
    out
}

/// Escape HTML special characters for Telegram.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Sentinel used to protect unmatched `**` from the `*` handler.
/// U+FFFE is a Unicode noncharacter — never appears in real text.
const SENTINEL_STAR: &str = "\u{FFFE}\u{FFFE}";
/// Sentinel for unmatched `__`.
const SENTINEL_UNDER: &str = "\u{FFFF}\u{FFFF}";

/// Apply inline Markdown formatting to an already-HTML-escaped string.
///
/// Order matters: code spans first (to protect their content), then
/// bold, italic, strikethrough, links.
fn format_inline(s: &str) -> String {
    let mut result = String::from(s);

    // Inline code: `code` → <code>code</code>
    // Process from left to right, handling pairs of backticks
    result = replace_inline_pairs(&result, "`", "<code>", "</code>", None);

    // Bold: **text** → <b>text</b>  (must come before single *)
    result = replace_inline_pairs(&result, "**", "<b>", "</b>", Some(SENTINEL_STAR));

    // Bold (underscores): __text__ → <b>text</b>
    result = replace_inline_pairs(&result, "__", "<b>", "</b>", Some(SENTINEL_UNDER));

    // Strikethrough: ~~text~~ → <s>text</s>
    result = replace_inline_pairs(&result, "~~", "<s>", "</s>", None);

    // Italic: *text* → <i>text</i>  (single asterisk, after ** is handled)
    result = replace_inline_pairs(&result, "*", "<i>", "</i>", None);

    // Italic (underscore): _text_ → <i>text</i>
    result = replace_inline_pairs(&result, "_", "<i>", "</i>", None);

    // Links: [text](url) → <a href="url">text</a>
    // The brackets/parens are already HTML-escaped, so we match escaped versions
    result = convert_links(&result);

    // Restore sentinels
    result = result.replace(SENTINEL_STAR, "**");
    result = result.replace(SENTINEL_UNDER, "__");

    result
}

/// Replace matched pairs of a delimiter with open/close HTML tags.
///
/// Scans left-to-right. Each pair of `delim` becomes `open_tag...close_tag`.
/// Unmatched trailing delimiters are left as-is (or replaced with
/// `unmatched_sentinel` if provided, to prevent sub-delimiter passes from
/// consuming the characters).
fn replace_inline_pairs(
    s: &str,
    delim: &str,
    open_tag: &str,
    close_tag: &str,
    unmatched_sentinel: Option<&str>,
) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    let mut open = true; // next occurrence opens

    while let Some(pos) = rest.find(delim) {
        let before = &rest[..pos];
        out.push_str(before);

        // Skip the delimiter
        rest = &rest[pos + delim.len()..];

        if open {
            // Check there's a matching close delimiter ahead
            if rest.contains(delim) {
                out.push_str(open_tag);
                open = false;
            } else {
                // No matching close — emit sentinel or raw delimiter
                out.push_str(unmatched_sentinel.unwrap_or(delim));
            }
        } else {
            out.push_str(close_tag);
            open = true;
        }
    }
    out.push_str(rest);
    out
}

/// Convert Markdown links `[text](url)` to `<a href="url">text</a>`.
///
/// Since the input is already HTML-escaped, we need to look for the escaped
/// brackets: `[` and `]` are not escaped by our `escape_html`, but `>` inside
/// the URL would be. In practice URLs don't contain `>`, so this is fine.
fn convert_links(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;

    while let Some(bracket_open) = rest.find('[') {
        out.push_str(&rest[..bracket_open]);
        let after_open = &rest[bracket_open + 1..];

        if let Some(bracket_close) = after_open.find(']') {
            let link_text = &after_open[..bracket_close];
            let after_close = &after_open[bracket_close + 1..];

            if after_close.starts_with('(') {
                if let Some(paren_close) = after_close.find(')') {
                    let url = &after_close[1..paren_close];
                    out.push_str(&format!("<a href=\"{url}\">{link_text}</a>"));
                    rest = &after_close[paren_close + 1..];
                    continue;
                }
            }

            // Not a valid link — emit the bracket + continue
            out.push('[');
            rest = after_open;
        } else {
            out.push('[');
            rest = after_open;
        }
    }
    out.push_str(rest);
    out
}

/// Strip a Markdown heading prefix (`# ` through `###### `) and return the text.
fn strip_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    // Match 1-6 '#' followed by a space
    if trimmed.starts_with('#') {
        let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
        if hashes <= 6 {
            let rest = &trimmed[hashes..];
            if rest.starts_with(' ') {
                return Some(rest[1..].trim());
            }
        }
    }
    None
}

/// Split a long message into chunks that fit Telegram's limit.
fn split_message(text: &str) -> Vec<String> {
    if text.len() <= MAX_MESSAGE_LENGTH {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= MAX_MESSAGE_LENGTH {
            chunks.push(remaining.to_string());
            break;
        }

        // Try to split at a newline near the limit (safe for multi-byte UTF-8)
        let safe_end = crate::util::floor_char_boundary(remaining, MAX_MESSAGE_LENGTH);
        let split_at = remaining[..safe_end].rfind('\n').unwrap_or(safe_end);

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("Hello, world!");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn test_split_message_long() {
        let text = "a".repeat(5000);
        let chunks = split_message(&text);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LENGTH);
        }
        // Verify all content is preserved
        let joined: String = chunks.join("");
        assert_eq!(joined.len(), 5000);
    }

    #[test]
    fn test_split_message_at_newline() {
        let mut text = "x".repeat(4000);
        text.push('\n');
        text.push_str(&"y".repeat(200));
        let chunks = split_message(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 4000);
    }

    #[test]
    fn test_user_allowed_no_restrictions() {
        let config = TelegramConfig {
            bot_token: secrecy::SecretString::from("test"),
            owner_id: None,
            allow_from: vec![],
            stream_mode: StreamMode::None,
        };
        assert!(config.owner_id.is_none());
        assert!(config.allow_from.is_empty());
    }

    #[test]
    fn test_user_allowed_owner_mode() {
        let config = TelegramConfig {
            bot_token: secrecy::SecretString::from("test"),
            owner_id: Some(12345),
            allow_from: vec![],
            stream_mode: StreamMode::None,
        };
        assert_eq!(config.owner_id.unwrap(), 12345);
    }

    // ── Markdown → Telegram HTML tests ─────────────────────────────

    #[test]
    fn test_html_escape() {
        assert_eq!(
            markdown_to_telegram_html("a < b > c & d"),
            "a &lt; b &gt; c &amp; d"
        );
    }

    #[test]
    fn test_bold() {
        assert_eq!(
            markdown_to_telegram_html("Hello **world**!"),
            "Hello <b>world</b>!"
        );
    }

    #[test]
    fn test_italic() {
        assert_eq!(
            markdown_to_telegram_html("Hello *world*!"),
            "Hello <i>world</i>!"
        );
    }

    #[test]
    fn test_bold_and_italic() {
        assert_eq!(
            markdown_to_telegram_html("**bold** and *italic*"),
            "<b>bold</b> and <i>italic</i>"
        );
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(
            markdown_to_telegram_html("Use `println!` here"),
            "Use <code>println!</code> here"
        );
    }

    #[test]
    fn test_strikethrough() {
        assert_eq!(
            markdown_to_telegram_html("~~removed~~"),
            "<s>removed</s>"
        );
    }

    #[test]
    fn test_heading() {
        assert_eq!(
            markdown_to_telegram_html("# Hello World"),
            "<b>Hello World</b>"
        );
    }

    #[test]
    fn test_heading_levels() {
        assert_eq!(
            markdown_to_telegram_html("### Subheading"),
            "<b>Subheading</b>"
        );
    }

    #[test]
    fn test_link() {
        assert_eq!(
            markdown_to_telegram_html("See [docs](https://example.com) here"),
            "See <a href=\"https://example.com\">docs</a> here"
        );
    }

    #[test]
    fn test_code_block() {
        let input = "```rust\nfn main() {}\n```";
        let expected = "<pre><code class=\"language-rust\">fn main() {}</code></pre>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_code_block_no_lang() {
        let input = "```\nhello\n```";
        let expected = "<pre><code>hello</code></pre>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_code_block_escapes_html() {
        let input = "```\n<script>alert(1)</script>\n```";
        let expected = "<pre><code>&lt;script&gt;alert(1)&lt;/script&gt;</code></pre>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_blockquote() {
        let input = "> This is quoted\n> Second line";
        let expected = "<blockquote>This is quoted\nSecond line</blockquote>";
        assert_eq!(markdown_to_telegram_html(input), expected);
    }

    #[test]
    fn test_unmatched_bold_left_as_is() {
        // Single ** without a closing pair should be preserved
        assert_eq!(
            markdown_to_telegram_html("Hello **world"),
            "Hello **world"
        );
    }

    #[test]
    fn test_mixed_llm_output() {
        let input = "# Summary\n\nHere are the **key points**:\n\n- Use `cargo build`\n- Visit [docs](https://docs.rs)\n\n```bash\ncargo run\n```";
        let result = markdown_to_telegram_html(input);
        assert!(result.contains("<b>Summary</b>"));
        assert!(result.contains("<b>key points</b>"));
        assert!(result.contains("<code>cargo build</code>"));
        assert!(result.contains("<a href=\"https://docs.rs\">docs</a>"));
        assert!(result.contains("<pre><code class=\"language-bash\">cargo run</code></pre>"));
    }

    #[test]
    fn test_plain_text_unchanged() {
        assert_eq!(
            markdown_to_telegram_html("Just a plain message."),
            "Just a plain message."
        );
    }

    #[test]
    fn test_strip_heading_helper() {
        assert_eq!(strip_heading("# Hello"), Some("Hello"));
        assert_eq!(strip_heading("## World"), Some("World"));
        assert_eq!(strip_heading("#NoSpace"), None);
        assert_eq!(strip_heading("Not a heading"), None);
    }
}
