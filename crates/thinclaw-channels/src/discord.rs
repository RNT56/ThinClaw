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
use tokio::sync::{Mutex, Notify, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message as WsMessage, protocol::WebSocketConfig},
};

use thinclaw_channels_core::{
    Channel, DraftReplyState, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
    StreamMode,
};
use thinclaw_media::MediaContent;
use thinclaw_tools_core::{OutboundUrlGuardOptions, validate_outbound_url_pinned_async};
use thinclaw_types::error::ChannelError;

use crate::util::floor_char_boundary;

/// Channel name constant.
const NAME: &str = "discord";

/// Maximum message length for Discord.
const MAX_MESSAGE_LENGTH: usize = 2000;

/// Discord API base URL.
const API_BASE: &str = "https://discord.com/api/v10";

const CHANNEL_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Upper bound on the exponential reconnect backoff.
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(60);

/// Fallback heartbeat interval (ms) if the Gateway sends an out-of-range value.
const DEFAULT_HEARTBEAT_INTERVAL_MS: u64 = 45_000;

/// Times a rate-limited (429) Discord REST call is retried before giving up.
const MAX_REST_RETRIES: u32 = 3;
const MAX_DISCORD_JSON_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISCORD_ERROR_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_DISCORD_GATEWAY_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DISCORD_ATTACHMENTS: usize = 10;
const MAX_DISCORD_IDENTIFIER_BYTES: usize = 32;
const MAX_DISCORD_TOKEN_BYTES: usize = 64 * 1024;

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
    /// Stream mode for progressive message rendering.
    pub stream_mode: StreamMode,
}

fn valid_discord_snowflake(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_DISCORD_IDENTIFIER_BYTES
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|value| value > 0)
}

fn validate_discord_config(config: &DiscordConfig) -> Result<(), ChannelError> {
    let token = config.bot_token.expose_secret();
    if token.is_empty()
        || token.len() > MAX_DISCORD_TOKEN_BYTES
        || token.chars().any(char::is_control)
        || config
            .guild_id
            .as_deref()
            .is_some_and(|value| !valid_discord_snowflake(value))
        || config.allow_from.len() > 1024
        || config
            .allow_from
            .iter()
            .any(|value| value != "*" && !valid_discord_snowflake(value))
    {
        return Err(ChannelError::StartupFailed {
            name: NAME.to_string(),
            reason: "Discord configuration is malformed or oversized".to_string(),
        });
    }
    Ok(())
}

fn normalize_discord_gateway_url(value: &str) -> Result<String, ChannelError> {
    if value.is_empty() || value.len() > 4096 {
        return Err(ChannelError::StartupFailed {
            name: NAME.to_string(),
            reason: "Discord returned an invalid Gateway URL".to_string(),
        });
    }
    let mut url = url::Url::parse(value).map_err(|_| ChannelError::StartupFailed {
        name: NAME.to_string(),
        reason: "Discord returned an invalid Gateway URL".to_string(),
    })?;
    if url.scheme() != "wss"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str() != Some("gateway.discord.gg")
        || url.port().is_some_and(|port| port != 443)
        || url.fragment().is_some()
    {
        return Err(ChannelError::StartupFailed {
            name: NAME.to_string(),
            reason: "Discord returned an untrusted Gateway URL".to_string(),
        });
    }
    url.set_query(None);
    url.query_pairs_mut()
        .append_pair("v", "10")
        .append_pair("encoding", "json");
    Ok(url.to_string())
}

fn discord_gateway_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(64 * 1024)
        .write_buffer_size(64 * 1024)
        .max_write_buffer_size(512 * 1024)
        .max_message_size(Some(MAX_DISCORD_GATEWAY_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_DISCORD_GATEWAY_MESSAGE_BYTES))
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
#[allow(dead_code)] // Fields populated by serde deserialization
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
    /// File attachments (images, docs, audio, video).
    #[serde(default)]
    attachments: Vec<DiscordAttachment>,
}

/// Discord attachment from a MESSAGE_CREATE event.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct DiscordAttachment {
    id: String,
    filename: String,
    /// MIME type (e.g. "image/png").
    content_type: Option<String>,
    /// File size in bytes.
    size: u64,
    /// CDN URL to download from.
    url: String,
}

/// Maximum single attachment size we'll download (20 MB).
const MAX_DISCORD_ATTACHMENT_SIZE: u64 = 20 * 1024 * 1024;
const MAX_DISCORD_TOTAL_ATTACHMENT_SIZE: usize = 40 * 1024 * 1024;

/// Message author.
#[derive(Debug, Clone, Deserialize)]
struct MessageAuthor {
    id: String,
    username: String,
    #[serde(default)]
    bot: bool,
}

