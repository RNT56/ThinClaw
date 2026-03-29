//! Slack channel via Socket Mode (Events API over WebSocket).
//!
//! Uses raw WebSocket connection to the Slack Socket Mode endpoint,
//! avoiding the need for a public HTTP endpoint. The bot receives
//! events (messages, app mentions) over a WebSocket and sends
//! responses via the Slack Web API (`chat.postMessage`).
//!
//! ## Required Configuration
//!
//! - `SLACK_BOT_TOKEN` (xoxb-...) — Bot User OAuth Token
//! - `SLACK_APP_TOKEN` (xapp-...) — App-Level Token with `connections:write` scope
//!
//! ## Socket Mode Flow
//!
//! 1. Call `apps.connections.open` with the app token to get a WSS URL
//! 2. Connect to the WSS URL
//! 3. Receive events as JSON envelopes
//! 4. ACK each envelope with `{"envelope_id": "..."}` within 3 seconds
//! 5. Process the event and respond via the Web API

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
const NAME: &str = "slack";

/// Maximum message length for Slack.
const MAX_MESSAGE_LENGTH: usize = 4000;

// ── Configuration ───────────────────────────────────────────────────

/// Slack channel configuration.
#[derive(Debug, Clone)]
pub struct SlackConfig {
    /// Bot User OAuth Token (xoxb-...).
    pub bot_token: SecretString,
    /// App-Level Token (xapp-...) for Socket Mode.
    pub app_token: SecretString,
    /// Allowed channel/DM IDs (empty = allow all).
    pub allow_from: Vec<String>,
}

// ── Socket Mode API types ───────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ConnectionsOpenResponse {
    ok: bool,
    url: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SocketEnvelope {
    envelope_id: String,
    #[serde(rename = "type")]
    envelope_type: String,
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SlackEvent {
    #[serde(rename = "type")]
    event_type: String,
    text: Option<String>,
    user: Option<String>,
    channel: Option<String>,
    ts: Option<String>,
    thread_ts: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EventPayload {
    event: Option<SlackEvent>,
}

// ── Channel implementation ──────────────────────────────────────────

/// Slack channel using Socket Mode.
pub struct SlackChannel {
    config: SlackConfig,
    client: Client,
    shutdown: Arc<AtomicBool>,
}

impl SlackChannel {
    /// Create a new Slack channel.
    pub fn new(config: SlackConfig) -> Result<Self, ChannelError> {
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

    /// Get a Socket Mode WebSocket URL.
    async fn get_ws_url(client: &Client, app_token: &str) -> Result<String, ChannelError> {
        let resp = client
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(app_token)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("apps.connections.open: {e}"),
            })?;

        let body: ConnectionsOpenResponse =
            resp.json().await.map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("Parse connections.open: {e}"),
            })?;

        if !body.ok {
            return Err(ChannelError::AuthFailed {
                name: NAME.to_string(),
                reason: format!(
                    "apps.connections.open failed: {}",
                    body.error.unwrap_or_default()
                ),
            });
        }

        body.url.ok_or_else(|| ChannelError::StartupFailed {
            name: NAME.to_string(),
            reason: "No WebSocket URL in response".to_string(),
        })
    }

    /// Send a message via the Slack Web API.
    async fn post_message(
        client: &Client,
        bot_token: &str,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> Result<(), ChannelError> {
        let chunks = split_message(text);

        for chunk in chunks {
            let mut body = serde_json::json!({
                "channel": channel,
                "text": chunk,
            });

            if let Some(ts) = thread_ts {
                body["thread_ts"] = serde_json::Value::String(ts.to_string());
            }

            let resp = client
                .post("https://slack.com/api/chat.postMessage")
                .bearer_auth(bot_token)
                .json(&body)
                .send()
                .await
                .map_err(|e| ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("chat.postMessage: {e}"),
                })?;

            let result: serde_json::Value = resp.json().await.unwrap_or_default();
            if result.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                let error = result
                    .get("error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                return Err(ChannelError::SendFailed {
                    name: NAME.to_string(),
                    reason: format!("chat.postMessage: {error}"),
                });
            }
        }

        Ok(())
    }
}

#[async_trait]
impl Channel for SlackChannel {
    fn name(&self) -> &str {
        NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(64);

        let app_token = self.config.app_token.expose_secret().to_string();
        let bot_token = self.config.bot_token.expose_secret().to_string();
        let allow_from = self.config.allow_from.clone();
        let client = self.client.clone();
        let shutdown = self.shutdown.clone();

        // Validate bot token
        let auth_resp = client
            .post("https://slack.com/api/auth.test")
            .bearer_auth(&bot_token)
            .send()
            .await
            .map_err(|e| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!("auth.test: {e}"),
            })?;

