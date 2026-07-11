//! Gmail channel — receives emails via Google Pub/Sub, replies via Gmail API.
//!
//! ## Architecture
//!
//! ```text
//! Google Pub/Sub (push)
//!        │
//!        ▼
//!   GmailChannel::poll_loop()
//!     ├─ pulls messages from Pub/Sub subscription
//!     ├─ fetches full email via Gmail API
//!     ├─ filters by label + allowed senders
//!     └─ emits IncomingMessage to agent loop
//!
//! Agent response
//!        │
//!        ▼
//!   GmailChannel::respond()
//!     └─ sends reply via Gmail API messages.send()
//! ```
//!
//! ## Required Environment Variables
//!
//! - `GMAIL_ENABLED=true`
//! - `GMAIL_PROJECT_ID` — GCP project ID
//! - `GMAIL_SUBSCRIPTION_ID` — Pub/Sub subscription name
//! - `GMAIL_TOPIC_ID` — Pub/Sub topic name
//! - `GMAIL_OAUTH_TOKEN` — OAuth2 access token (ThinClaw Desktop Gmail setup)
//! - `GMAIL_REFRESH_TOKEN` / `GMAIL_CLIENT_ID` / `GMAIL_CLIENT_SECRET` — enable
//!   unattended access-token refresh for long-running deployments
//! - `GMAIL_ALLOWED_SENDERS` — comma-separated email allowlist (empty = all)

use std::sync::Arc;
use std::time::Duration;
use std::{fs, path::PathBuf};

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Notify, RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;

use crate::gmail_wiring::{GmailConfig, is_sender_allowed};
use thinclaw_channels_core::{
    Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate,
};
use thinclaw_types::error::ChannelError;

mod message;

use message::{build_multipart_message, sanitize_header_value};

/// Gmail channel implementing the `Channel` trait.
pub struct GmailChannel {
    config: GmailConfig,
    state: Arc<GmailChannelState>,
    http: Client,
}

struct GmailChannelState {
    /// Message sender — populated on start(), consumed by the agent loop.
    msg_tx: RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    /// OAuth2 access token for Google APIs.
    access_token: RwLock<String>,
    /// Whether the channel is running.
    running: RwLock<bool>,
    /// Errors encountered during polling.
    last_error: RwLock<Option<String>>,
    /// Count of messages processed (for health check telemetry).
    messages_processed: std::sync::atomic::AtomicU64,
    /// Last processed Gmail history ID persisted across restarts.
    last_history_id: RwLock<Option<u64>>,
    /// Notifies long-running Gmail loops to re-check their running state.
    shutdown_notify: Notify,
    /// Owned polling task, drained on restart and shutdown.
    poll_task: Mutex<Option<JoinHandle<()>>>,
    /// Owned token-refresh task, drained on restart and shutdown.
    refresh_task: Mutex<Option<JoinHandle<()>>>,
}

/// Response from Gmail API `messages.list`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailMessageListResponse {
    messages: Option<Vec<GmailMessageRef>>,
    #[serde(default)]
    #[allow(dead_code)]
    next_page_token: Option<String>,
}

/// Reference to a Gmail message (id only).
#[derive(Debug, Deserialize, Clone)]
struct GmailMessageRef {
    id: String,
    #[serde(default)]
    #[allow(dead_code)]
    thread_id: Option<String>,
}

/// Full Gmail message from the API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailMessage {
    id: String,
    thread_id: Option<String>,
    snippet: Option<String>,
    payload: Option<GmailPayload>,
    #[allow(dead_code)]
    label_ids: Option<Vec<String>>,
    #[serde(default)]
    #[allow(dead_code)]
    internal_date: Option<String>,
}

/// Gmail message payload (headers + body parts).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailPayload {
    headers: Option<Vec<GmailHeader>>,
    body: Option<GmailBody>,
    parts: Option<Vec<GmailPart>>,
    mime_type: Option<String>,
}

/// A single header key-value.
#[derive(Debug, Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}

/// Gmail body data.
#[derive(Debug, Deserialize)]
struct GmailBody {
    data: Option<String>,
    #[allow(dead_code)]
    size: Option<u64>,
}

/// Gmail MIME part.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailPart {
    mime_type: Option<String>,
    body: Option<GmailBody>,
    parts: Option<Vec<GmailPart>>,
}

/// Pub/Sub pull response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PubSubPullResponse {
    received_messages: Option<Vec<PubSubReceivedMessage>>,
}

/// A single Pub/Sub received message.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PubSubReceivedMessage {
    ack_id: String,
    message: PubSubMessage,
}

/// Pub/Sub message content.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PubSubMessage {
    data: Option<String>,
    #[allow(dead_code)]
    message_id: Option<String>,
    #[allow(dead_code)]
    attributes: Option<std::collections::HashMap<String, String>>,
}

/// Decoded Pub/Sub notification data for Gmail.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailNotification {
    email_address: Option<String>,
    history_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailHistoryResponse {
    history: Option<Vec<GmailHistoryEntry>>,
    history_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailHistoryEntry {
    messages_added: Option<Vec<GmailHistoryMessageAdded>>,
}

#[derive(Debug, Deserialize)]
struct GmailHistoryMessageAdded {
    message: GmailMessageRef,
}

struct FetchMessagesResult {
    messages: Vec<GmailMessage>,
    latest_history_id: Option<u64>,
}

/// Pub/Sub pull request body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PubSubPullRequest {
    max_messages: u32,
}

/// Pub/Sub acknowledge request body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PubSubAckRequest {
    ack_ids: Vec<String>,
}

/// Gmail API send request body (RFC 2822 base64url-encoded).
#[derive(Debug, Serialize)]
struct GmailSendRequest {
    raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
}

/// Maximum messages to pull per Pub/Sub request.
const PUBSUB_MAX_MESSAGES: u32 = 10;

/// Poll interval between Pub/Sub pull requests.
const POLL_INTERVAL: Duration = Duration::from_secs(10);

const CHANNEL_TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const GMAIL_UNREAD_FALLBACK_DAYS: u32 = 7;