fn validate_discord_message(message: &MessageCreate) -> bool {
    valid_discord_snowflake(&message.id)
        && valid_discord_snowflake(&message.channel_id)
        && message
            .guild_id
            .as_deref()
            .is_none_or(valid_discord_snowflake)
        && valid_discord_snowflake(&message.author.id)
        && !message.author.username.is_empty()
        && message.author.username.len() <= 256
        && !message.author.username.chars().any(char::is_control)
        && message.content.len() <= 64 * 1024
        && message.mentions.len() <= 256
        && message.mentions.iter().all(|author| {
            valid_discord_snowflake(&author.id)
                && author.username.len() <= 256
                && !author.username.chars().any(char::is_control)
        })
        && message.attachments.len() <= MAX_DISCORD_ATTACHMENTS
        && message.attachments.iter().all(|attachment| {
            valid_discord_snowflake(&attachment.id)
                && !attachment.filename.is_empty()
                && attachment.filename.len() <= 255
                && !attachment.filename.chars().any(char::is_control)
                && !attachment.filename.contains(['/', '\\'])
                && attachment.url.len() <= 16 * 1024
                && attachment.content_type.as_ref().is_none_or(|value| {
                    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
                })
        })
}

// ── Channel implementation ──────────────────────────────────────────

/// Discord channel using Gateway WebSocket.
pub struct DiscordChannel {
    config: DiscordConfig,
    client: Client,
    shutdown: Arc<AtomicBool>,
    shutdown_notify: Arc<Notify>,
    gateway_task: Mutex<Option<JoinHandle<()>>>,
}