        let auth: serde_json::Value = auth_resp.json().await.unwrap_or_default();
        if auth.get("ok").and_then(|v| v.as_bool()) != Some(true) {
            return Err(ChannelError::AuthFailed {
                name: NAME.to_string(),
                reason: "Invalid bot token".to_string(),
            });
        }

        let bot_user_id = auth
            .get("user_id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        tracing::info!(
            "Slack bot connected as {} ({})",
            auth.get("user").and_then(|v| v.as_str()).unwrap_or("?"),
            bot_user_id
        );

        // Spawn Socket Mode connection
        tokio::spawn(async move {
            loop {
                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                // Get a fresh WebSocket URL
                let ws_url = match Self::get_ws_url(&client, &app_token).await {
                    Ok(url) => url,
                    Err(e) => {
                        tracing::error!("Slack: failed to get WS URL: {e}");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                // Connect
                let ws_stream = match connect_async(&ws_url).await {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        tracing::error!("Slack: WebSocket connect failed: {e}");
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        continue;
                    }
                };

                tracing::info!("Slack Socket Mode connected");
                let (mut ws_write, mut ws_read) = ws_stream.split();

                // Process messages
                loop {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }

                    let msg = tokio::select! {
                        msg = ws_read.next() => msg,
                        _ = tokio::time::sleep(Duration::from_secs(60)) => {
                            // Send ping to keep connection alive
                            if ws_write.send(WsMessage::Ping(vec![].into())).await.is_err() {
                                break;
                            }
                            continue;
                        }
                    };

                    let msg = match msg {
                        Some(Ok(WsMessage::Text(text))) => text,
                        Some(Ok(WsMessage::Close(_))) | None => {
                            tracing::warn!("Slack: WebSocket closed, reconnecting...");
                            break;
                        }
                        Some(Ok(WsMessage::Ping(data))) => {
                            let _ = ws_write.send(WsMessage::Pong(data)).await;
                            continue;
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => {
                            tracing::error!("Slack WS error: {e}");
                            break;
                        }
                    };

                    // Parse envelope
                    let envelope: SocketEnvelope = match serde_json::from_str(&msg) {
                        Ok(e) => e,
                        Err(_) => continue,
                    };

                    // ACK immediately (Slack requires < 3s)
                    let ack = serde_json::json!({"envelope_id": envelope.envelope_id});
                    let _ = ws_write.send(WsMessage::Text(ack.to_string().into())).await;

                    // Only process events_api envelopes
                    if envelope.envelope_type != "events_api" {
                        continue;
                    }

                    let payload = match envelope.payload {
                        Some(p) => p,
                        None => continue,
                    };

                    let event_payload: EventPayload = match serde_json::from_value(payload) {
                        Ok(ep) => ep,
                        Err(_) => continue,
                    };

                    let event = match event_payload.event {
                        Some(e) => e,
                        None => continue,
                    };

                    // Only process messages and app_mentions
                    match event.event_type.as_str() {
                        "message" | "app_mention" => {}
                        _ => continue,
                    }

                    let text = match &event.text {
                        Some(t) if !t.is_empty() => t.clone(),
                        _ => continue,
                    };

                    let user_id = match &event.user {
                        Some(u) => u.clone(),
                        None => continue,
                    };

                    // Skip bot's own messages
                    if user_id == bot_user_id {
                        continue;
                    }

                    let channel_id = match &event.channel {
                        Some(c) => c.clone(),
                        None => continue,
                    };

                    // Check allow-list
                    if !allow_from.is_empty()
                        && !allow_from.iter().any(|a| a == "*" || a == &channel_id)
                    {
                        continue;
                    }

                    // Strip bot mention from text if present
                    let clean_text = text
                        .replace(&format!("<@{bot_user_id}>"), "")
                        .trim()
                        .to_string();

                    if clean_text.is_empty() {
                        continue;
                    }

                    let incoming = IncomingMessage::new(NAME, &user_id, &clean_text).with_metadata(
                        serde_json::json!({
                            "channel": channel_id,
                            "ts": event.ts,
                            "thread_ts": event.thread_ts,
                        }),
                    );

                    if tx.send(incoming).await.is_err() {
                        tracing::warn!("Slack channel receiver dropped");
                        return;
                    }
                }

                if !shutdown.load(Ordering::Relaxed) {
                    tracing::info!("Slack: reconnecting in 5s...");
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
        let channel = msg
            .metadata
            .get("channel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: "Missing channel in metadata".to_string(),
            })?;

        // Reply in thread if the original was in a thread
        let thread_ts = msg
            .metadata
            .get("thread_ts")
            .and_then(|v| v.as_str())
            .or_else(|| msg.metadata.get("ts").and_then(|v| v.as_str()));

        // Convert standard Markdown → Slack mrkdwn
        let mrkdwn_content = markdown_to_slack_mrkdwn(&response.content);

        let bot_token = self.config.bot_token.expose_secret();
        Self::post_message(
            &self.client,
            bot_token,
            channel,
            &mrkdwn_content,
            thread_ts,
        )
        .await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Forward agent progress messages as real Slack messages
        if let StatusUpdate::AgentMessage {
            content,
            message_type,
        } = status
        {
            let channel = match metadata.get("channel").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => return Ok(()),
            };
            let thread_ts = metadata.get("thread_ts").and_then(|v| v.as_str());

            let prefix = match message_type.as_str() {
                "warning" => "⚠️ ",
                "question" => "❓ ",
                "interim_result" => "📋 ",
                _ => "💬 ",
            };
            let text = format!("{}{}", prefix, markdown_to_slack_mrkdwn(&content));
            let bot_token = self.config.bot_token.expose_secret();
            let _ = Self::post_message(&self.client, bot_token, channel, &text, thread_ts).await;
        }
        // All other status updates are silently dropped (Slack has no typing indicator for bots)
        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Slack channel/DM IDs start with C, D, or G (e.g. "C1234", "D5678").
        // Skip gracefully if the ID doesn't look valid (e.g. "default").
        let first = user_id.chars().next().unwrap_or('_');
        if !matches!(first, 'C' | 'D' | 'G' | 'U' | 'W') {
            tracing::debug!(
                recipient = user_id,
                "Slack: skipping broadcast — recipient is not a valid Slack ID"
            );
            return Ok(());
        }
        let bot_token = self.config.bot_token.expose_secret();
        let mrkdwn_content = markdown_to_slack_mrkdwn(&response.content);
        Self::post_message(&self.client, bot_token, user_id, &mrkdwn_content, None).await
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let resp = self
            .client
            .post("https://slack.com/api/auth.test")
            .bearer_auth(self.config.bot_token.expose_secret())
            .send()
            .await
            .map_err(|e| ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("Health check: {e}"),
            })?;

        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
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

/// Split a long message into chunks for Slack's limit.
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
        let safe_end = crate::util::floor_char_boundary(remaining, MAX_MESSAGE_LENGTH);
        let split_at = remaining[..safe_end].rfind('\n').unwrap_or(safe_end);

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }

    chunks
}

// ── Markdown → Slack mrkdwn Converter ────────────────────────────────

/// Convert standard Markdown (as produced by LLMs) to Slack's mrkdwn format.
///
/// Key differences from standard Markdown:
/// - Bold: `**text**` → `*text*`
/// - Strikethrough: `~~text~~` → `~text~`
/// - Links: `[text](url)` → `<url|text>`
/// - Headings: `# Heading` → `*Heading*` (bold, since Slack has no headings)
/// - Italic `_text_`, code `` `code` ``, code blocks ` ```...``` `,
///   and blockquotes `>` pass through unchanged.
fn markdown_to_slack_mrkdwn(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_code_block = false;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            result.push_str(line);
            result.push('\n');
            continue;
        }
        if in_code_block {
            result.push_str(line);
            result.push('\n');
            continue;
        }