/// Refresh the access token this many seconds before it actually expires, so a
/// request is never made with a token that lapses mid-flight.
const TOKEN_REFRESH_MARGIN_SECS: u64 = 300;
/// Fallback delay before retrying a failed proactive token refresh.
const TOKEN_REFRESH_RETRY_SECS: u64 = 60;

/// Response from Google's OAuth2 token endpoint on a refresh-token grant.
#[derive(Debug, Deserialize)]
struct GmailTokenRefreshResponse {
    access_token: String,
    #[serde(default = "default_token_lifetime")]
    expires_in: u64,
}

fn default_token_lifetime() -> u64 {
    3600
}

impl GmailChannel {
    /// Create a new Gmail channel from configuration.
    pub fn new(config: GmailConfig) -> Result<Self, ChannelError> {
        if !config.is_configured() {
            return Err(ChannelError::StartupFailed {
                name: "gmail".into(),
                reason: format!(
                    "Gmail channel not fully configured. Missing: {}",
                    config.validate().join(", ")
                ),
            });
        }

        let access_token = config.oauth_token.clone().unwrap_or_default();
        let persisted_history_id = Self::load_persisted_history_id(&config);

        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("ThinClaw/1.0")
            .build()
            .map_err(|e| ChannelError::StartupFailed {
                name: "gmail".into(),
                reason: format!("Failed to create HTTP client: {}", e),
            })?;

        Ok(Self {
            config,
            state: Arc::new(GmailChannelState {
                msg_tx: RwLock::new(None),
                access_token: RwLock::new(access_token),
                running: RwLock::new(false),
                last_error: RwLock::new(None),
                messages_processed: std::sync::atomic::AtomicU64::new(0),
                last_history_id: RwLock::new(persisted_history_id),
                shutdown_notify: Notify::new(),
                poll_task: Mutex::new(None),
                refresh_task: Mutex::new(None),
            }),
            http,
        })
    }

    /// Update the OAuth access token (e.g., after a token refresh).
    pub async fn set_access_token(&self, token: &str) {
        *self.state.access_token.write().await = token.to_string();
    }

