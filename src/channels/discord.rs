//! Discord channel via Gateway WebSocket (Bot API).
//!
//! Uses raw WebSocket connection to the Discord Gateway for receiving
//! events and the REST API for sending messages. This approach avoids
//! the `serenity`/`poise` frameworks for a lighter footprint.
//!
//! ## Required Configuration
//!
//! - `DISCORD_BOT_TOKEN` — Bot token from Discord Developer Portal
//! - `DISCORD_GUILD_ID` (optional) — Restrict to a specific guild
//!
//! ## Architecture
//!
//! 1. GET `/gateway/bot` to get the WSS URL
//! 2. Connect to Gateway, receive Hello, send Identify
//! 3. Maintain heartbeat loop
//! 4. Receive MESSAGE_CREATE events
//! 5. Send responses via REST `/channels/{id}/messages`

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Channel name constant.
const NAME: &str = "discord";

/// Maximum message length for Discord.
const MAX_MESSAGE_LENGTH: usize = 2000;

/// Discord API base URL.
const API_BASE: &str = "https://discord.com/api/v10";

/// Gateway intents: GUILDS (1) + GUILD_MESSAGES (512) + MESSAGE_CONTENT (32768) + DIRECT_MESSAGES (4096)
const GATEWAY_INTENTS: u64 = 1 | 512 | 4096 | 32768;

// ── Configuration ───────────────────────────────────────────────────

/// Discord channel configuration.
#[derive(Debug, Clone)]
pub struct DiscordConfig {
    /// Bot token.
    pub bot_token: SecretString,
    /// Optional guild ID to restrict to.
    pub guild_id: Option<String>,
    /// Allowed channel IDs (empty = allow all).
    pub allow_from: Vec<String>,
}

// ── Discord Gateway types ───────────────────────────────────────────

/// Gateway payload (opcode-based dispatch).
#[derive(Debug, Deserialize)]
struct GatewayPayload {
    /// Opcode.
    op: u8,
    /// Event data.
    d: Option<serde_json::Value>,
    /// Sequence number.
    s: Option<u64>,
    /// Event name (for op 0).
    t: Option<String>,
}

/// Hello payload (op 10).
#[derive(Debug, Deserialize)]
struct HelloData {
    heartbeat_interval: u64,
}

/// Message Create event.
#[derive(Debug, Deserialize)]
struct MessageCreate {
    id: String,
    content: String,
    channel_id: String,
    author: MessageAuthor,
    guild_id: Option<String>,
    #[serde(default)]
    mention_everyone: bool,
    #[serde(default)]
    mentions: Vec<MessageAuthor>,
}

/// Message author.
#[derive(Debug, Clone, Deserialize)]
struct MessageAuthor {
    id: String,
    username: String,
    #[serde(default)]
    bot: bool,
}

// ── Channel implementation ──────────────────────────────────────────

/// Discord channel using Gateway WebSocket.
pub struct DiscordChannel {
    config: DiscordConfig,
    client: Client,
    shutdown: Arc<AtomicBool>,
}

impl DiscordChannel {
    /// Create a new Discord channel.
    pub fn new(config: DiscordConfig) -> Result<Self, ChannelError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("HTTP client: {e}"),
            })?;

        Ok(Self {
            config,
            client,
            shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Get the Gateway WebSocket URL.
    async fn get_gateway_url(client: &Client, bot_token: &str) -> Result<String, ChannelError> {
        let resp = client
            .get(format!("{API_BASE}/gateway/bot"))
            .header("Authorization", format!("Bot {bot_token}"))
            .send()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("GET /gateway/bot: {e}"),
            })?;

        let body: serde_json::Value =
            resp.json().await.map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("Parse /gateway/bot: {e}"),
            })?;

        let url =
            body.get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ChannelError::AuthFailed {
                    name: NAME.to_string(),
                    reason: "No gateway URL (invalid bot token?)".to_string(),
                })?;

        // Append version and encoding
        Ok(format!("{url}?v=10&encoding=json"))
    }

    /// Send a message via the REST API.
    async fn send_message(
        client: &Client,
        bot_token: &str,
        channel_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        let chunks = split_message(text);

        for chunk in chunks {
            let resp = client
                .post(format!("{API_BASE}/channels/{channel_id}/messages"))
                .header("Authorization", format!("Bot {bot_token}"))
                .json(&serde_json::json!({ "content": chunk }))
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("POST message: {e}"),
                })?;

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("Discord API: {body}"),
                });
            }
        }

        Ok(())
    }

    /// Send typing indicator.
    async fn send_typing(
        client: &Client,
        bot_token: &str,
        channel_id: &str,
    ) -> Result<(), ChannelError> {
        let _ = client
            .post(format!("{API_BASE}/channels/{channel_id}/typing"))
            .header("Authorization", format!("Bot {bot_token}"))
            .send()
            .await;
        Ok(())
    }

    /// Build the Identify payload.
    fn identify_payload(bot_token: &str) -> serde_json::Value {
        serde_json::json!({
            "op": 2,
            "d": {
                "token": bot_token,
                "intents": GATEWAY_INTENTS,
                "properties": {
                    "os": std::env::consts::OS,
                    "browser": "ironclaw",
                    "device": "ironclaw"
                }
            }
        })
    }
}