impl DiscordChannel {
    /// Create a new Discord channel.
    pub fn new(config: DiscordConfig) -> Result<Self, ChannelError> {
        validate_discord_config(&config)?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("HTTP client: {e}"),
            })?;

        Ok(Self {
            config,
            client,
            shutdown: Arc::new(AtomicBool::new(false)),
            shutdown_notify: Arc::new(Notify::new()),
            gateway_task: Mutex::new(None),
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

        if !resp.status().is_success() {
            return Err(ChannelError::AuthFailed {
                name: NAME.to_string(),
                reason: format!("Discord Gateway discovery returned {}", resp.status()),
            });
        }

        let body: serde_json::Value =
            crate::response::bounded_json(resp, MAX_DISCORD_JSON_RESPONSE_BYTES)
                .await
                .map_err(|e| ChannelError::StartupFailed {
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

        normalize_discord_gateway_url(url)
    }

    /// Send a message via the REST API.
    async fn send_message(
        client: &Client,
        bot_token: &str,
        channel_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        if !valid_discord_snowflake(channel_id) {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord channel ID is malformed".to_string(),
            });
        }
        let chunks = split_message(text);

        for chunk in chunks {
            let resp = send_rest(|| {
                client
                    .post(format!("{API_BASE}/channels/{channel_id}/messages"))
                    .header("Authorization", format!("Bot {bot_token}"))
                    .json(&serde_json::json!({ "content": chunk.as_str() }))
            })
            .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                return Err(ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("Discord message API returned {status}"),
                });
            }
        }

        Ok(())
    }

    async fn send_message_payload(
        client: &Client,
        bot_token: &str,
        channel_id: &str,
        response: &OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if !valid_discord_snowflake(channel_id) {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord channel ID is malformed".to_string(),
            });
        }
        if response.attachments.is_empty() {
            return Self::send_message(client, bot_token, channel_id, &response.content).await;
        }

        if response.attachments.len() > MAX_DISCORD_ATTACHMENTS {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord response contains too many attachments".to_string(),
            });
        }
        let total_attachment_bytes = response
            .attachments
            .iter()
            .try_fold(0usize, |total, attachment| {
                total.checked_add(attachment.data.len())
            })
            .ok_or_else(|| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord response attachment size overflow".to_string(),
            })?;
        if total_attachment_bytes > MAX_DISCORD_TOTAL_ATTACHMENT_SIZE {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord response attachments exceed the configured total size limit"
                    .to_string(),
            });
        }

        // Keep text within Discord's 2,000-character limit and avoid repeating
        // it once for every attachment.
        if !response.content.trim().is_empty() {
            Self::send_message(client, bot_token, channel_id, &response.content).await?;
        }

        for attachment in &response.attachments {
            if attachment.data.len() > MAX_DISCORD_ATTACHMENT_SIZE as usize {
                return Err(ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: "Discord attachment exceeds the configured size limit".to_string(),
                });
            }
            let filename = attachment
                .filename
                .as_deref()
                .unwrap_or("attachment")
                .to_string();
            if filename.is_empty()
                || filename.len() > 255
                || filename.chars().any(char::is_control)
                || filename.contains(['/', '\\'])
            {
                return Err(ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: "Discord attachment filename is malformed".to_string(),
                });
            }
            let part = reqwest::multipart::Part::bytes(attachment.data.clone())
                .file_name(filename.clone())
                .mime_str(&attachment.mime_type)
                .map_err(|e| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("Invalid attachment MIME {}: {e}", attachment.mime_type),
                })?;
            let payload = serde_json::json!({
                "content": "",
                "attachments": [{"id": 0, "filename": filename}],
            });
            let form = reqwest::multipart::Form::new()
                .text("payload_json", payload.to_string())
                .part("files[0]", part);
            let resp = client
                .post(format!("{API_BASE}/channels/{channel_id}/messages"))
                .header("Authorization", format!("Bot {bot_token}"))
                .multipart(form)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("POST attachment message: {e}"),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                return Err(ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("Discord attachment API returned {status}"),
                });
            }
        }
        Ok(())
    }

    /// Send a new message and return its message ID.
    async fn send_message_with_id(
        client: &Client,
        bot_token: &str,
        channel_id: &str,
        text: &str,
    ) -> Result<String, ChannelError> {
        if !valid_discord_snowflake(channel_id) {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord channel ID is malformed".to_string(),
            });
        }
        let resp = send_rest(|| {
            client
                .post(format!("{API_BASE}/channels/{channel_id}/messages"))
                .header("Authorization", format!("Bot {bot_token}"))
                .json(&serde_json::json!({ "content": text }))
        })
        .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("Discord message API returned {status}"),
            });
        }

        let body: serde_json::Value =
            crate::response::bounded_json(resp, MAX_DISCORD_JSON_RESPONSE_BYTES)
                .await
                .map_err(|error| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("Invalid Discord message response: {error}"),
                })?;
        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|id| valid_discord_snowflake(id))
            .ok_or_else(|| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord message response did not contain a valid ID".to_string(),
            })?;
        Ok(id.to_string())
    }

    /// Edit an existing message.
    async fn edit_message(
        client: &Client,
        bot_token: &str,
        channel_id: &str,
        message_id: &str,
        text: &str,
    ) -> Result<(), ChannelError> {
        if !valid_discord_snowflake(channel_id) || !valid_discord_snowflake(message_id) {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord message identifiers are malformed".to_string(),
            });
        }
        let resp = send_rest(|| {
            client
                .patch(format!(
                    "{API_BASE}/channels/{channel_id}/messages/{message_id}"
                ))
                .header("Authorization", format!("Bot {bot_token}"))
                .json(&serde_json::json!({ "content": text }))
        })
        .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("Discord edit API returned {status}"),
            });
        }

        Ok(())
    }

    /// Send typing indicator.
    async fn send_typing(
        client: &Client,
        bot_token: &str,
        channel_id: &str,
    ) -> Result<(), ChannelError> {
        if !valid_discord_snowflake(channel_id) {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Discord channel ID is malformed".to_string(),
            });
        }
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
                    "browser": "thinclaw",
                    "device": "thinclaw"
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

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "Discord supports markdown. Use ``` for code blocks. Embeds are available for media. Use <@user_id> for mentions."
                .to_string(),
        )
    }

    fn config_schema(&self) -> Option<thinclaw_channels_core::ConfigSchema> {
        use thinclaw_channels_core::{ConfigField, ConfigSchema};
        Some(ConfigSchema {
            channel_id: "discord".to_string(),
            channel_name: "Discord".to_string(),
            fields: vec![
                ConfigField {
                    id: "allow_from".to_string(),
                    label: "Allowed channel IDs".to_string(),
                    field_type: "textarea".to_string(),
                    required: false,
                    help_text: Some(
                        "One Discord channel ID per line. Empty allows every channel and DM."
                            .to_string(),
                    ),
                    default_value: None,
                    options: None,
                },
                ConfigField {
                    id: "guild_id".to_string(),
                    label: "Guild ID".to_string(),
                    field_type: "text".to_string(),
                    required: false,
                    help_text: Some(
                        "Restrict the bot to a single guild (server). Empty allows all guilds."
                            .to_string(),
                    ),
                    default_value: None,
                    options: None,
                },
            ],
            help: Some(
                "Configure which Discord users and guild the agent responds to.".to_string(),
            ),
        })
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);
        if let Some(handle) = self.gateway_task.lock().await.take() {
            self.shutdown.store(true, Ordering::Relaxed);
            self.shutdown_notify.notify_waiters();
            drain_channel_task(handle, NAME).await;
        }
        self.shutdown.store(false, Ordering::Relaxed);

        let bot_token = self.config.bot_token.expose_secret().to_string();
        let guild_id = self.config.guild_id.clone();
        let allow_from = self.config.allow_from.clone();
        let client = self.client.clone();
        let shutdown = self.shutdown.clone();
        let shutdown_notify = Arc::clone(&self.shutdown_notify);

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

        if !me_resp.status().is_success() {
            return Err(ChannelError::AuthFailed {
                name: NAME.to_string(),
                reason: format!("Discord identity API returned {}", me_resp.status()),
            });
        }

        let me: serde_json::Value =
            crate::response::bounded_json(me_resp, MAX_DISCORD_JSON_RESPONSE_BYTES)
                .await
                .map_err(|error| ChannelError::AuthFailed {
                    name: NAME.to_string(),
                    reason: format!("Invalid Discord identity response: {error}"),
                })?;
        let bot_user_id = me
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|id| valid_discord_snowflake(id))
            .ok_or_else(|| ChannelError::AuthFailed {
                name: NAME.to_string(),
                reason: "Invalid bot token".to_string(),
            })?
            .to_string();

        let bot_name = me
            .get("username")
            .and_then(|v| v.as_str())
            .filter(|value| value.len() <= 256 && !value.chars().any(char::is_control))
            .unwrap_or("ThinClaw");
        tracing::info!("Discord bot connected as {} ({})", bot_name, bot_user_id);

        // Spawn Gateway connection
        let handle = tokio::spawn(async move {
            let sequence = Arc::new(AtomicU64::new(0));
            // Resume state captured from READY; lets a reconnect replay missed
            // events instead of re-Identifying (which burns the daily quota).
            let mut session_id: Option<String> = None;
            let mut resume_gateway_url: Option<String> = None;
            let mut reconnect_backoff = Duration::from_secs(1);

            'gateway: loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                // Resume only when we hold a full session and have seen events.
                let resuming = session_id.is_some()
                    && resume_gateway_url.is_some()
                    && sequence.load(Ordering::Relaxed) > 0;

                // Resume uses the session's dedicated gateway URL; a fresh
                // connect discovers one via GET /gateway/bot.
                let ws_url = if resuming {
                    match normalize_discord_gateway_url(
                        resume_gateway_url.as_deref().unwrap_or_default(),
                    ) {
                        Ok(url) => url,
                        Err(error) => {
                            tracing::warn!(%error, "Discord supplied an invalid resume Gateway URL; rediscovering");
                            session_id = None;
                            resume_gateway_url = None;
                            continue;
                        }
                    }
                } else {
                    match Self::get_gateway_url(&client, &bot_token).await {
                        Ok(url) => url,
                        Err(e) => {
                            tracing::error!("Discord: failed to get gateway URL: {e}");
                            if sleep_backoff(&shutdown, &shutdown_notify, &mut reconnect_backoff)
                                .await
                            {
                                break;
                            }
                            continue;
                        }
                    }
                };

                // Connect
                let ws_stream = match connect_async_with_config(
                    &ws_url,
                    Some(discord_gateway_websocket_config()),
                    false,
                )
                .await
                {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        tracing::error!("Discord: WebSocket connect failed: {e}");
                        // A bad resume URL must not trap us; force rediscovery.
                        if resuming {
                            session_id = None;
                            resume_gateway_url = None;
                        }
                        if sleep_backoff(&shutdown, &shutdown_notify, &mut reconnect_backoff).await
                        {
                            break;
                        }
                        continue;
                    }
                };

                tracing::info!(resuming, "Discord Gateway connected");
                let (mut ws_write, mut ws_read) = ws_stream.split();

                // Wait for Hello (op 10)
                let hello_msg = tokio::select! {
                    msg = ws_read.next() => msg,
                    _ = tokio::time::sleep(Duration::from_secs(30)) => {
                        tracing::warn!("Discord: timed out waiting for Gateway Hello");
                        if sleep_backoff(&shutdown, &shutdown_notify, &mut reconnect_backoff).await {
                            break 'gateway;
                        }
                        continue;
                    }
                    _ = shutdown_notify.notified() => {
                        if shutdown.load(Ordering::Relaxed) {
                            break 'gateway;
                        }
                        continue;
                    }
                };
                let heartbeat_interval = match hello_msg {
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
                            heartbeat_interval: DEFAULT_HEARTBEAT_INTERVAL_MS,
                        });
                        sanitize_heartbeat_interval(hello.heartbeat_interval)
                    }
                    _ => {
                        tracing::error!("Discord: no Hello received");
                        if sleep_backoff(&shutdown, &shutdown_notify, &mut reconnect_backoff).await
                        {
                            break;
                        }
                        continue;
                    }
                };

                // Send Resume (op 6) to replay the existing session, or Identify
                // (op 2) to start a fresh one.
                let handshake = if resuming {
                    serde_json::json!({
                        "op": 6,
                        "d": {
                            "token": bot_token,
                            "session_id": session_id,
                            "seq": sequence.load(Ordering::Relaxed),
                        }
                    })
                } else {
                    Self::identify_payload(&bot_token)
                };
                if ws_write
                    .send(WsMessage::Text(handshake.to_string().into()))
                    .await
                    .is_err()
                {
                    tracing::error!("Discord: failed to send handshake");
                    if sleep_backoff(&shutdown, &shutdown_notify, &mut reconnect_backoff).await {
                        break;
                    }
                    continue;
                }

                // Handshake sent successfully — the connection is healthy, so
                // reset the reconnect backoff for the next disconnect.
                reconnect_backoff = Duration::from_secs(1);
                // Tracks whether the last heartbeat we sent has been ACKed; a
                // missed ACK means the socket is a zombie and must be replaced.
                let mut awaiting_heartbeat_ack = false;

                let mut heartbeat_tick =
                    tokio::time::interval(Duration::from_millis(heartbeat_interval));

                // Process events
                loop {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    let msg = tokio::select! {
                        msg = ws_read.next() => msg,
                        _ = heartbeat_tick.tick() => {
                            // Discord ACKs every heartbeat (op 11). If the prior
                            // beat was never ACKed, the connection is a zombie
                            // (half-open TCP) — tear it down and reconnect
                            // instead of writing into a dead socket.
                            if awaiting_heartbeat_ack {
                                tracing::warn!(
                                    "Discord: heartbeat not ACKed; connection is a zombie, reconnecting"
                                );
                                break;
                            }
                            let seq = sequence.load(Ordering::Relaxed);
                            let hb = if seq == 0 {
                                r#"{"op":1,"d":null}"#.to_string()
                            } else {
                                format!(r#"{{"op":1,"d":{seq}}}"#)
                            };
                            if ws_write.send(WsMessage::Text(hb.into())).await.is_err() {
                                tracing::warn!("Discord: failed to send heartbeat");
                                break;
                            }
                            awaiting_heartbeat_ack = true;
                            continue;
                        }
                        _ = shutdown_notify.notified() => {
                            if shutdown.load(Ordering::Relaxed) {
                                break;
                            }
                            continue;
                        }
                    };

                    let text = match msg {
                        Some(Ok(WsMessage::Text(t))) => t,
                        Some(Ok(WsMessage::Close(frame))) => {
                            let code = frame.as_ref().map(|f| u16::from(f.code)).unwrap_or(0);
                            let reason = frame
                                .as_ref()
                                .map(|f| {
                                    let value = f.reason.as_ref();
                                    let end = floor_char_boundary(value, value.len().min(1024));
                                    value[..end].to_string()
                                })
                                .unwrap_or_default();
                            if is_fatal_close_code(code) {
                                tracing::error!(
                                    code,
                                    reason = %reason,
                                    "Discord: Gateway closed with a non-recoverable code; not reconnecting (check bot token/intents)"
                                );
                                break 'gateway;
                            }
                            tracing::warn!(code, reason = %reason, "Discord: Gateway closed, reconnecting...");
                            // A close often invalidates the session; a fresh
                            // Identify is safest unless Discord told us to resume.
                            break;
                        }
                        None => {
                            tracing::warn!("Discord: Gateway stream ended, reconnecting...");
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
                            let event_name = match payload.t.as_deref() {
                                Some(t) if t.len() <= 64 && !t.chars().any(char::is_control) => t,
                                None => continue,
                                Some(_) => continue,
                            };

                            // Capture resume state from READY so a later
                            // reconnect can op-6 Resume instead of re-Identify.
                            if event_name == "READY" {
                                if let Some(d) = payload.d.as_ref() {
                                    session_id = d
                                        .get("session_id")
                                        .and_then(|v| v.as_str())
                                        .filter(|value| {
                                            !value.is_empty()
                                                && value.len() <= 1024
                                                && !value.chars().any(char::is_control)
                                        })
                                        .map(String::from);
                                    resume_gateway_url = d
                                        .get("resume_gateway_url")
                                        .and_then(|v| v.as_str())
                                        .and_then(|value| {
                                            normalize_discord_gateway_url(value).ok()
                                        });
                                    if session_id.is_none() || resume_gateway_url.is_none() {
                                        session_id = None;
                                        resume_gateway_url = None;
                                    }
                                }
                                continue;
                            }

                            if event_name != "MESSAGE_CREATE" {
                                // RESUMED and other dispatches are handled by
                                // advancing the sequence counter above.
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
                            if !validate_discord_message(&msg) {
                                tracing::warn!(
                                    "Discord: ignoring malformed or oversized message event"
                                );
                                continue;
                            }

                            // Skip bot messages
                            if msg.author.bot {
                                continue;
                            }

                            // Skip own messages
                            if msg.author.id == bot_user_id {
                                continue;
                            }

                            // Guild filter
                            if let Some(ref target_guild) = guild_id
                                && msg.guild_id.as_deref() != Some(target_guild.as_str())
                            {
                                continue;
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

                            // Download media attachments from Discord CDN
                            let attachments = download_discord_attachments(&msg.attachments).await;

                            // Skip messages with no text AND no media
                            if clean.is_empty() && attachments.is_empty() {
                                continue;
                            }

                            let content = if clean.is_empty() && !attachments.is_empty() {
                                "[Media received — please analyze the attached content]".to_string()
                            } else {
                                clean
                            };

                            let incoming = IncomingMessage::new(NAME, &msg.author.id, &content)
                                .with_user_name(msg.author.username.clone())
                                .with_metadata(serde_json::json!({
                                    "channel_id": msg.channel_id,
                                    "message_id": msg.id,
                                    "guild_id": msg.guild_id,
                                }))
                                .with_attachments(attachments);

                            if tx.send(incoming).await.is_err() {
                                tracing::warn!("Discord channel receiver dropped");
                                return;
                            }
                        }
                        // Heartbeat ACK — the connection is alive.
                        11 => {
                            awaiting_heartbeat_ack = false;
                        }
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
                        // Reconnect — Discord asks us to reconnect and resume.
                        7 => {
                            tracing::info!("Discord: received reconnect request");
                            break;
                        }
                        // Invalid session — `d: true` means resumable.
                        9 => {
                            let resumable = payload
                                .d
                                .as_ref()
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if !resumable {
                                tracing::warn!(
                                    "Discord: session invalidated, re-identifying fresh"
                                );
                                session_id = None;
                                resume_gateway_url = None;
                                sequence.store(0, Ordering::Relaxed);
                            } else {
                                tracing::info!("Discord: session invalid but resumable");
                            }
                            break;
                        }
                        _ => {}
                    }
                }

                if !shutdown.load(Ordering::Relaxed)
                    && sleep_backoff(&shutdown, &shutdown_notify, &mut reconnect_backoff).await
                {
                    break;
                }
            }
        });
        *self.gateway_task.lock().await = Some(handle);

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
        Self::send_message_payload(&self.client, bot_token, channel_id, &response).await
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
        // Discord channel/user IDs are snowflakes (large integers).
        // Skip gracefully if not numeric (e.g. "default").
        if user_id.parse::<u64>().is_err() {
            tracing::debug!(
                recipient = user_id,
                "Discord: skipping broadcast — recipient is not a valid snowflake ID"
            );
            return Ok(());
        }
        let bot_token = self.config.bot_token.expose_secret();
        Self::send_message_payload(&self.client, bot_token, user_id, &response).await
    }

    async fn send_draft(
        &self,
        draft: &DraftReplyState,
        metadata: &serde_json::Value,
    ) -> Result<Option<String>, ChannelError> {
        let channel_id = match metadata.get("channel_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => return Ok(None),
        };

        let bot_token = self.config.bot_token.expose_secret();
        let text = if draft.accumulated.len() > MAX_MESSAGE_LENGTH {
            // Truncate to fit Discord limits (safe for multi-byte UTF-8)
            let safe_end = floor_char_boundary(&draft.accumulated, MAX_MESSAGE_LENGTH - 20);
            let cutoff = draft.accumulated[..safe_end].rfind(' ').unwrap_or(safe_end);
            format!("{}...", &draft.accumulated[..cutoff])
        } else {
            draft.display_text()
        };

        if let Some(ref msg_id) = draft.message_id {
            // Edit existing message
            Self::edit_message(&self.client, bot_token, channel_id, msg_id, &text).await?;
            Ok(Some(msg_id.clone()))
        } else {
            // Post new message
            let msg_id =
                Self::send_message_with_id(&self.client, bot_token, channel_id, &text).await?;
            Ok(Some(msg_id))
        }
    }

    fn stream_mode(&self) -> StreamMode {
        self.config.stream_mode
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
        self.shutdown_notify.notify_waiters();
        if let Some(handle) = self.gateway_task.lock().await.take() {
            drain_channel_task(handle, NAME).await;
        }
        Ok(())
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

async fn sleep_or_channel_shutdown(
    shutdown: &Arc<AtomicBool>,
    shutdown_notify: &Arc<Notify>,
    duration: Duration,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => shutdown.load(Ordering::Relaxed),
        _ = shutdown_notify.notified() => shutdown.load(Ordering::Relaxed),
    }
}