    /// Exchange the configured refresh token for a fresh access token and
    /// install it into shared state.
    ///
    /// Google access tokens expire after ~1 hour; without this a long-running
    /// gateway/service deployment silently stops sending/receiving once the
    /// initial token lapses. Returns the new token's lifetime in seconds so the
    /// proactive loop can schedule the next refresh before expiry.
    async fn refresh_access_token(
        config: &GmailConfig,
        state: &Arc<GmailChannelState>,
        http: &Client,
    ) -> Result<u64, ChannelError> {
        let (Some(refresh_token), Some(client_id), Some(client_secret)) = (
            config.refresh_token.as_deref(),
            config.client_id.as_deref(),
            config.client_secret.as_deref(),
        ) else {
            return Err(ChannelError::AuthFailed {
                name: "gmail".into(),
                reason: "Cannot refresh Gmail token: refresh_token/client_id/client_secret \
                         not configured (set GMAIL_REFRESH_TOKEN, GMAIL_CLIENT_ID, \
                         GMAIL_CLIENT_SECRET)"
                    .into(),
            });
        };

        let resp = http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("refresh_token", refresh_token),
                ("grant_type", "refresh_token"),
            ])
            .send()
            .await
            .map_err(|e| ChannelError::AuthFailed {
                name: "gmail".into(),
                reason: format!("Gmail token refresh request failed: {e}"),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::AuthFailed {
                name: "gmail".into(),
                reason: format!("Gmail token refresh returned {status}: {body}"),
            });
        }

        let token: GmailTokenRefreshResponse =
            resp.json().await.map_err(|e| ChannelError::AuthFailed {
                name: "gmail".into(),
                reason: format!("Invalid Gmail token refresh response: {e}"),
            })?;

        *state.access_token.write().await = token.access_token;
        *state.last_error.write().await = None;
        tracing::info!(
            expires_in = token.expires_in,
            "Refreshed Gmail OAuth access token"
        );
        Ok(token.expires_in)
    }

    /// Heuristic: does this error look like an expired/invalid access token?
    fn is_auth_error(error: &ChannelError) -> bool {
        if matches!(error, ChannelError::AuthFailed { .. }) {
            return true;
        }
        let text = error.to_string();
        text.contains("401")
            || text.contains("UNAUTHENTICATED")
            || text.contains("invalid_token")
            || text.contains("Invalid Credentials")
            || text.contains("invalid authentication credentials")
    }

    /// Background task: refresh the access token before it expires so the
    /// channel keeps working indefinitely. No-op (task exits) when refresh
    /// credentials are not configured.
    async fn token_refresh_loop(config: GmailConfig, state: Arc<GmailChannelState>, http: Client) {
        if !config.can_refresh_token() {
            return;
        }
        loop {
            if !*state.running.read().await {
                break;
            }
            // Refresh immediately on entry, then reschedule based on the token
            // lifetime with a safety margin; fall back to 45 min on failure.
            let next_delay_secs = match Self::refresh_access_token(&config, &state, &http).await {
                Ok(expires_in) => expires_in.saturating_sub(TOKEN_REFRESH_MARGIN_SECS).max(60),
                Err(e) => {
                    tracing::warn!(error = %e, "Gmail proactive token refresh failed; retrying soon");
                    *state.last_error.write().await = Some(e.to_string());
                    TOKEN_REFRESH_RETRY_SECS
                }
            };
            if !*state.running.read().await {
                break;
            }
            if sleep_or_gmail_shutdown(&state, Duration::from_secs(next_delay_secs)).await {
                break;
            }
        }
    }

    /// Returns the number of messages processed since startup.
    pub fn messages_processed(&self) -> u64 {
        self.state
            .messages_processed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Pull messages from Pub/Sub subscription.
    async fn pull_pubsub(&self) -> Result<Vec<PubSubReceivedMessage>, ChannelError> {
        let token = self.state.access_token.read().await.clone();
        if token.is_empty() {
            return Err(ChannelError::AuthFailed {
                name: "gmail".into(),
                reason: "No OAuth token configured (authenticate via ThinClaw Desktop Gmail setup, or set GMAIL_OAUTH_TOKEN)".into(),
            });
        }

        let url = format!(
            "https://pubsub.googleapis.com/v1/projects/{}/subscriptions/{}:pull",
            self.config.project_id, self.config.subscription_id
        );

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&PubSubPullRequest {
                max_messages: PUBSUB_MAX_MESSAGES,
            })
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Pub/Sub pull failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Pub/Sub pull returned {}: {}", status, body),
            });
        }

        let pull_response: PubSubPullResponse =
            resp.json().await.map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Failed to parse Pub/Sub response: {}", e),
            })?;

        Ok(pull_response.received_messages.unwrap_or_default())
    }

    /// Acknowledge Pub/Sub messages after processing.
    async fn ack_pubsub(&self, ack_ids: Vec<String>) -> Result<(), ChannelError> {
        if ack_ids.is_empty() {
            return Ok(());
        }

        let token = self.state.access_token.read().await.clone();
        let url = format!(
            "https://pubsub.googleapis.com/v1/projects/{}/subscriptions/{}:acknowledge",
            self.config.project_id, self.config.subscription_id
        );

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&PubSubAckRequest { ack_ids })
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Pub/Sub ack failed: {}", e),
            })?;

        if !resp.status().is_success() {
            tracing::warn!(
                status = %resp.status(),
                "Pub/Sub acknowledge returned non-success status"
            );
        }

        Ok(())
    }

    fn history_state_path(config: &GmailConfig) -> PathBuf {
        let sanitize = |value: &str| {
            value
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>()
        };
        thinclaw_platform::resolve_thinclaw_home()
            .join("channels")
            .join("gmail")
            .join(format!(
                "{}_{}.history_id",
                sanitize(&config.project_id),
                sanitize(&config.subscription_id)
            ))
    }

    fn load_persisted_history_id(config: &GmailConfig) -> Option<u64> {
        let path = Self::history_state_path(config);
        let content = fs::read_to_string(path).ok()?;
        content.trim().parse::<u64>().ok()
    }

    async fn persist_history_id(&self, history_id: u64) -> Result<(), ChannelError> {
        let path = Self::history_state_path(&self.config);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Failed to create Gmail state directory: {}", e),
            })?;
        }
        fs::write(&path, history_id.to_string()).map_err(|e| ChannelError::SendFailed {
            name: "gmail".into(),
            reason: format!("Failed to persist Gmail history ID: {}", e),
        })?;
        *self.state.last_history_id.write().await = Some(history_id);
        Ok(())
    }

    async fn fetch_new_messages(
        &self,
        notification_history_id: Option<u64>,
    ) -> Result<FetchMessagesResult, ChannelError> {
        let last_history_id = *self.state.last_history_id.read().await;
        if let Some(start_history_id) = last_history_id {
            match self
                .fetch_messages_from_history(start_history_id, notification_history_id)
                .await
            {
                Ok(result) => return Ok(result),
                Err(ChannelError::SendFailed { reason, .. })
                    if reason.contains("history window expired")
                        || reason.contains("invalid startHistoryId") =>
                {
                    tracing::info!("Gmail history cursor expired, falling back to unread rescan");
                }
                Err(err) => return Err(err),
            }
        }

        let messages = self.fetch_unread_messages_bounded().await?;
        Ok(FetchMessagesResult {
            messages,
            latest_history_id: notification_history_id.or(last_history_id),
        })
    }

    async fn fetch_messages_from_history(
        &self,
        start_history_id: u64,
        notification_history_id: Option<u64>,
    ) -> Result<FetchMessagesResult, ChannelError> {
        let token = self.state.access_token.read().await.clone();
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/history?startHistoryId={}&historyTypes=messageAdded&maxResults=100",
            start_history_id
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail history fetch failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::NOT_FOUND
                || body.contains("startHistoryId")
                || body.contains("historyId")
            {
                return Err(ChannelError::SendFailed {
                    name: "gmail".into(),
                    reason: format!(
                        "Gmail history window expired for startHistoryId {}",
                        start_history_id
                    ),
                });
            }
            return Err(ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail history returned {}: {}", status, body),
            });
        }

        let history: GmailHistoryResponse =
            resp.json().await.map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Failed to parse Gmail history response: {}", e),
            })?;
        let latest_history_id = history
            .history_id
            .as_deref()
            .and_then(|value| value.parse::<u64>().ok())
            .or(notification_history_id);

        let mut message_ids = std::collections::BTreeSet::new();
        for entry in history.history.unwrap_or_default() {
            for added in entry.messages_added.unwrap_or_default() {
                message_ids.insert(added.message.id);
            }
        }

        let mut messages = Vec::new();
        for id in message_ids {
            match self.fetch_message(&id, &token).await {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!(
                        message_id = %id,
                        error = %e,
                        "Failed to fetch Gmail history message, skipping"
                    );
                }
            }
        }

        Ok(FetchMessagesResult {
            messages,
            latest_history_id,
        })
    }

    async fn fetch_unread_messages_bounded(&self) -> Result<Vec<GmailMessage>, ChannelError> {
        let token = self.state.access_token.read().await.clone();

        // List unread messages matching label filters.
        let label_query = self
            .config
            .label_filters
            .iter()
            .map(|l| format!("label:{}", l.to_lowercase()))
            .collect::<Vec<_>>()
            .join(" ");

        let query = if label_query.is_empty() {
            format!("is:unread newer_than:{}d", GMAIL_UNREAD_FALLBACK_DAYS)
        } else {
            format!(
                "is:unread newer_than:{}d {}",
                GMAIL_UNREAD_FALLBACK_DAYS, label_query
            )
        };

        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages?q={}&maxResults=10",
            urlencoding::encode(&query)
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail list failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail list returned {}: {}", status, body),
            });
        }

        let list: GmailMessageListResponse =
            resp.json().await.map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Failed to parse Gmail list: {}", e),
            })?;

        let refs = list.messages.unwrap_or_default();
        let mut messages = Vec::new();

        for msg_ref in refs.iter().take(10) {
            match self.fetch_message(&msg_ref.id, &token).await {
                Ok(msg) => messages.push(msg),
                Err(e) => {
                    tracing::warn!(
                        message_id = %msg_ref.id,
                        error = %e,
                        "Failed to fetch Gmail message, skipping"
                    );
                }
            }
        }
        Ok(messages)
    }

    /// Fetch a single message by ID.
    async fn fetch_message(&self, id: &str, token: &str) -> Result<GmailMessage, ChannelError> {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=full",
            id
        );

        let resp = self
            .http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail get message failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail get returned {}: {}", status, body),
            });
        }

        resp.json().await.map_err(|e| ChannelError::SendFailed {
            name: "gmail".into(),
            reason: format!("Failed to parse Gmail message: {}", e),
        })
    }

    /// Mark a message as read by removing the UNREAD label.
    async fn mark_as_read(&self, message_id: &str) -> Result<(), ChannelError> {
        let token = self.state.access_token.read().await.clone();
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}/modify",
            message_id
        );

        let body = serde_json::json!({
            "removeLabelIds": ["UNREAD"]
        });

        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail mark-read failed: {}", e),
            })?;

        if !resp.status().is_success() {
            tracing::warn!(
                message_id = message_id,
                status = %resp.status(),
                "Failed to mark message as read"
            );
        }

        Ok(())
    }

    /// Send a reply email via Gmail API.
    async fn send_reply(
        &self,
        to: &str,
        subject: &str,
        body_text: &str,
        thread_id: Option<&str>,
        in_reply_to: Option<&str>,
        attachments: &[thinclaw_media::MediaContent],
    ) -> Result<(), ChannelError> {
        let token = self.state.access_token.read().await.clone();

        // Build RFC 2822 message. Every interpolated header value is stripped
        // of CR/LF first so an agent- or tool-supplied recipient/subject cannot
        // inject additional headers (e.g. a hidden `Bcc:`) into the message.
        let to = sanitize_header_value(to);
        let subject = sanitize_header_value(subject);
        let mut headers = format!("To: {}\r\nSubject: {}\r\n", to, subject);
        if let Some(reply_to) = in_reply_to {
            let reply_to = sanitize_header_value(reply_to);
            headers.push_str(&format!(
                "In-Reply-To: {}\r\nReferences: {}\r\n",
                reply_to, reply_to
            ));
        }
        let raw_message = if attachments.is_empty() {
            format!(
                "{}Content-Type: text/plain; charset=utf-8\r\n\r\n{}",
                headers, body_text
            )
        } else {
            build_multipart_message(headers, body_text, attachments)
        };

        // Base64url encode the raw message.
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let encoded = URL_SAFE_NO_PAD.encode(raw_message.as_bytes());

        let send_body = GmailSendRequest {
            raw: encoded,
            thread_id: thread_id.map(|s| s.to_string()),
        };

        let url = "https://gmail.googleapis.com/gmail/v1/users/me/messages/send";

        let resp = self
            .http
            .post(url)
            .bearer_auth(&token)
            .json(&send_body)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail send failed: {}", e),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ChannelError::SendFailed {
                name: "gmail".into(),
                reason: format!("Gmail send returned {}: {}", status, body),
            });
        }

        Ok(())
    }

    /// Extract sender email from message headers.
    fn extract_sender(msg: &GmailMessage) -> Option<String> {
        msg.payload
            .as_ref()?
            .headers
            .as_ref()?
            .iter()
            .find_map(|h| {
                if h.name.eq_ignore_ascii_case("from") {
                    // Extract email from "Name <email>" format. Use the last
                    // angle brackets and guard the range: a quoted display name
                    // may legally contain '<' or '>' (e.g. `"a > b" <x@y>`),
                    // and an unguarded slice on `start > end` panics.
                    if let Some(start) = h.value.rfind('<')
                        && let Some(end) = h.value.rfind('>')
                        && start < end
                    {
                        return Some(h.value[start + 1..end].to_string());
                    }
                    Some(h.value.clone())
                } else {
                    None
                }
            })
    }

    /// Extract subject from message headers.
    fn extract_subject(msg: &GmailMessage) -> Option<String> {
        msg.payload
            .as_ref()?
            .headers
            .as_ref()?
            .iter()
            .find_map(|h| {
                if h.name.eq_ignore_ascii_case("subject") {
                    Some(h.value.clone())
                } else {
                    None
                }
            })
    }

    /// Extract Message-ID header for threading.
    fn extract_message_id_header(msg: &GmailMessage) -> Option<String> {
        msg.payload
            .as_ref()?
            .headers
            .as_ref()?
            .iter()
            .find_map(|h| {
                if h.name.eq_ignore_ascii_case("message-id") {
                    Some(h.value.clone())
                } else {
                    None
                }
            })
    }

    /// Extract plain text body from message.
    fn extract_body(msg: &GmailMessage) -> String {
        // Try snippet first (always available, limited to ~200 chars).
        let snippet = msg.snippet.clone().unwrap_or_default();

        // Try to get full body from payload.
        if let Some(ref payload) = msg.payload
            && let Some(text) = Self::extract_text_from_payload(payload)
        {
            return text;
        }

        snippet
    }

    /// Recursively extract text/plain content from payload.
    fn extract_text_from_payload(payload: &GmailPayload) -> Option<String> {
        // Direct body (for simple messages without MIME parts).
        if payload.mime_type.as_deref() == Some("text/plain")
            && let Some(ref body) = payload.body
            && let Some(ref data) = body.data
        {
            return Self::decode_base64url(data);
        }

        // Check parts (for multipart messages).
        if let Some(ref parts) = payload.parts {
            for part in parts {
                if part.mime_type.as_deref() == Some("text/plain")
                    && let Some(ref body) = part.body
                    && let Some(ref data) = body.data
                {
                    return Self::decode_base64url(data);
                }
                // Recurse into nested parts.
                if let Some(ref nested) = part.parts {
                    for nested_part in nested {
                        if nested_part.mime_type.as_deref() == Some("text/plain")
                            && let Some(ref body) = nested_part.body
                            && let Some(ref data) = body.data
                        {
                            return Self::decode_base64url(data);
                        }
                    }
                }
            }
        }

        None
    }

    /// Decode base64url-encoded string (Gmail API format).
    fn decode_base64url(data: &str) -> Option<String> {
        use base64::Engine;
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let bytes = URL_SAFE_NO_PAD.decode(data).ok()?;
        String::from_utf8(bytes).ok()
    }

    /// Convert a Gmail message to an IncomingMessage.
    fn to_incoming_message(&self, msg: &GmailMessage) -> Option<IncomingMessage> {
        // Never process a message this account itself authored. The history
        // feed surfaces the agent's own outbound replies as `messagesAdded`;
        // without this guard a reply to an allowed sender re-enters the loop and
        // the agent answers its own mail indefinitely.
        if msg
            .label_ids
            .as_ref()
            .is_some_and(|labels| labels.iter().any(|l| l == "SENT"))
        {
            tracing::debug!(
                gmail_message_id = %msg.id,
                "Gmail message is SENT by this account, skipping"
            );
            return None;
        }

        let sender = Self::extract_sender(msg)?;

        // Check sender allowlist.
        if !is_sender_allowed(&sender, &self.config.allowed_senders) {
            tracing::debug!(
                sender = %sender,
                "Gmail message from unauthorized sender, skipping"
            );
            return None;
        }

        // Check message size.
        let body = Self::extract_body(msg);
        if body.len() > self.config.max_message_size_bytes {
            tracing::warn!(
                sender = %sender,
                size = body.len(),
                max = self.config.max_message_size_bytes,
                "Gmail message too large, skipping"
            );
            return None;
        }

        if body.trim().is_empty() {
            tracing::debug!(
                sender = %sender,
                "Gmail message has empty body, skipping"
            );
            return None;
        }

        let subject = Self::extract_subject(msg).unwrap_or_default();
        let message_id_header = Self::extract_message_id_header(msg);
        let thread_id = msg.thread_id.clone();

        let mut incoming = IncomingMessage::new("gmail", &sender, &body)
            .with_user_name(&sender)
            .with_metadata(serde_json::json!({
                "gmail_message_id": msg.id,
                "subject": subject,
                "message_id_header": message_id_header,
                "sender": sender,
            }));

        if let Some(ref tid) = thread_id {
            incoming = incoming.with_thread(tid);
        }

        Some(incoming)
    }

    /// Main polling loop — pulls from Pub/Sub, fetches emails, delivers to agent.
    async fn poll_loop(
        config: GmailConfig,
        state: Arc<GmailChannelState>,
        _http: Client,
        channel: GmailChannel,
    ) {
        tracing::info!(
            project = %config.project_id,
            subscription = %config.subscription_id,
            labels = ?config.label_filters,
            senders = ?config.allowed_senders,
            "Gmail polling loop started"
        );

        loop {
            // Check if still running.
            if !*state.running.read().await {
                tracing::info!("Gmail polling loop stopping");
                break;
            }

            // Pull from Pub/Sub.
            match channel.pull_pubsub().await {
                Ok(received) if !received.is_empty() => {
                    tracing::debug!(count = received.len(), "Received Pub/Sub notifications");

                    // Collect ack IDs for all received messages.
                    let ack_ids: Vec<String> = received.iter().map(|m| m.ack_id.clone()).collect();

                    let latest_notification_history_id =
                        received.iter().fold(None::<u64>, |latest, pubsub_msg| {
                            let history_id = pubsub_msg
                                .message
                                .data
                                .as_ref()
                                .and_then(|data| Self::decode_base64url(data))
                                .and_then(|decoded| {
                                    serde_json::from_str::<GmailNotification>(&decoded).ok()
                                })
                                .and_then(|notification| {
                                    tracing::debug!(
                                        email = ?notification.email_address,
                                        history_id = ?notification.history_id,
                                        "Gmail notification: new mail"
                                    );
                                    notification.history_id
                                });
                            match (latest, history_id) {
                                (Some(current), Some(next)) => Some(current.max(next)),
                                (None, Some(next)) => Some(next),
                                (current, None) => current,
                            }
                        });

                    // Process each notification.
                    for pubsub_msg in &received {
                        // Decode notification data.
                        if let Some(ref data) = pubsub_msg.message.data
                            && let Some(decoded) = Self::decode_base64url(data)
                            && let Ok(notification) =
                                serde_json::from_str::<GmailNotification>(&decoded)
                        {
                            tracing::debug!(
                                email = ?notification.email_address,
                                history_id = ?notification.history_id,
                                "Gmail notification: new mail"
                            );
                        }
                    }

                    // Fetch actual emails from Gmail API.
                    let mut should_ack = false;
                    match channel
                        .fetch_new_messages(latest_notification_history_id)
                        .await
                    {
                        Ok(result) => {
                            let mut delivery_failed = false;
                            for gmail_msg in &result.messages {
                                if let Some(incoming) = channel.to_incoming_message(gmail_msg) {
                                    let tx_guard = state.msg_tx.read().await;
                                    if let Some(ref tx) = *tx_guard {
                                        if tx.send(incoming).await.is_err() {
                                            tracing::error!("Gmail message channel closed");
                                            *state.running.write().await = false;
                                            delivery_failed = true;
                                            break;
                                        }
                                        state
                                            .messages_processed
                                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    }

                                    // Mark as read to avoid re-processing.
                                    if let Err(e) = channel.mark_as_read(&gmail_msg.id).await {
                                        tracing::warn!(
                                            error = %e,
                                            "Failed to mark Gmail message as read"
                                        );
                                    }
                                }
                            }

                            if let Some(history_id) = result.latest_history_id
                                && let Err(e) = channel.persist_history_id(history_id).await
                            {
                                tracing::warn!(
                                    error = %e,
                                    history_id,
                                    "Failed to persist Gmail history cursor"
                                );
                            }

                            if delivery_failed {
                                tracing::warn!(
                                    "Skipping Pub/Sub ack because message delivery failed"
                                );
                            } else {
                                should_ack = true;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to fetch Gmail messages");
                            *state.last_error.write().await = Some(e.to_string());
                        }
                    }

                    // Only acknowledge when we successfully fetched and processed.
                    if should_ack {
                        if let Err(e) = channel.ack_pubsub(ack_ids).await {
                            tracing::warn!(error = %e, "Failed to ack Pub/Sub messages");
                        }
                    } else {
                        tracing::warn!(
                            "Skipping Pub/Sub ack due to processing failure; message will be retried"
                        );
                    }
                }
                Ok(_) => {
                    // No messages — normal, just wait.
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Pub/Sub pull error");
                    *state.last_error.write().await = Some(e.to_string());
                    // Reactively refresh on an auth failure so we recover within
                    // one poll cycle instead of waiting for the proactive timer.
                    if config.can_refresh_token() && Self::is_auth_error(&e) {
                        tracing::info!("Gmail pull failed auth; refreshing access token");
                        if let Err(refresh_err) =
                            Self::refresh_access_token(&config, &state, &channel.http).await
                        {
                            tracing::warn!(error = %refresh_err, "Reactive Gmail token refresh failed");
                        }
                    }
                }
            }

            if sleep_or_gmail_shutdown(&state, POLL_INTERVAL).await {
                break;
            }
        }
    }
}