#[async_trait]
impl Channel for DiscordChannel {
    fn name(&self) -> &str {
        NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);

        let bot_token = self.config.bot_token.expose_secret().to_string();
        let guild_id = self.config.guild_id.clone();
        let allow_from = self.config.allow_from.clone();
        let client = self.client.clone();
        let shutdown = self.shutdown.clone();

        // Validate token and get bot user ID
        let me_resp = client
            .get(format!("{API_BASE}/users/@me"))
            .header("Authorization", format!("Bot {bot_token}"))
            .send()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("GET /users/@me: {e}"),
            })?;

        let me: serde_json::Value = me_resp.json().await.unwrap_or_default();
        let bot_user_id = me
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::AuthFailed {
                name: NAME.to_string(),
                reason: "Invalid bot token".to_string(),
            })?
            .to_string();

        let bot_name = me
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or("IronClaw");
        tracing::info!("Discord bot connected as {} ({})", bot_name, bot_user_id);

        // Spawn Gateway connection
        tokio::spawn(async move {
            let sequence = Arc::new(AtomicU64::new(0));

            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                // Get Gateway URL
                let ws_url = match Self::get_gateway_url(&client, &bot_token).await {
                    Ok(url) => url,
                    Err(e) => {
                        tracing::error!("Discord: failed to get gateway URL: {e}");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                // Connect
                let ws_stream = match connect_async(&ws_url).await {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        tracing::error!("Discord: WebSocket connect failed: {e}");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                tracing::info!("Discord Gateway connected");
                let (mut ws_write, mut ws_read) = ws_stream.split();

                // Wait for Hello (op 10)
                let heartbeat_interval = match ws_read.next().await {
                    Some(Ok(WsMessage::Text(text))) => {
                        let payload: GatewayPayload = match serde_json::from_str(&text) {
                            Ok(p) => p,
                            Err(_) => {
                                tracing::error!("Discord: invalid Hello");
                                break;
                            }
                        };
                        if payload.op != 10 {
                            tracing::error!("Discord: expected Hello, got op {}", payload.op);
                            break;
                        }
                        let hello: HelloData = serde_json::from_value(
                            payload.d.unwrap_or_default(),
                        )
                        .unwrap_or(HelloData {
                            heartbeat_interval: 45000,
                        });
                        hello.heartbeat_interval
                    }
                    _ => {
                        tracing::error!("Discord: no Hello received");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                };

                // Send Identify
                let identify = Self::identify_payload(&bot_token);
                if ws_write
                    .send(WsMessage::Text(identify.to_string().into()))
                    .await
                    .is_err()
                {
                    tracing::error!("Discord: failed to send Identify");
                    continue;
                }

                // Spawn heartbeat task
                let seq_heartbeat = sequence.clone();
                let shutdown_hb = shutdown.clone();
                let (hb_tx, mut hb_rx) = mpsc::channel::<String>(1);
                tokio::spawn(async move {
                    let mut interval =
                        tokio::time::interval(Duration::from_millis(heartbeat_interval));
                    loop {
                        interval.tick().await;
                        if shutdown_hb.load(Ordering::Relaxed) {
                            break;
                        }
                        let seq = seq_heartbeat.load(Ordering::Relaxed);
                        let hb = if seq == 0 {
                            r#"{"op":1,"d":null}"#.to_string()
                        } else {
                            format!(r#"{{"op":1,"d":{seq}}}"#)
                        };
                        if hb_tx.send(hb).await.is_err() {
                            break;
                        }
                    }
                });

                // Process events
                loop {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    let msg = tokio::select! {
                        msg = ws_read.next() => msg,
                        hb = hb_rx.recv() => {
                            if let Some(hb) = hb {
                                let _ = ws_write.send(WsMessage::Text(hb.into())).await;
                            }
                            continue;
                        }
                    };

                    let text = match msg {
                        Some(Ok(WsMessage::Text(t))) => t,
                        Some(Ok(WsMessage::Close(_))) | None => {
                            tracing::warn!("Discord: Gateway closed, reconnecting...");
                            break;
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => {
                            tracing::error!("Discord WS error: {e}");
                            break;
                        }
                    };

                    let payload: GatewayPayload = match serde_json::from_str(&text) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };

                    // Update sequence
                    if let Some(s) = payload.s {
                        sequence.store(s, Ordering::Relaxed);
                    }

                    match payload.op {
                        // Dispatch (events)
                        0 => {
                            let event_name = match &payload.t {
                                Some(t) => t.as_str(),
                                None => continue,
                            };

                            if event_name != "MESSAGE_CREATE" {
                                continue;
                            }

                            let data = match payload.d {
                                Some(d) => d,
                                None => continue,
                            };

                            let msg: MessageCreate = match serde_json::from_value(data) {
                                Ok(m) => m,
                                Err(_) => continue,
                            };

                            // Skip bot messages
                            if msg.author.bot {
                                continue;
                            }

                            // Skip own messages
                            if msg.author.id == bot_user_id {
                                continue;
                            }

                            // Guild filter
                            if let Some(ref target_guild) = guild_id {
                                if msg.guild_id.as_deref() != Some(target_guild.as_str()) {
                                    continue;
                                }
                            }

                            // Channel allow-list
                            if !allow_from.is_empty()
                                && !allow_from.iter().any(|a| a == "*" || a == &msg.channel_id)
                            {
                                continue;
                            }

                            // Strip bot mention
                            let clean = msg
                                .content
                                .replace(&format!("<@{bot_user_id}>"), "")
                                .replace(&format!("<@!{bot_user_id}>"), "")
                                .trim()
                                .to_string();

                            if clean.is_empty() {
                                continue;
                            }

                            let incoming = IncomingMessage::new(NAME, &msg.author.id, &clean)
                                .with_user_name(msg.author.username.clone())
                                .with_metadata(serde_json::json!({
                                    "channel_id": msg.channel_id,
                                    "message_id": msg.id,
                                    "guild_id": msg.guild_id,
                                }));

                            if tx.send(incoming).await.is_err() {
                                tracing::warn!("Discord channel receiver dropped");
                                return;
                            }
                        }
                        // Heartbeat ACK
                        11 => {}
                        // Heartbeat request
                        1 => {
                            let seq = sequence.load(Ordering::Relaxed);
                            let hb = if seq == 0 {
                                r#"{"op":1,"d":null}"#.to_string()
                            } else {
                                format!(r#"{{"op":1,"d":{seq}}}"#)
                            };
                            let _ = ws_write.send(WsMessage::Text(hb.into())).await;
                        }
                        // Reconnect
                        7 => {
                            tracing::info!("Discord: received reconnect request");
                            break;
                        }
                        // Invalid session
                        9 => {
                            tracing::warn!("Discord: invalid session, re-identifying");
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            break;
                        }
                        _ => {}
                    }
                }

                if !shutdown.load(Ordering::Relaxed) {
                    tracing::info!("Discord: reconnecting in 5s...");
                    tokio::time::sleep(Duration::from_secs(5)).await;
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
        let channel_id = msg
            .metadata
            .get("channel_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Missing channel_id in metadata".to_string(),
            })?;

        let bot_token = self.config.bot_token.expose_secret();
        Self::send_message(&self.client, bot_token, channel_id, &response.content).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let channel_id = match metadata.get("channel_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return Ok(()),
        };

        match status {
            StatusUpdate::Thinking(_) | StatusUpdate::ToolStarted { .. } => {
                let bot_token = self.config.bot_token.expose_secret();
                Self::send_typing(&self.client, bot_token, channel_id).await?;
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
        let bot_token = self.config.bot_token.expose_secret();
        Self::send_message(&self.client, bot_token, user_id, &response.content).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let resp = self
            .client
            .get(format!("{API_BASE}/users/@me"))
            .header(
                "Authorization",
                format!("Bot {}", self.config.bot_token.expose_secret()),
            )
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

/// Split a long message into chunks for Discord's 2000-char limit.
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
        let chunks = split_message("Hello!");
        assert_eq!(chunks, vec!["Hello!"]);
    }

    #[test]
    fn test_split_message_long() {
        let text = "a".repeat(3000);
        let chunks = split_message(&text);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn test_split_at_newline() {
        let mut text = "x".repeat(1900);
        text.push('\n');
        text.push_str(&"y".repeat(200));
        let chunks = split_message(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 1900);
    }

    #[test]
    fn test_identify_payload() {
        let payload = DiscordChannel::identify_payload("fake_token");
        assert_eq!(payload["op"], 2);
        assert_eq!(payload["d"]["token"], "fake_token");
        assert_eq!(payload["d"]["intents"], GATEWAY_INTENTS);
    }

    #[test]
    fn test_gateway_intents() {
        // GUILDS (1) + GUILD_MESSAGES (512) + DIRECT_MESSAGES (4096) + MESSAGE_CONTENT (32768)
        assert_eq!(GATEWAY_INTENTS, 1 | 512 | 4096 | 32768);
    }
}