/// Wait for the current reconnect backoff (shutdown-aware), then grow it toward
/// [`MAX_RECONNECT_BACKOFF`]. Returns true if shutdown was signalled while waiting.
async fn sleep_backoff(
    shutdown: &Arc<AtomicBool>,
    shutdown_notify: &Arc<Notify>,
    backoff: &mut Duration,
) -> bool {
    let wait = *backoff;
    *backoff = (*backoff * 2).min(MAX_RECONNECT_BACKOFF);
    sleep_or_channel_shutdown(shutdown, shutdown_notify, wait).await
}

async fn drain_channel_task(mut handle: JoinHandle<()>, name: &'static str) {
    tokio::select! {
        result = &mut handle => {
            if let Err(error) = result {
                tracing::warn!(channel = name, error = %error, "channel gateway task exited with error");
            }
        }
        _ = tokio::time::sleep(CHANNEL_TASK_SHUTDOWN_TIMEOUT) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(channel = name, "channel gateway task did not drain before timeout; aborted");
        }
    }
}

/// Download Discord CDN attachments, returning `MediaContent` for each.
async fn download_discord_attachments(attachments: &[DiscordAttachment]) -> Vec<MediaContent> {
    let mut result = Vec::new();
    let mut total_bytes = 0usize;

    for att in attachments {
        // Skip oversized files
        if att.size > MAX_DISCORD_ATTACHMENT_SIZE {
            tracing::warn!(
                filename = %att.filename,
                size = att.size,
                max = MAX_DISCORD_ATTACHMENT_SIZE,
                "Discord: skipping oversized attachment"
            );
            continue;
        }

        let guarded = match validate_outbound_url_pinned_async(
            &att.url,
            &OutboundUrlGuardOptions {
                require_https: true,
                upgrade_http_to_https: false,
                allowlist: vec![
                    "cdn.discordapp.com".to_string(),
                    "media.discordapp.net".to_string(),
                ],
            },
        )
        .await
        {
            Ok(guarded) if guarded.url.port().is_none() || guarded.url.port() == Some(443) => {
                guarded
            }
            _ => {
                tracing::warn!(filename = %att.filename, "Discord: rejected untrusted attachment URL");
                continue;
            }
        };
        let mut builder = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy();
        if !guarded.pinned_addrs.is_empty()
            && let Some(host) = guarded.url.host_str()
        {
            builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
        }
        let Ok(download_client) = builder.build() else {
            tracing::warn!(filename = %att.filename, "Discord: failed to build attachment client");
            continue;
        };

        match download_client.get(guarded.url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match crate::response::bounded_bytes(resp, MAX_DISCORD_ATTACHMENT_SIZE as usize)
                    .await
                {
                    Ok(bytes)
                        if total_bytes.saturating_add(bytes.len())
                            <= MAX_DISCORD_TOTAL_ATTACHMENT_SIZE =>
                    {
                        total_bytes += bytes.len();
                        let mime = att
                            .content_type
                            .as_deref()
                            .unwrap_or("application/octet-stream");
                        let mc = MediaContent::new(bytes.to_vec(), mime)
                            .with_filename(att.filename.clone());
                        tracing::debug!(
                            filename = %att.filename,
                            mime = %mime,
                            size = bytes.len(),
                            "Discord: downloaded attachment"
                        );
                        result.push(mc);
                    }
                    Ok(_) => {
                        tracing::warn!(filename = %att.filename, "Discord: total attachment size limit reached");
                        break;
                    }
                    Err(e) => {
                        tracing::warn!(
                            filename = %att.filename,
                            error = %e,
                            "Discord: failed to read attachment bytes"
                        );
                    }
                }
            }
            Ok(resp) => {
                tracing::warn!(
                    filename = %att.filename,
                    status = %resp.status(),
                    "Discord: attachment download returned non-200"
                );
            }
            Err(e) => {
                tracing::warn!(
                    filename = %att.filename,
                    error = %e,
                    "Discord: attachment download failed"
                );
            }
        }
    }

    result
}

