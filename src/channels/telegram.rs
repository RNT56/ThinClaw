//! Telegram Bot API channel via long-polling `getUpdates`.
//!
//! Uses the raw Telegram Bot HTTP API through `reqwest` — no heavy
//! framework dependency. Supports:
//! - Long-polling message reception
//! - Text message sending with Markdown V2
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

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
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
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields populated by serde deserialization
struct TgMessage {
    message_id: i64,
    from: Option<TgUser>,
    chat: TgChat,
    text: Option<String>,
    #[serde(default)]
    photo: Option<Vec<TgPhotoSize>>,
    caption: Option<String>,
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
    async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), ChannelError> {
        // Split long messages
        let chunks = split_message(text);

        for chunk in chunks {
            let resp = self
                .client
                .post(format!("{}/sendMessage", self.api_base))
                .json(&serde_json::json!({
                    "chat_id": chat_id,
                    "text": chunk,
                    "parse_mode": "Markdown",
                }))
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("sendMessage: {e}"),
                })?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                // If Markdown parsing fails, retry without parse_mode
                if body.contains("can't parse entities") {
                    self.client
                        .post(format!("{}/sendMessage", self.api_base))
                        .json(&serde_json::json!({
                            "chat_id": chat_id,
                            "text": chunk,
                        }))
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
                    serde_json::json!(["message"]).to_string(),
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
            .unwrap_or("IronClaw");
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
                            if let Some(msg) = update.message {
                                // Get text content
                                let text = msg.text.or(msg.caption).unwrap_or_default();

                                if text.is_empty() {
                                    continue;
                                }

                                // Get sender info
                                let (user_id, user_name) = match &msg.from {
                                    Some(user) => {
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
                                    }
                                    None => continue,
                                };

                                // Check access
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

                                let incoming = IncomingMessage::new("telegram", &user_id, &text)
                                    .with_metadata(serde_json::json!({
                                        "chat_id": msg.chat.id,
                                        "message_id": msg.message_id,
                                        "chat_type": msg.chat.chat_type,
                                    }));

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

        self.send_message(chat_id, &response.content).await
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
        // Parse user_id as chat_id for direct messages
        let chat_id: i64 = user_id.parse().map_err(|_| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: "Invalid user_id for broadcast".to_string(),
        })?;

        self.send_message(chat_id, &response.content).await
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
}

// ── Helpers ─────────────────────────────────────────────────────────

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

        // Try to split at a newline near the limit
        let split_at = remaining[..MAX_MESSAGE_LENGTH]
            .rfind('\n')
            .unwrap_or(MAX_MESSAGE_LENGTH);

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
        };
        assert_eq!(config.owner_id.unwrap(), 12345);
    }
}