        // Convert heading lines
        if let Some(heading_text) = slack_parse_heading(trimmed) {
            let leading_ws = &line[..line.len() - trimmed.len()];
            result.push_str(leading_ws);
            result.push('*');
            result.push_str(heading_text);
            result.push('*');
            result.push('\n');
            continue;
        }

        let converted = slack_convert_inline(line);
        result.push_str(&converted);
        result.push('\n');
    }

    if result.ends_with('\n') && !input.ends_with('\n') {
        result.pop();
    }
    result
}

fn slack_parse_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim())
}

fn slack_convert_inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;

    while i < len {
        if chars[i] == '`' {
            out.push('`');
            i += 1;
            while i < len && chars[i] != '`' {
                out.push(chars[i]);
                i += 1;
            }
            if i < len {
                out.push('`');
                i += 1;
            }
            continue;
        }

        if chars[i] == '[' {
            if let Some((text, url, end)) = slack_parse_link(&chars, i) {
                out.push('<');
                out.push_str(&url);
                out.push('|');
                out.push_str(&text);
                out.push('>');
                i = end;
                continue;
            }
        }

        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some((content, end)) = slack_extract_delimited(&chars, i, '~', 2) {
                out.push('~');
                out.push_str(&content);
                out.push('~');
                i = end;
                continue;
            }
        }

        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some((content, end)) = slack_extract_delimited(&chars, i, '*', 2) {
                out.push('*');
                out.push_str(&content);
                out.push('*');
                i = end;
                continue;
            }
        }

        if i + 1 < len && chars[i] == '_' && chars[i + 1] == '_' {
            if let Some((content, end)) = slack_extract_delimited(&chars, i, '_', 2) {
                out.push('*');
                out.push_str(&content);
                out.push('*');
                i = end;
                continue;
            }
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

fn slack_parse_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if chars[start] != '[' {
        return None;
    }
    let mut i = start + 1;
    let mut text = String::new();
    let mut depth = 1;
    while i < chars.len() && depth > 0 {
        match chars[i] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        text.push(chars[i]);
        i += 1;
    }
    if depth != 0 || i >= chars.len() {
        return None;
    }
    i += 1;
    if i >= chars.len() || chars[i] != '(' {
        return None;
    }
    i += 1;
    let mut url = String::new();
    while i < chars.len() && chars[i] != ')' {
        url.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    i += 1;
    Some((text, url, i))
}

fn slack_extract_delimited(
    chars: &[char],
    start: usize,
    delimiter: char,
    count: usize,
) -> Option<(String, usize)> {
    let len = chars.len();
    for j in 0..count {
        if start + j >= len || chars[start + j] != delimiter {
            return None;
        }
    }
    let content_start = start + count;
    if content_start >= len {
        return None;
    }
    let mut i = content_start;
    while i + count - 1 < len {
        let mut found = true;
        for j in 0..count {
            if chars[i + j] != delimiter {
                found = false;
                break;
            }
        }
        if found {
            let content: String = chars[content_start..i].iter().collect();
            if !content.is_empty() {
                return Some((content, i + count));
            }
        }
        i += 1;
    }
    None
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
        let text = "a".repeat(5000);
        let chunks = split_message(&text);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= MAX_MESSAGE_LENGTH);
        }
    }

    #[test]
    fn test_split_at_newline() {
        let mut text = "x".repeat(3900);
        text.push('\n');
        text.push_str(&"y".repeat(200));
        let chunks = split_message(&text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 3900);
    }

    #[test]
    fn test_mrkdwn_bold() {
        assert_eq!(
            markdown_to_slack_mrkdwn("**hello world**"),
            "*hello world*"
        );
    }

    #[test]
    fn test_mrkdwn_double_underscore_bold() {
        assert_eq!(
            markdown_to_slack_mrkdwn("__bold text__"),
            "*bold text*"
        );
    }

    #[test]
    fn test_mrkdwn_strikethrough() {
        assert_eq!(
            markdown_to_slack_mrkdwn("~~deleted~~"),
            "~deleted~"
        );
    }

    #[test]
    fn test_mrkdwn_link() {
        assert_eq!(
            markdown_to_slack_mrkdwn("[Google](https://google.com)"),
            "<https://google.com|Google>"
        );
    }

    #[test]
    fn test_mrkdwn_heading() {
        assert_eq!(markdown_to_slack_mrkdwn("# Hello"), "*Hello*");
        assert_eq!(markdown_to_slack_mrkdwn("## Sub"), "*Sub*");
        assert_eq!(markdown_to_slack_mrkdwn("### Deep"), "*Deep*");
    }

    #[test]
    fn test_mrkdwn_code_block_preserved() {
        let input = "```rust\nlet x = **bold**;\n```";
        let output = markdown_to_slack_mrkdwn(input);
        assert_eq!(output, input);
    }

    #[test]
    fn test_mrkdwn_inline_code_preserved() {
        assert_eq!(
            markdown_to_slack_mrkdwn("use `**not bold**` here"),
            "use `**not bold**` here"
        );
    }

    #[test]
    fn test_mrkdwn_mixed() {
        let input = "**Bold** and _italic_ with [link](http://ex.com)";
        let expected = "*Bold* and _italic_ with <http://ex.com|link>";
        assert_eq!(markdown_to_slack_mrkdwn(input), expected);
    }

    #[test]
    fn test_mrkdwn_plain_text_passthrough() {
        let input = "Just plain text.";
        assert_eq!(markdown_to_slack_mrkdwn(input), input);
    }

    #[test]
    fn test_mrkdwn_blockquote_passthrough() {
        let input = "> quoted text";
        assert_eq!(markdown_to_slack_mrkdwn(input), input);
    }
}