/// Clamp a Gateway-supplied `heartbeat_interval` into a sane range. A hostile
/// or buggy Hello frame carrying `0` would otherwise panic `tokio::time::interval`.
fn sanitize_heartbeat_interval(raw: u64) -> u64 {
    if (1_000..=600_000).contains(&raw) {
        raw
    } else {
        tracing::warn!(
            raw,
            "Discord: out-of-range heartbeat_interval, using default"
        );
        DEFAULT_HEARTBEAT_INTERVAL_MS
    }
}

/// Discord Gateway close codes that will never succeed on retry — reconnecting
/// on these just spins. See the Discord Gateway close-code table.
fn is_fatal_close_code(code: u16) -> bool {
    matches!(
        code,
        // authentication failed, invalid shard, sharding required,
        // invalid API version, invalid intents, disallowed intents
        4004 | 4010 | 4011 | 4012 | 4013 | 4014
    )
}

/// Execute a Discord REST request, honoring 429 rate limits with bounded
/// retries. `build` is invoked fresh per attempt so non-cloneable bodies work.
async fn send_rest<F>(build: F) -> Result<reqwest::Response, ChannelError>
where
    F: Fn() -> reqwest::RequestBuilder,
{
    let mut attempt = 0u32;
    loop {
        let resp = build().send().await.map_err(|e| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("request: {e}"),
        })?;

        if resp.status().as_u16() == 429 && attempt < MAX_REST_RETRIES {
            attempt += 1;
            let retry_after = retry_after_secs(resp).await;
            tracing::warn!(
                attempt,
                retry_after_secs = retry_after,
                "Discord: rate limited (429), backing off"
            );
            tokio::time::sleep(Duration::from_secs_f64(retry_after)).await;
            continue;
        }
        return Ok(resp);
    }
}