#[async_trait]
impl Channel for GmailChannel {
    fn name(&self) -> &str {
        "gmail"
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let token = self.state.access_token.read().await.clone();
        if token.is_empty() && !self.config.can_refresh_token() {
            return Err(ChannelError::AuthFailed {
                name: "gmail".into(),
                reason: "No OAuth access token or refresh credentials configured. Authenticate \
                         via the ThinClaw Desktop Gmail setup, or set GMAIL_OAUTH_TOKEN (and \
                         GMAIL_REFRESH_TOKEN/GMAIL_CLIENT_ID/GMAIL_CLIENT_SECRET for auto-refresh)."
                    .into(),
            });
        }

        self.stop_background_tasks().await;

        let (tx, rx) = mpsc::channel(256);
        *self.state.msg_tx.write().await = Some(tx);
        *self.state.running.write().await = true;

        // Proactively keep the access token fresh so a long-running deployment
        // does not silently stop after the initial token expires (~1 hour).
        if self.config.can_refresh_token() {
            let refresh_config = self.config.clone();
            let refresh_state = Arc::clone(&self.state);
            let refresh_http = self.http.clone();
            let handle = tokio::spawn(async move {
                Self::token_refresh_loop(refresh_config, refresh_state, refresh_http).await;
            });
            *self.state.refresh_task.lock().await = Some(handle);
        }

        // Spawn the polling loop.
        // We need a second GmailChannel instance for the loop since `self` can't
        // be moved. Clone the shared state + config and create a channel for the loop.
        let loop_channel = GmailChannel {
            config: self.config.clone(),
            state: Arc::clone(&self.state),
            http: self.http.clone(),
        };

        let config_clone = self.config.clone();
        let state_clone = Arc::clone(&self.state);
        let http_clone = self.http.clone();

        let handle = tokio::spawn(async move {
            Self::poll_loop(config_clone, state_clone, http_clone, loop_channel).await;
        });
        *self.state.poll_task.lock().await = Some(handle);

        tracing::info!(
            project = %self.config.project_id,
            subscription = %self.config.subscription_id,
            labels = ?self.config.label_filters,
            "Gmail channel started"
        );

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Extract reply metadata from the original message.
        let sender = msg
            .metadata
            .get("sender")
            .and_then(|v| v.as_str())
            .unwrap_or(&msg.user_id);

        let subject = msg
            .metadata
            .get("subject")
            .and_then(|v| v.as_str())
            .map(|s| {
                if s.starts_with("Re: ") {
                    s.to_string()
                } else {
                    format!("Re: {}", s)
                }
            })
            .unwrap_or_else(|| "Re: Your message".to_string());

        let message_id_header = msg
            .metadata
            .get("message_id_header")
            .and_then(|v| v.as_str());

        let thread_id = msg.thread_id.as_deref();

        self.send_reply(
            sender,
            &subject,
            &response.content,
            thread_id,
            message_id_header,
            &response.attachments,
        )
        .await?;

        tracing::info!(
            to = %sender,
            subject = %subject,
            "Gmail reply sent"
        );

        Ok(())
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        // Gmail doesn't support real-time status updates.
        // Status is silently dropped (email is async by nature).
        Ok(())
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Only send to valid email addresses.
        // Proactive notifications may arrive with user_id="default".
        if !user_id.contains('@') {
            tracing::debug!(
                recipient = user_id,
                "Gmail: skipping broadcast — recipient is not an email address"
            );
            return Ok(());
        }
        // For broadcast, send a new email (not a reply).
        self.send_reply(
            user_id,
            "Message from ThinClaw",
            &response.content,
            None,
            None,
            &response.attachments,
        )
        .await
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "- Gmail replies are email. Use clear headings, concise paragraphs, and direct prose.\n\
- Avoid markdown tables; they do not render reliably in mail clients.\n\
- Quote or summarize prior context explicitly when replying in a thread."
                .to_string(),
        )
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let running = *self.state.running.read().await;
        if running {
            Ok(())
        } else {
            Err(ChannelError::HealthCheckFailed {
                name: "gmail".into(),
            })
        }
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.stop_background_tasks().await;
        tracing::info!("Gmail channel shut down");
        Ok(())
    }
}