/// Extract the retry delay (seconds) from a Discord 429 response, preferring the
/// `Retry-After` header and falling back to the JSON `retry_after` body.
async fn retry_after_secs(resp: reqwest::Response) -> f64 {
    if let Some(secs) = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<f64>().ok())
    {
        return sanitize_retry_after(secs);
    }
    crate::response::bounded_json::<serde_json::Value>(resp, MAX_DISCORD_ERROR_RESPONSE_BYTES)
        .await
        .ok()
        .and_then(|b| b.get("retry_after").and_then(|v| v.as_f64()))
        .map(sanitize_retry_after)
        .unwrap_or(1.0)
}

fn sanitize_retry_after(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value.min(60.0)
    } else {
        1.0
    }
}

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

        // Safe for multi-byte UTF-8: round down to a valid char boundary
        let safe_end = floor_char_boundary(remaining, MAX_MESSAGE_LENGTH);
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
    fn fatal_close_codes_are_not_retried() {
        // Auth failure and intent problems are terminal.
        for code in [4004, 4010, 4011, 4012, 4013, 4014] {
            assert!(is_fatal_close_code(code), "{code} should be fatal");
        }
        // Transient/normal closes should reconnect.
        for code in [1000, 1001, 1006, 4000, 4001, 4002, 4008, 4009] {
            assert!(!is_fatal_close_code(code), "{code} should reconnect");
        }
    }

    #[test]
    fn heartbeat_interval_is_sanitized() {
        // Zero would panic tokio::time::interval; out-of-range falls back.
        assert_eq!(
            sanitize_heartbeat_interval(0),
            DEFAULT_HEARTBEAT_INTERVAL_MS
        );
        assert_eq!(
            sanitize_heartbeat_interval(u64::MAX),
            DEFAULT_HEARTBEAT_INTERVAL_MS
        );
        // A normal value passes through unchanged.
        assert_eq!(sanitize_heartbeat_interval(41_250), 41_250);
    }

    #[test]
    fn retry_after_is_finite_and_bounded() {
        assert_eq!(sanitize_retry_after(f64::NAN), 1.0);
        assert_eq!(sanitize_retry_after(f64::INFINITY), 1.0);
        assert_eq!(sanitize_retry_after(-1.0), 1.0);
        assert_eq!(sanitize_retry_after(0.25), 0.25);
        assert_eq!(sanitize_retry_after(600.0), 60.0);
    }

    #[test]
    fn gateway_url_is_restricted_to_discord() {
        assert_eq!(
            normalize_discord_gateway_url("wss://gateway.discord.gg/?v=9&encoding=etf")
                .expect("official Gateway URL"),
            "wss://gateway.discord.gg/?v=10&encoding=json"
        );
        for url in [
            "ws://gateway.discord.gg",
            "wss://gateway.discord.gg:444",
            "wss://gateway.discord.gg@example.com",
            "wss://example.com",
        ] {
            assert!(normalize_discord_gateway_url(url).is_err(), "{url}");
        }
    }

    #[test]
    fn config_debug_redacts_bot_token() {
        let config = DiscordConfig {
            bot_token: secrecy::SecretString::new("super-secret-token".to_string().into()),
            guild_id: None,
            allow_from: vec![],
            stream_mode: StreamMode::None,
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("super-secret-token"));
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

    #[test]
    fn formatting_hints_describe_discord_markdown() {
        let config = DiscordConfig {
            bot_token: secrecy::SecretString::new("token".to_string().into()),
            guild_id: None,
            allow_from: vec![],
            stream_mode: StreamMode::None,
        };
        let channel = DiscordChannel::new(config).expect("discord channel");
        assert_eq!(
            channel.formatting_hints().as_deref(),
            Some(
                "Discord supports markdown. Use ``` for code blocks. Embeds are available for media. Use <@user_id> for mentions."
            )
        );
    }
}