impl GmailChannel {
    async fn stop_background_tasks(&self) {
        *self.state.running.write().await = false;
        self.state.shutdown_notify.notify_waiters();
        *self.state.msg_tx.write().await = None;

        let poll_handle = { self.state.poll_task.lock().await.take() };
        if let Some(handle) = poll_handle {
            drain_channel_task(handle, "gmail-poll").await;
        }

        let refresh_handle = { self.state.refresh_task.lock().await.take() };
        if let Some(handle) = refresh_handle {
            drain_channel_task(handle, "gmail-refresh").await;
        }
    }
}

async fn sleep_or_gmail_shutdown(state: &Arc<GmailChannelState>, duration: Duration) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => !*state.running.read().await,
        _ = state.shutdown_notify.notified() => !*state.running.read().await,
    }
}

async fn drain_channel_task(mut handle: JoinHandle<()>, name: &'static str) {
    tokio::select! {
        result = &mut handle => {
            if let Err(error) = result {
                tracing::warn!(channel = "gmail", task = name, error = %error, "Gmail channel task exited with error");
            }
        }
        _ = tokio::time::sleep(CHANNEL_TASK_SHUTDOWN_TIMEOUT) => {
            handle.abort();
            let _ = handle.await;
            tracing::warn!(channel = "gmail", task = name, "Gmail channel task did not drain before timeout; aborted");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> GmailConfig {
        GmailConfig {
            enabled: true,
            project_id: "test-project".into(),
            subscription_id: "test-sub".into(),
            topic_id: "test-topic".into(),
            oauth_token: Some("test-token".into()),
            ..Default::default()
        }
    }

    #[test]
    fn test_new_requires_configuration() {
        let config = GmailConfig::default();
        let result = GmailChannel::new(config);
        assert!(result.is_err());
    }

    #[test]
    fn can_refresh_token_requires_all_three_credentials() {
        let base = test_config();
        assert!(!base.can_refresh_token(), "no refresh creds yet");

        let full = GmailConfig {
            refresh_token: Some("r".into()),
            client_id: Some("c".into()),
            client_secret: Some("s".into()),
            ..test_config()
        };
        assert!(full.can_refresh_token());

        // Any one missing/empty disables refresh.
        for partial in [
            GmailConfig {
                refresh_token: None,
                ..full.clone()
            },
            GmailConfig {
                client_id: Some(String::new()),
                ..full.clone()
            },
            GmailConfig {
                client_secret: None,
                ..full.clone()
            },
        ] {
            assert!(!partial.can_refresh_token());
        }
    }

    #[test]
    fn is_auth_error_detects_expired_token_signals() {
        assert!(GmailChannel::is_auth_error(&ChannelError::AuthFailed {
            name: "gmail".into(),
            reason: "nope".into(),
        }));
        assert!(GmailChannel::is_auth_error(&ChannelError::SendFailed {
            name: "gmail".into(),
            reason: "HTTP 401 Unauthorized: invalid_token".into(),
        }));
        assert!(!GmailChannel::is_auth_error(&ChannelError::SendFailed {
            name: "gmail".into(),
            reason: "HTTP 500 Internal Server Error".into(),
        }));
    }

    #[test]
    fn token_refresh_response_defaults_expiry() {
        let parsed: GmailTokenRefreshResponse =
            serde_json::from_str(r#"{"access_token":"abc"}"#).unwrap();
        assert_eq!(parsed.access_token, "abc");
        assert_eq!(parsed.expires_in, 3600);

        let parsed: GmailTokenRefreshResponse =
            serde_json::from_str(r#"{"access_token":"abc","expires_in":1799}"#).unwrap();
        assert_eq!(parsed.expires_in, 1799);
    }

    #[test]
    fn test_new_succeeds_with_valid_config() {
        let config = test_config();
        let channel = GmailChannel::new(config).unwrap();
        assert_eq!(channel.name(), "gmail");
    }

    #[test]
    fn test_extract_sender_standard() {
        let msg = GmailMessage {
            id: "1".into(),
            thread_id: None,
            snippet: None,
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "Alice <alice@example.com>".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert_eq!(
            GmailChannel::extract_sender(&msg),
            Some("alice@example.com".into())
        );
    }

    #[test]
    fn test_extract_sender_bare_email() {
        let msg = GmailMessage {
            id: "2".into(),
            thread_id: None,
            snippet: None,
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "bob@example.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert_eq!(
            GmailChannel::extract_sender(&msg),
            Some("bob@example.com".into())
        );
    }

    #[test]
    fn test_extract_sender_malformed_angle_brackets() {
        let msg = GmailMessage {
            id: "2a".into(),
            thread_id: None,
            snippet: None,
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "Bob <bob@example.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert_eq!(
            GmailChannel::extract_sender(&msg),
            Some("Bob <bob@example.com".into())
        );
    }

    #[test]
    fn test_extract_sender_display_name_with_gt_before_lt_does_not_panic() {
        // RFC-legal quoted display name containing '>' before the real '<'.
        // The old first-`<`/first-`>` slice panicked with `begin > end`.
        let msg = GmailMessage {
            id: "2b".into(),
            thread_id: None,
            snippet: None,
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "\"Doe > John\" <evil@example.com>".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert_eq!(
            GmailChannel::extract_sender(&msg),
            Some("evil@example.com".into())
        );
    }

    #[test]
    fn test_to_incoming_message_skips_sent_label() {
        // A message this account authored (SENT label) must never re-enter the
        // agent loop, even from an allowed sender — otherwise the agent answers
        // its own replies forever.
        let channel = GmailChannel::new(GmailConfig {
            allowed_senders: vec![],
            ..test_config()
        })
        .unwrap();
        let msg = GmailMessage {
            id: "sent-1".into(),
            thread_id: Some("t1".into()),
            snippet: Some("a reply the agent just sent".into()),
            label_ids: Some(vec!["SENT".into()]),
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "agent@example.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert!(channel.to_incoming_message(&msg).is_none());
    }

    #[test]
    fn test_send_reply_headers_are_crlf_sanitized() {
        // Header injection: a recipient/subject carrying CR/LF must not be able
        // to smuggle an extra header line.
        assert_eq!(
            sanitize_header_value("a@b.com\r\nBcc: victim@x.com"),
            "a@b.com__Bcc: victim@x.com"
        );
        assert_eq!(sanitize_header_value("plain subject"), "plain subject");
    }

    #[test]
    fn test_extract_subject() {
        let msg = GmailMessage {
            id: "3".into(),
            thread_id: None,
            snippet: None,
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![
                    GmailHeader {
                        name: "From".into(),
                        value: "x@y.com".into(),
                    },
                    GmailHeader {
                        name: "Subject".into(),
                        value: "Hello World".into(),
                    },
                ]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert_eq!(
            GmailChannel::extract_subject(&msg),
            Some("Hello World".into())
        );
    }

    #[test]
    fn test_decode_base64url() {
        let encoded = "SGVsbG8gV29ybGQ"; // "Hello World" in base64url
        let decoded = GmailChannel::decode_base64url(encoded);
        assert_eq!(decoded, Some("Hello World".into()));
    }

    #[test]
    fn test_decode_base64url_invalid() {
        assert_eq!(GmailChannel::decode_base64url("???"), None);
    }

    #[test]
    fn test_extract_body_from_snippet() {
        let msg = GmailMessage {
            id: "4".into(),
            thread_id: None,
            snippet: Some("This is a test snippet".into()),
            label_ids: None,
            internal_date: None,
            payload: None,
        };
        assert_eq!(GmailChannel::extract_body(&msg), "This is a test snippet");
    }

    #[test]
    fn test_extract_body_from_plain_payload() {
        let msg = GmailMessage {
            id: "5".into(),
            thread_id: None,
            snippet: Some("snippet".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: None,
                body: Some(GmailBody {
                    data: Some("SGVsbG8gZnJvbSBlbWFpbA".into()), // "Hello from email"
                    size: None,
                }),
                parts: None,
                mime_type: Some("text/plain".into()),
            }),
        };
        assert_eq!(GmailChannel::extract_body(&msg), "Hello from email");
    }

    #[test]
    fn test_to_incoming_message_filters_unauthorized() {
        let config = GmailConfig {
            allowed_senders: vec!["allowed@example.com".into()],
            ..test_config()
        };
        let channel = GmailChannel::new(config).unwrap();

        let msg = GmailMessage {
            id: "6".into(),
            thread_id: Some("thread-1".into()),
            snippet: Some("Hello".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "stranger@evil.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };

        assert!(channel.to_incoming_message(&msg).is_none());
    }

    #[test]
    fn test_to_incoming_message_accepts_allowed() {
        let config = GmailConfig {
            allowed_senders: vec!["allowed@example.com".into()],
            ..test_config()
        };
        let channel = GmailChannel::new(config).unwrap();

        let msg = GmailMessage {
            id: "7".into(),
            thread_id: Some("thread-2".into()),
            snippet: Some("Hello from allowed".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "allowed@example.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };

        let incoming = channel.to_incoming_message(&msg).unwrap();
        assert_eq!(incoming.channel, "gmail");
        assert_eq!(incoming.user_id, "allowed@example.com");
        assert_eq!(incoming.content, "Hello from allowed");
        assert_eq!(incoming.thread_id, Some("thread-2".into()));
    }

    #[test]
    fn test_to_incoming_message_empty_allowlist_accepts_all() {
        let config = test_config(); // empty allowed_senders
        let channel = GmailChannel::new(config).unwrap();

        let msg = GmailMessage {
            id: "8".into(),
            thread_id: None,
            snippet: Some("Hi there".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "anyone@anywhere.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };

        assert!(channel.to_incoming_message(&msg).is_some());
    }

    #[test]
    fn test_to_incoming_message_skips_empty_body() {
        let config = test_config();
        let channel = GmailChannel::new(config).unwrap();

        let msg = GmailMessage {
            id: "9".into(),
            thread_id: None,
            snippet: Some("   ".into()), // whitespace only
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "user@example.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };

        assert!(channel.to_incoming_message(&msg).is_none());
    }

    #[test]
    fn test_messages_processed_counter() {
        let config = test_config();
        let channel = GmailChannel::new(config).unwrap();
        assert_eq!(channel.messages_processed(), 0);
    }

    #[tokio::test]
    async fn test_start_fails_without_token() {
        let config = GmailConfig {
            enabled: true,
            project_id: "p".into(),
            subscription_id: "s".into(),
            topic_id: "t".into(),
            oauth_token: None,
            ..Default::default()
        };
        let channel = GmailChannel::new(config).unwrap();
        let result = channel.start().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_shutdown() {
        let config = test_config();
        let channel = GmailChannel::new(config).unwrap();
        let result = channel.shutdown().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_health_check_not_started() {
        let config = test_config();
        let channel = GmailChannel::new(config).unwrap();
        let result = channel.health_check().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_access_token() {
        let config = test_config();
        let channel = GmailChannel::new(config).unwrap();
        channel.set_access_token("new-token").await;
        let token = channel.state.access_token.read().await.clone();
        assert_eq!(token, "new-token");
    }

    #[test]
    fn test_extract_body_from_multipart() {
        let msg = GmailMessage {
            id: "10".into(),
            thread_id: None,
            snippet: Some("snippet".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: None,
                body: None,
                parts: Some(vec![
                    GmailPart {
                        mime_type: Some("text/html".into()),
                        body: Some(GmailBody {
                            data: Some("PFBA".into()),
                            size: None,
                        }),
                        parts: None,
                    },
                    GmailPart {
                        mime_type: Some("text/plain".into()),
                        body: Some(GmailBody {
                            data: Some("UGxhaW4gdGV4dA".into()), // "Plain text"
                            size: None,
                        }),
                        parts: None,
                    },
                ]),
                mime_type: Some("multipart/alternative".into()),
            }),
        };
        assert_eq!(GmailChannel::extract_body(&msg), "Plain text");
    }

    #[test]
    fn test_extract_message_id_header() {
        let msg = GmailMessage {
            id: "11".into(),
            thread_id: None,
            snippet: Some("snippet".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![
                    GmailHeader {
                        name: "From".into(),
                        value: "alice@example.com".into(),
                    },
                    GmailHeader {
                        name: "Message-ID".into(),
                        value: "<id-1234@example.com>".into(),
                    },
                ]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert_eq!(
            GmailChannel::extract_message_id_header(&msg),
            Some("<id-1234@example.com>".into())
        );
    }

    #[test]
    fn test_extract_message_id_header_missing() {
        let msg = GmailMessage {
            id: "12".into(),
            thread_id: None,
            snippet: Some("snippet".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "From".into(),
                    value: "alice@example.com".into(),
                }]),
                body: None,
                parts: None,
                mime_type: None,
            }),
        };
        assert_eq!(GmailChannel::extract_message_id_header(&msg), None);
    }

    #[test]
    fn test_extract_text_from_payload_nested_part() {
        let payload = GmailPayload {
            headers: None,
            body: None,
            parts: Some(vec![GmailPart {
                mime_type: Some("multipart/related".into()),
                body: None,
                parts: Some(vec![GmailPart {
                    mime_type: Some("text/plain".into()),
                    body: Some(GmailBody {
                        data: Some("UGxhaW4gbmVzdGVkIHRleHQ".into()), // "Plain nested text"
                        size: None,
                    }),
                    parts: None,
                }]),
            }]),
            mime_type: Some("multipart/mixed".into()),
        };
        let msg = GmailMessage {
            id: "13".into(),
            thread_id: None,
            snippet: Some("snippet".into()),
            label_ids: None,
            internal_date: None,
            payload: Some(payload),
        };
        assert_eq!(GmailChannel::extract_body(&msg), "Plain nested text");
    }
}
