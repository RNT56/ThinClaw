//! BlueBubbles iMessage bridge channel (cross-platform).
//!
//! Connects to a [BlueBubbles](https://bluebubbles.app/) macOS server over
//! REST API + webhooks to send and receive iMessages from any platform.
//!
//! ## Architecture
//!
//! ```text
//! ThinClaw (any OS)          Mac relay
//! ┌──────────────────┐       ┌──────────────────┐
//! │ BlueBubblesChannel│──────▶│ BlueBubbles Server│
//! │  reqwest client   │ REST  │   (macOS app)     │
//! │                  │◀──────│                    │
//! │  webhook handler │ POST  │   Messages.app     │
//! └──────────────────┘       └──────────────────┘
//! ```
//!
//! ## Requirements
//!
//! - BlueBubbles Server v1.0+ running on a macOS device
//! - Server accessible from ThinClaw host (LAN or tunnel)
//! - Password configured on the server

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::{RwLock, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;

/// Channel name constant.
const NAME: &str = "bluebubbles";

/// Default webhook listen host.
const DEFAULT_WEBHOOK_HOST: &str = "127.0.0.1";

/// Default webhook listen port.
const DEFAULT_WEBHOOK_PORT: u16 = 8645;

/// Default webhook path.
const DEFAULT_WEBHOOK_PATH: &str = "/bluebubbles-webhook";

/// Maximum text length for a single iMessage.
const MAX_TEXT_LENGTH: usize = 4000;

/// BlueBubbles webhook event types that carry messages.
const MESSAGE_EVENTS: &[&str] = &["new-message", "message", "updated-message"];

/// Tapback reaction codes (BlueBubbles `associatedMessageType` values).
/// When a webhook payload contains one of these, the event is a tapback
/// reaction — not a user–typed message — and must be filtered out.
const TAPBACK_CODES: &[i64] = &[
    // Added
    2000, 2001, 2002, 2003, 2004, 2005,
    // Removed
    3000, 3001, 3002, 3003, 3004, 3005,
];

// ── Configuration ───────────────────────────────────────────────────

/// BlueBubbles channel configuration.
#[derive(Debug, Clone)]
pub struct BlueBubblesConfig {
    /// BlueBubbles server URL (e.g. "http://192.168.1.50:1234").
    pub server_url: String,
    /// Server password for API authentication.
    pub password: SecretString,
    /// Webhook listen host.
    pub webhook_host: String,
    /// Webhook listen port.
    pub webhook_port: u16,
    /// Webhook URL path.
    pub webhook_path: String,
    /// Allowed phone numbers / email addresses (empty = allow all).
    pub allow_from: Vec<String>,
    /// Whether to send read receipts (requires Private API on server).
    pub send_read_receipts: bool,
}

impl Default for BlueBubblesConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            password: SecretString::from(String::new()),
            webhook_host: DEFAULT_WEBHOOK_HOST.to_string(),
            webhook_port: DEFAULT_WEBHOOK_PORT,
            webhook_path: DEFAULT_WEBHOOK_PATH.to_string(),
            allow_from: Vec::new(),
            send_read_receipts: true,
        }
    }
}

impl From<&crate::config::BlueBubblesChannelConfig> for BlueBubblesConfig {
    fn from(cfg: &crate::config::BlueBubblesChannelConfig) -> Self {
        Self {
            server_url: normalize_server_url(&cfg.server_url),
            password: cfg.password.clone(),
            webhook_host: cfg.webhook_host.clone(),
            webhook_port: cfg.webhook_port,
            webhook_path: if cfg.webhook_path.starts_with('/') {
                cfg.webhook_path.clone()
            } else {
                format!("/{}", cfg.webhook_path)
            },
            allow_from: cfg.allow_from.clone(),
            send_read_receipts: cfg.send_read_receipts,
        }
    }
}

// ── Diagnostics ─────────────────────────────────────────────────────

/// BlueBubbles channel diagnostic information.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BlueBubblesDiagnostic {
    /// Whether the server is reachable.
    pub server_reachable: bool,
    /// Server URL (redacted).
    pub server_url: String,
    /// Whether the Private API is enabled on the server.
    pub private_api_enabled: Option<bool>,
    /// Whether the helper is connected on the server.
    pub helper_connected: Option<bool>,
    /// Whether the webhook is registered.
    pub webhook_registered: bool,
    /// Errors found during diagnostic.
    pub errors: Vec<String>,
}

// ── Channel implementation ──────────────────────────────────────────

/// BlueBubbles iMessage channel.
///
/// Communicates with a BlueBubbles macOS server via REST API for sending
/// and a webhook listener for receiving messages.
pub struct BlueBubblesChannel {
    config: BlueBubblesConfig,
    client: Client,
    shutdown: Arc<AtomicBool>,
    /// Whether the server has Private API enabled.
    private_api: Arc<AtomicBool>,
    /// Whether the helper bundle is connected.
    helper_connected: Arc<AtomicBool>,
    /// LRU cache: phone/email → BlueBubbles chat GUID.
    guid_cache: Arc<RwLock<HashMap<String, String>>>,
    /// Sender half for pushing incoming messages from the webhook handler.
    incoming_tx: Option<mpsc::Sender<IncomingMessage>>,
    /// Receiver half — taken once by `start()` and returned as the stream.
    incoming_rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    /// ID of our registered webhook (for cleanup).
    webhook_id: Arc<RwLock<Option<i64>>>,
}

impl BlueBubblesChannel {
    /// Create a new BlueBubbles channel.
    pub async fn new(config: BlueBubblesConfig) -> Result<Self, ChannelError> {
        if config.server_url.is_empty() {
            return Err(ChannelError::Configuration(
                "BlueBubbles server URL is required".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ChannelError::Configuration(format!("HTTP client error: {e}")))?;

        // Pre-create the mpsc channel for incoming messages.
        // tx is cloned into the webhook handler; rx is taken by start().
        let (tx, rx) = mpsc::channel(64);

        Ok(Self {
            config,
            client,
            shutdown: Arc::new(AtomicBool::new(false)),
            private_api: Arc::new(AtomicBool::new(false)),
            helper_connected: Arc::new(AtomicBool::new(false)),
            guid_cache: Arc::new(RwLock::new(HashMap::new())),
            incoming_tx: Some(tx),
            incoming_rx: tokio::sync::Mutex::new(Some(rx)),
            webhook_id: Arc::new(RwLock::new(None)),
        })
    }

    // ── API helpers ─────────────────────────────────────────────────

    /// Build a full API URL with password authentication.
    fn api_url(&self, path: &str) -> String {
        let sep = if path.contains('?') { "&" } else { "?" };
        let password = urlencoding::encode(self.config.password.expose_secret());
        format!("{}{}{sep}password={password}", self.config.server_url, path)
    }

    /// GET request to the BlueBubbles API.
    async fn api_get(&self, path: &str) -> Result<serde_json::Value, ChannelError> {
        let url = self.api_url(path);
        let res = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("API GET {path} failed: {e}"),
            })?;

        if !res.status().is_success() {
            return Err(ChannelError::Disconnected {
                name: NAME.to_string(),
                reason: format!("API GET {path} returned {}", res.status()),
            });
        }

        res.json().await.map_err(|e| ChannelError::Disconnected {
            name: NAME.to_string(),
            reason: format!("API GET {path} parse error: {e}"),
        })
    }

    /// POST request to the BlueBubbles API.
    async fn api_post(
        &self,
        path: &str,
        payload: &serde_json::Value,
    ) -> Result<serde_json::Value, ChannelError> {
        let url = self.api_url(path);
        let res = self
            .client
            .post(&url)
            .json(payload)
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("API POST {path} failed: {e}"),
            })?;

        if !res.status().is_success() {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("API POST {path} returned {}", res.status()),
            });
        }

        res.json().await.map_err(|e| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("API POST {path} parse error: {e}"),
        })
    }

    // ── Connection ──────────────────────────────────────────────────

    /// Connect to the BlueBubbles server and detect capabilities.
    async fn connect(&mut self) -> Result<(), ChannelError> {
        // Ping
        self.api_get("/api/v1/ping").await.map_err(|e| {
            ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: format!(
                    "Cannot reach BlueBubbles server at {}: {e}",
                    self.config.server_url
                ),
            }
        })?;

        // Get server info
        match self.api_get("/api/v1/server/info").await {
            Ok(info) => {
                let data = info.get("data").cloned().unwrap_or_default();
                let pa = data
                    .get("private_api")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let hc = data
                    .get("helper_connected")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.private_api.store(pa, Ordering::Relaxed);
                self.helper_connected.store(hc, Ordering::Relaxed);
                tracing::info!(
                    server = %redact_url(&self.config.server_url),
                    private_api = pa,
                    helper_connected = hc,
                    "BlueBubbles: connected to server"
                );
            }
            Err(e) => {
                tracing::warn!("BlueBubbles: could not fetch server info: {e}");
            }
        }

        Ok(())
    }

    // ── Chat GUID resolution ────────────────────────────────────────

    /// Resolve a phone/email to a BlueBubbles chat GUID.
    ///
    /// If `target` already contains a semicolon (raw GUID format like
    /// `iMessage;-;user@example.com`), it is returned as-is.
    async fn resolve_chat_guid(&self, target: &str) -> Result<Option<String>, ChannelError> {
        let target = target.trim();
        if target.is_empty() {
            return Ok(None);
        }

        // Already a raw GUID
        if target.contains(';') {
            return Ok(Some(target.to_string()));
        }

        // Check cache
        {
            let cache = self.guid_cache.read().await;
            if let Some(guid) = cache.get(target) {
                return Ok(Some(guid.clone()));
            }
        }

        // Query BlueBubbles for chats
        let payload = serde_json::json!({
            "limit": 100,
            "offset": 0,
            "with": ["participants"]
        });

        match self.api_post("/api/v1/chat/query", &payload).await {
            Ok(res) => {
                if let Some(chats) = res.get("data").and_then(|d| d.as_array()) {
                    for chat in chats {
                        let guid = chat
                            .get("guid")
                            .or(chat.get("chatGuid"))
                            .and_then(|v| v.as_str());
                        let identifier = chat
                            .get("chatIdentifier")
                            .or(chat.get("identifier"))
                            .and_then(|v| v.as_str());

                        if identifier == Some(target) {
                            if let Some(g) = guid {
                                let mut cache = self.guid_cache.write().await;
                                cache.insert(target.to_string(), g.to_string());
                                return Ok(Some(g.to_string()));
                            }
                        }

                        // Check participants
                        if let Some(participants) =
                            chat.get("participants").and_then(|p| p.as_array())
                        {
                            for part in participants {
                                let addr =
                                    part.get("address").and_then(|a| a.as_str()).unwrap_or("");
                                if addr.trim() == target {
                                    if let Some(g) = guid {
                                        let mut cache = self.guid_cache.write().await;
                                        cache.insert(target.to_string(), g.to_string());
                                        return Ok(Some(g.to_string()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::debug!("BlueBubbles: chat query failed: {e}");
            }
        }

        Ok(None)
    }

    // ── Webhook lifecycle ───────────────────────────────────────────

    /// Compute the webhook URL that BlueBubbles should POST events to.
    fn webhook_url(&self) -> String {
        let host = match self.config.webhook_host.as_str() {
            "0.0.0.0" | "::" => "localhost",
            h => h,
        };
        let password = urlencoding::encode(self.config.password.expose_secret());
        format!(
            "http://{}:{}{}?password={}",
            host, self.config.webhook_port, self.config.webhook_path, password
        )
    }

    /// Register our webhook URL with the BlueBubbles server.
    ///
    /// Checks for existing registrations first (crash resilience).
    async fn register_webhook(&self) -> Result<(), ChannelError> {
        let url = self.webhook_url();

        // Check for existing registration
        if let Ok(res) = self.api_get("/api/v1/webhook").await {
            if let Some(webhooks) = res.get("data").and_then(|d| d.as_array()) {
                for wh in webhooks {
                    if wh.get("url").and_then(|u| u.as_str()) == Some(&url) {
                        let id = wh.get("id").and_then(|i| i.as_i64());
                        if let Some(id) = id {
                            let mut wid = self.webhook_id.write().await;
                            *wid = Some(id);
                        }
                        tracing::info!("BlueBubbles: reusing existing webhook registration");
                        return Ok(());
                    }
                }
            }
        }

        // Register new webhook
        let payload = serde_json::json!({
            "url": url,
            "events": ["new-message", "updated-message"]
        });

        match self.api_post("/api/v1/webhook", &payload).await {
            Ok(res) => {
                let status = res
                    .get("status")
                    .and_then(|s| s.as_i64())
                    .unwrap_or(0);
                if (200..300).contains(&status) {
                    // Extract webhook ID for cleanup
                    let id = res
                        .get("data")
                        .and_then(|d| d.get("id"))
                        .and_then(|i| i.as_i64());
                    if let Some(id) = id {
                        let mut wid = self.webhook_id.write().await;
                        *wid = Some(id);
                    }
                    tracing::info!("BlueBubbles: webhook registered with server");
                    Ok(())
                } else {
                    tracing::warn!(
                        status,
                        "BlueBubbles: webhook registration returned non-success"
                    );
                    Ok(()) // Non-fatal — polling fallback could be added later
                }
            }
            Err(e) => {
                tracing::warn!("BlueBubbles: failed to register webhook: {e}");
                Ok(()) // Non-fatal
            }
        }
    }

    /// Unregister our webhook on shutdown.
    async fn unregister_webhook(&self) {
        let wid = self.webhook_id.read().await;
        if let Some(id) = *wid {
            let url = self.api_url(&format!("/api/v1/webhook/{id}"));
            match self.client.delete(&url).send().await {
                Ok(res) if res.status().is_success() => {
                    tracing::info!("BlueBubbles: webhook unregistered");
                }
                Ok(res) => {
                    tracing::debug!(
                        status = %res.status(),
                        "BlueBubbles: webhook unregister returned non-success (non-critical)"
                    );
                }
                Err(e) => {
                    tracing::debug!("BlueBubbles: webhook unregister failed (non-critical): {e}");
                }
            }
        }
    }

    // ── Text sending ────────────────────────────────────────────────

    /// Send a text message to a chat GUID.
    ///
    /// Optionally threads the reply to a specific message GUID when the
    /// Private API + helper are available.
    async fn send_text(
        &self,
        chat_guid: &str,
        text: &str,
        reply_to: Option<&str>,
    ) -> Result<(), ChannelError> {
        let chunks = split_message(text, MAX_TEXT_LENGTH);
        for chunk in chunks {
            let mut payload = serde_json::json!({
                "chatGuid": chat_guid,
                "tempGuid": format!("temp-{}", uuid::Uuid::new_v4()),
                "message": chunk,
            });

            // Thread the reply if Private API is available
            if let Some(original_guid) = reply_to {
                if self.private_api.load(Ordering::Relaxed)
                    && self.helper_connected.load(Ordering::Relaxed)
                {
                    let obj = payload.as_object_mut().unwrap();
                    obj.insert("method".into(), serde_json::json!("private-api"));
                    obj.insert(
                        "selectedMessageGuid".into(),
                        serde_json::json!(original_guid),
                    );
                    obj.insert("partIndex".into(), serde_json::json!(0));
                }
            }

            self.api_post("/api/v1/message/text", &payload).await?;
        }
        Ok(())
    }

    // ── Outbound attachment sending ─────────────────────────────────

    /// Send a file attachment via BlueBubbles multipart upload.
    ///
    /// Supports any file type — the server infers the MIME or uses the
    /// provided filename. An optional caption is sent as a follow-up text.
    async fn send_attachment(
        &self,
        chat_guid: &str,
        data: Vec<u8>,
        filename: &str,
        mime: &str,
        caption: Option<&str>,
        is_audio_message: bool,
    ) -> Result<(), ChannelError> {
        let url = self.api_url("/api/v1/message/attachment");

        let file_part = reqwest::multipart::Part::bytes(data)
            .file_name(filename.to_string())
            .mime_str(mime)
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("Invalid MIME type '{mime}': {e}"),
            })?;

        let mut form = reqwest::multipart::Form::new()
            .part("attachment", file_part)
            .text("chatGuid", chat_guid.to_string())
            .text("name", filename.to_string())
            .text("tempGuid", uuid::Uuid::new_v4().to_string());

        if is_audio_message {
            form = form.text("isAudioMessage", "true");
        }

        let res = self
            .client
            .post(&url)
            .multipart(form)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
            .map_err(|e| ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("Attachment upload failed: {e}"),
            })?;

        if !res.status().is_success() {
            return Err(ChannelError::SendFailed {
                name: NAME.to_string(),
                reason: format!("Attachment upload returned {}", res.status()),
            });
        }

        // Send caption as a follow-up text message
        if let Some(cap) = caption {
            if !cap.is_empty() {
                self.send_text(chat_guid, cap, None).await?;
            }
        }

        Ok(())
    }

    // ── Typing indicators ───────────────────────────────────────────

    /// Send a typing indicator (requires Private API).
    async fn send_typing_indicator(&self, chat_guid: &str) {
        if !self.private_api.load(Ordering::Relaxed)
            || !self.helper_connected.load(Ordering::Relaxed)
        {
            return;
        }

        let encoded = urlencoding::encode(chat_guid);
        let url = self.api_url(&format!("/api/v1/chat/{encoded}/typing"));
        let _ = self
            .client
            .post(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
    }

    /// Cancel a typing indicator (requires Private API).
    async fn stop_typing_indicator(&self, chat_guid: &str) {
        if !self.private_api.load(Ordering::Relaxed)
            || !self.helper_connected.load(Ordering::Relaxed)
        {
            return;
        }

        let encoded = urlencoding::encode(chat_guid);
        let url = self.api_url(&format!("/api/v1/chat/{encoded}/typing"));
        let _ = self
            .client
            .delete(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
    }

    // ── Read receipts ───────────────────────────────────────────────

    /// Mark a chat as read (requires Private API).
    #[allow(dead_code)] // Called from webhook handler's fire-and-forget path
    async fn mark_read(&self, chat_guid: &str) {
        if !self.private_api.load(Ordering::Relaxed)
            || !self.helper_connected.load(Ordering::Relaxed)
        {
            return;
        }

        let encoded = urlencoding::encode(chat_guid);
        let url = self.api_url(&format!("/api/v1/chat/{encoded}/read"));
        let _ = self
            .client
            .post(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
    }

    // ── Chat info ───────────────────────────────────────────────────

    /// Query chat metadata and participant list from BlueBubbles.
    #[allow(dead_code)] // Public integration point for diagnostics and tools
    async fn get_chat_info(
        &self,
        chat_id: &str,
    ) -> Result<serde_json::Value, ChannelError> {
        let guid = self.resolve_chat_guid(chat_id).await?;
        let guid = guid.as_deref().unwrap_or(chat_id);
        let encoded = urlencoding::encode(guid);
        let res = self
            .api_get(&format!("/api/v1/chat/{encoded}?with=participants"))
            .await?;

        let data = res.get("data").cloned().unwrap_or_default();
        let display_name = data
            .get("displayName")
            .or(data.get("chatIdentifier"))
            .and_then(|v| v.as_str())
            .unwrap_or(chat_id);

        let participants: Vec<String> = data
            .get("participants")
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| p.get("address").and_then(|a| a.as_str()))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let is_group = guid.contains(";+;");
        Ok(serde_json::json!({
            "name": display_name,
            "type": if is_group { "group" } else { "dm" },
            "participants": participants,
        }))
    }

    // ── Inbound attachment download ─────────────────────────────────

    /// Download an attachment from BlueBubbles by its GUID.
    #[allow(dead_code)] // Integration point; webhook handler downloads inline for now
    async fn download_attachment(
        &self,
        att_guid: &str,
        mime: &str,
    ) -> Option<crate::media::MediaContent> {
        let encoded = urlencoding::encode(att_guid);
        let url = self.api_url(&format!("/api/v1/attachment/{encoded}/download"));
        match self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => match res.bytes().await {
                Ok(data) => {
                    let mc = crate::media::MediaContent::new(data.to_vec(), mime);
                    Some(mc)
                }
                Err(e) => {
                    tracing::warn!("BlueBubbles: attachment body read failed: {e}");
                    None
                }
            },
            Ok(res) => {
                tracing::warn!(
                    status = %res.status(),
                    "BlueBubbles: attachment download returned non-success"
                );
                None
            }
            Err(e) => {
                tracing::warn!("BlueBubbles: attachment download failed: {e}");
                None
            }
        }
    }

    // ── Webhook request handler ─────────────────────────────────────

    /// Build the axum router for the webhook listener.
    pub fn webhook_routes(&self) -> axum::Router {
        let state = WebhookState {
            password: self.config.password.clone(),
            allow_from: self.config.allow_from.clone(),
            send_read_receipts: self.config.send_read_receipts,
            tx: self.incoming_tx.clone(),
            client: self.client.clone(),
            config: self.config.clone(),
            private_api: Arc::clone(&self.private_api),
            helper_connected: Arc::clone(&self.helper_connected),
        };

        axum::Router::new()
            .route(
                &self.config.webhook_path,
                axum::routing::post(handle_webhook),
            )
            .with_state(state)
    }
}

// ── Channel trait ───────────────────────────────────────────────────

#[async_trait]
impl Channel for BlueBubblesChannel {
    fn name(&self) -> &str {
        NAME
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        // Take the pre-created receiver out of the Mutex.
        // This can only be called once (subsequent calls return an error).
        let rx = self
            .incoming_rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: NAME.to_string(),
                reason: "BlueBubbles channel already started (stream taken)".to_string(),
            })?;

        // The webhook handler pushes messages into incoming_tx;
        // the returned stream is what ChannelManager merges into the agent loop.
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Send outbound attachments if the response metadata carries them.
        // The agent pipeline attaches `MediaContent` items to the outgoing
        // response via the `attachments` metadata key when tool calls
        // produce files (e.g. image generation, doc export).
        if let Some(attachments) = msg.metadata.get("response_attachments") {
            if let Some(arr) = attachments.as_array() {
                let chat_id_for_att = msg
                    .metadata
                    .get("chat_guid")
                    .and_then(|v| v.as_str())
                    .or_else(|| msg.metadata.get("chat_id").and_then(|v| v.as_str()))
                    .unwrap_or(&msg.user_id);
                if let Some(resolved_att) = self.resolve_chat_guid(chat_id_for_att).await? {
                    for att in arr {
                        let data_b64 = att.get("data").and_then(|d| d.as_str()).unwrap_or("");
                        let fname = att
                            .get("filename")
                            .and_then(|f| f.as_str())
                            .unwrap_or("attachment");
                        let mime = att
                            .get("mime_type")
                            .and_then(|m| m.as_str())
                            .unwrap_or("application/octet-stream");
                        if let Ok(bytes) = base64_decode(data_b64) {
                            let is_audio = mime.starts_with("audio/");
                            if let Err(e) = self
                                .send_attachment(&resolved_att, bytes, fname, mime, None, is_audio)
                                .await
                            {
                                tracing::warn!(error = %e, "BlueBubbles: failed to send attachment");
                            }
                        }
                    }
                }
            }
        }

        let chat_guid = msg
            .metadata
            .get("chat_guid")
            .and_then(|v| v.as_str())
            .or_else(|| msg.metadata.get("chat_id").and_then(|v| v.as_str()))
            .unwrap_or(&msg.user_id);

        // Extract reply-to message GUID for threading
        let reply_to = msg
            .metadata
            .get("message_id")
            .and_then(|v| v.as_str());

        // Resolve GUID if needed
        let resolved = self
            .resolve_chat_guid(chat_guid)
            .await?
            .unwrap_or_else(|| chat_guid.to_string());

        self.send_text(&resolved, &response.content, reply_to).await
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        match &status {
            StatusUpdate::Thinking(_) => {
                if let Some(chat_guid) = metadata.get("chat_guid").and_then(|v| v.as_str()) {
                    self.send_typing_indicator(chat_guid).await;
                }
            }
            // Stop typing when the agent finishes (lifecycle end or stream chunk arrival)
            StatusUpdate::LifecycleEnd { .. } => {
                if let Some(chat_guid) = metadata.get("chat_guid").and_then(|v| v.as_str()) {
                    self.stop_typing_indicator(chat_guid).await;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "- iMessage (via BlueBubbles) renders plain text best. Avoid markdown.\n\
             - Keep replies compact and conversational."
                .to_string(),
        )
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        // Resolve the user_id to a chat GUID
        match self.resolve_chat_guid(user_id).await? {
            Some(guid) => self.send_text(&guid, &response.content, None).await,
            None => {
                // Try creating a new chat if it looks like a valid address
                if user_id.contains('@') || user_id.starts_with('+') {
                    let payload = serde_json::json!({
                        "addresses": [user_id],
                        "message": response.content,
                        "tempGuid": format!("temp-{}", uuid::Uuid::new_v4()),
                    });
                    self.api_post("/api/v1/chat/new", &payload).await?;
                    Ok(())
                } else {
                    tracing::debug!(
                        recipient = %redact_pii(user_id),
                        "BlueBubbles: skipping broadcast — not a valid iMessage address"
                    );
                    Ok(())
                }
            }
        }
    }

    async fn send_typing(&self, chat_id: &str) -> Result<(), ChannelError> {
        self.send_typing_indicator(chat_id).await;
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.api_get("/api/v1/ping").await.map(|_| ())
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        let server_reachable = self.api_get("/api/v1/ping").await.is_ok();
        let mut errors = Vec::new();
        if !server_reachable {
            errors.push("BlueBubbles server is not reachable".to_string());
        }

        let webhook_registered = self.webhook_id.read().await.is_some();
        if !webhook_registered {
            errors.push("Webhook is not registered with the server".to_string());
        }

        let diag = BlueBubblesDiagnostic {
            server_reachable,
            server_url: redact_url(&self.config.server_url),
            private_api_enabled: Some(self.private_api.load(Ordering::Relaxed)),
            helper_connected: Some(self.helper_connected.load(Ordering::Relaxed)),
            webhook_registered,
            errors,
        };
        serde_json::to_value(&diag).ok()
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        self.shutdown.store(true, Ordering::Relaxed);
        self.unregister_webhook().await;
        Ok(())
    }
}

// ── Webhook handler state ───────────────────────────────────────────

#[derive(Clone)]
struct WebhookState {
    password: SecretString,
    allow_from: Vec<String>,
    send_read_receipts: bool,
    tx: Option<mpsc::Sender<IncomingMessage>>,
    client: Client,
    config: BlueBubblesConfig,
    private_api: Arc<AtomicBool>,
    helper_connected: Arc<AtomicBool>,
}

/// Handle incoming webhook POST from BlueBubbles server.
async fn handle_webhook(
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
    axum::extract::State(state): axum::extract::State<WebhookState>,
    body: axum::body::Bytes,
) -> axum::http::StatusCode {
    // Validate password
    let token = params
        .get("password")
        .or(params.get("guid"))
        .map(|s| s.as_str())
        .unwrap_or("");
    if token != state.password.expose_secret() {
        return axum::http::StatusCode::UNAUTHORIZED;
    }

    // Parse body — try JSON first, then form-encoded fallback (robustness
    // against misconfigured BlueBubbles webhook format).
    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => {
            // Form-encoded fallback: look for payload=, data=, or message= keys
            let body_str = String::from_utf8_lossy(&body);
            let mut parsed = serde_json::Value::Null;
            for key in &["payload", "data", "message"] {
                let prefix = format!("{key}=");
                if let Some(rest) = body_str.strip_prefix(&prefix) {
                    if let Ok(decoded) = urlencoding::decode(rest) {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&decoded) {
                            parsed = v;
                            break;
                        }
                    }
                }
            }
            if parsed.is_null() {
                tracing::debug!("BlueBubbles: webhook body was neither JSON nor form-encoded");
                return axum::http::StatusCode::BAD_REQUEST;
            }
            parsed
        }
    };

    // Extract event type
    let event_type = payload
        .get("type")
        .or(payload.get("event"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // Only process message events
    if !event_type.is_empty() && !MESSAGE_EVENTS.contains(&event_type) {
        return axum::http::StatusCode::OK;
    }

    // Extract message record from payload
    let record = extract_record(&payload);
    let record = match record {
        Some(r) => r,
        None => return axum::http::StatusCode::OK,
    };

    // Skip outgoing messages
    let is_from_me = record
        .get("isFromMe")
        .or(record.get("fromMe"))
        .or(record.get("is_from_me"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Skip tapback reactions delivered as messages
    let assoc_type = record
        .get("associatedMessageType")
        .and_then(|v| v.as_i64());
    if let Some(code) = assoc_type {
        if TAPBACK_CODES.contains(&code) {
            return axum::http::StatusCode::OK;
        }
    }

    if is_from_me {
        return axum::http::StatusCode::OK;
    }

    // Extract text
    let text = first_string(&[
        record.get("text"),
        record.get("message"),
        record.get("body"),
    ])
    .unwrap_or_default();

    // Extract attachments
    let attachments = record
        .get("attachments")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();

    if text.is_empty() && attachments.is_empty() {
        return axum::http::StatusCode::OK;
    }

    // Extract chat GUID
    let chat_guid = first_string(&[
        record.get("chatGuid"),
        payload.get("chatGuid"),
        record.get("chat_guid"),
        payload.get("chat_guid"),
    ])
    .or_else(|| {
        // BlueBubbles v1.9+: nested under data.chats[0].guid
        record
            .get("chats")
            .and_then(|c| c.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| {
                c.get("guid")
                    .or(c.get("chatGuid"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
    });

    let chat_identifier = first_string(&[
        record.get("chatIdentifier"),
        record.get("identifier"),
        payload.get("chatIdentifier"),
    ]);

    // Extract sender
    let sender = record
        .get("handle")
        .and_then(|h| h.get("address"))
        .and_then(|a| a.as_str())
        .map(|s| s.to_string())
        .or_else(|| first_string(&[record.get("sender"), record.get("from"), record.get("address")]))
        .or_else(|| chat_identifier.clone())
        .or_else(|| chat_guid.clone());

    let sender = match sender {
        Some(s) => s,
        None => return axum::http::StatusCode::BAD_REQUEST,
    };

    let session_chat_id = chat_guid
        .as_deref()
        .or(chat_identifier.as_deref())
        .unwrap_or(&sender);

    // Check allow-list
    if !state.allow_from.is_empty()
        && !state
            .allow_from
            .iter()
            .any(|a| a == "*" || a == &sender)
    {
        return axum::http::StatusCode::OK;
    }

    // Download attachments
    let mut media_attachments = Vec::new();
    for att in &attachments {
        let att_guid = att.get("guid").and_then(|g| g.as_str()).unwrap_or("");
        if att_guid.is_empty() {
            continue;
        }
        let mime = att
            .get("mimeType")
            .and_then(|m| m.as_str())
            .unwrap_or("application/octet-stream");

        let encoded = urlencoding::encode(att_guid);
        let password = urlencoding::encode(state.config.password.expose_secret());
        let url = format!(
            "{}/api/v1/attachment/{encoded}/download?password={password}",
            state.config.server_url
        );

        match state
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(60))
            .send()
            .await
        {
            Ok(res) if res.status().is_success() => {
                if let Ok(data) = res.bytes().await {
                    let mc = crate::media::MediaContent::new(data.to_vec(), mime);
                    media_attachments.push(mc);
                }
            }
            _ => {
                tracing::debug!(att_guid = %redact_pii(att_guid), "BlueBubbles: attachment download skipped");
            }
        }
    }

    let content = if text.is_empty() && !media_attachments.is_empty() {
        "[Media received — please analyze the attached content]".to_string()
    } else {
        text
    };

    let is_group = record
        .get("isGroup")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || chat_guid
            .as_deref()
            .map(|g| g.contains(";+;"))
            .unwrap_or(false);

    let conversation_kind = if is_group { "group" } else { "direct" };

    let message_id = first_string(&[
        record.get("guid"),
        record.get("messageGuid"),
        record.get("id"),
    ]);

    let incoming = IncomingMessage::new(NAME, &sender, &content)
        .with_metadata(serde_json::json!({
            "chat_guid": session_chat_id,
            "chat_id": session_chat_id,
            "is_group": is_group,
            "conversation_kind": conversation_kind,
            "conversation_scope_id": format!("bluebubbles:{conversation_kind}:{session_chat_id}"),
            "external_conversation_key": format!("bluebubbles://{conversation_kind}/{session_chat_id}"),
            "raw_sender_id": sender,
            "stable_sender_id": sender,
            "message_id": message_id,
            "reply_to_message_id": first_string(&[
                record.get("threadOriginatorGuid"),
                record.get("associatedMessageGuid"),
            ]),
            "attachment_count": media_attachments.len(),
        }))
        .with_attachments(media_attachments);

    // Send to channel
    if let Some(ref tx) = state.tx {
        if tx.send(incoming).await.is_err() {
            tracing::warn!("BlueBubbles: incoming message channel dropped");
        }
    }

    // Fire-and-forget read receipt
    if state.send_read_receipts
        && state.private_api.load(Ordering::Relaxed)
        && state.helper_connected.load(Ordering::Relaxed)
    {
        let client = state.client.clone();
        let password = urlencoding::encode(state.config.password.expose_secret()).to_string();
        let server_url = state.config.server_url.clone();
        let chat = session_chat_id.to_string();
        tokio::spawn(async move {
            let encoded = urlencoding::encode(&chat);
            let url = format!(
                "{server_url}/api/v1/chat/{encoded}/read?password={password}"
            );
            let _ = client
                .post(&url)
                .timeout(std::time::Duration::from_secs(5))
                .send()
                .await;
        });
    }

    axum::http::StatusCode::OK
}

// ── Init helper ─────────────────────────────────────────────────────

impl BlueBubblesChannel {
    /// Initialize the channel: connect to server, register webhook, and
    /// prepare for use. After calling this, add the channel to the
    /// ChannelManager which will call `start()` to get the message stream.
    ///
    /// Also starts the standalone webhook HTTP listener on the configured
    /// host/port. The webhook handler pushes incoming messages into the
    /// mpsc channel that `start()` returns.
    pub async fn init(config: BlueBubblesConfig) -> Result<Self, ChannelError> {
        let mut channel = Self::new(config).await?;
        channel.connect().await?;

        // Register webhook with the BlueBubbles server
        channel.register_webhook().await?;

        // Start the standalone webhook HTTP listener
        let router = channel.webhook_routes();
        let host = channel.config.webhook_host.clone();
        let port = channel.config.webhook_port;
        let shutdown = Arc::clone(&channel.shutdown);

        tokio::spawn(async move {
            let addr = format!("{host}:{port}");
            let listener = match tokio::net::TcpListener::bind(&addr).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!(
                        addr = %addr,
                        error = %e,
                        "BlueBubbles: failed to bind webhook listener"
                    );
                    return;
                }
            };
            tracing::info!(addr = %addr, "BlueBubbles: webhook listener started");

            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    loop {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                })
                .await
                .ok();
        });

        Ok(channel)
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

/// Normalize a server URL (add http:// if missing, strip trailing slash).
fn normalize_server_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let with_scheme = if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        format!("http://{trimmed}")
    } else {
        trimmed.to_string()
    };
    with_scheme.trim_end_matches('/').to_string()
}

/// Extract the message record from a BlueBubbles webhook payload.
fn extract_record(payload: &serde_json::Value) -> Option<serde_json::Value> {
    if let Some(data) = payload.get("data") {
        if data.is_object() {
            return Some(data.clone());
        }
        if let Some(arr) = data.as_array() {
            if let Some(first) = arr.first() {
                if first.is_object() {
                    return Some(first.clone());
                }
            }
        }
    }
    if let Some(msg) = payload.get("message") {
        if msg.is_object() {
            return Some(msg.clone());
        }
    }
    if payload.is_object() {
        return Some(payload.clone());
    }
    None
}

/// Get the first non-empty string from a slice of optional JSON values.
fn first_string(candidates: &[Option<&serde_json::Value>]) -> Option<String> {
    for candidate in candidates {
        if let Some(val) = candidate {
            if let Some(s) = val.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

/// Split a long message into chunks at newline boundaries.
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

        let safe_end = crate::util::floor_char_boundary(remaining, max_len);
        let split_at = remaining[..safe_end].rfind('\n').unwrap_or(safe_end);

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start_matches('\n');
    }

    chunks
}

/// Redact a URL for logging (hide password, port, etc.).
fn redact_url(url: &str) -> String {
    // Show scheme + host, hide everything else
    if let Some(idx) = url.find("://") {
        let after_scheme = &url[idx + 3..];
        if let Some(slash) = after_scheme.find('/') {
            return format!("{}://{}/***", &url[..idx], &after_scheme[..slash]);
        }
        return format!("{}://{}", &url[..idx], after_scheme);
    }
    "[redacted]".to_string()
}

/// Redact phone numbers and email addresses from text.
fn redact_pii(text: &str) -> String {
    use std::sync::OnceLock;
    static PHONE_RE: OnceLock<regex::Regex> = OnceLock::new();
    static EMAIL_RE: OnceLock<regex::Regex> = OnceLock::new();

    let phone = PHONE_RE.get_or_init(|| regex::Regex::new(r"\+?\d{7,15}").unwrap());
    let email = EMAIL_RE.get_or_init(|| regex::Regex::new(r"[\w.+-]+@[\w-]+\.[\w.]+").unwrap());
    let s = phone.replace_all(text, "[REDACTED]");
    email.replace_all(&s, "[REDACTED]").to_string()
}

/// Decode a base64-encoded string to bytes.
fn base64_decode(input: &str) -> Result<Vec<u8>, ChannelError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| ChannelError::SendFailed {
            name: NAME.to_string(),
            reason: format!("base64 decode error: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use futures::StreamExt;
    use secrecy::SecretString;
    use std::time::Duration;
    use tower::ServiceExt;
    use tokio::time::timeout;

    #[test]
    fn test_normalize_server_url() {
        assert_eq!(
            normalize_server_url("192.168.1.50:1234"),
            "http://192.168.1.50:1234"
        );
        assert_eq!(
            normalize_server_url("http://192.168.1.50:1234/"),
            "http://192.168.1.50:1234"
        );
        assert_eq!(
            normalize_server_url("https://my.tunnel.dev"),
            "https://my.tunnel.dev"
        );
        assert_eq!(normalize_server_url(""), "");
        assert_eq!(normalize_server_url("  "), "");
    }

    #[test]
    fn test_extract_record_from_data_object() {
        let payload = serde_json::json!({
            "type": "new-message",
            "data": { "text": "hello", "isFromMe": false }
        });
        let record = extract_record(&payload).unwrap();
        assert_eq!(record.get("text").unwrap().as_str(), Some("hello"));
    }

    #[test]
    fn test_extract_record_from_data_array() {
        let payload = serde_json::json!({
            "type": "new-message",
            "data": [{ "text": "hello" }]
        });
        let record = extract_record(&payload).unwrap();
        assert_eq!(record.get("text").unwrap().as_str(), Some("hello"));
    }

    #[test]
    fn test_first_string() {
        let a = serde_json::json!("");
        let b = serde_json::json!("hello");
        assert_eq!(first_string(&[Some(&a), Some(&b)]), Some("hello".into()));
        assert_eq!(first_string(&[None, None]), None);
    }

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("Hello!", 4000);
        assert_eq!(chunks, vec!["Hello!"]);
    }

    #[test]
    fn test_split_message_long() {
        let text = "a".repeat(5000);
        let chunks = split_message(&text, 4000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 4000);
    }

    #[tokio::test]
    async fn test_webhook_routes_forwards_allowed_inbound_message() {
        let config = BlueBubblesConfig {
            server_url: "http://127.0.0.1:1".to_string(),
            password: SecretString::from("bluebubbles-pass"),
            webhook_host: "127.0.0.1".to_string(),
            webhook_port: 8645,
            webhook_path: "/bluebubbles-webhook".to_string(),
            allow_from: vec!["alice@example.com".to_string()],
            send_read_receipts: false,
        };

        let channel = BlueBubblesChannel::new(config).await.expect("channel should initialize");
        let mut stream = channel.start().await.expect("stream should start");

        let app = channel.webhook_routes();
        let payload = serde_json::json!({
            "type": "new-message",
            "text": "hello from iMessage",
            "chatGuid": "chat-1",
            "isFromMe": false,
            "chatIdentifier": "chat-1",
            "handle": { "address": "alice@example.com" },
            "isGroup": false,
        });
        let request = Request::builder()
            .method("POST")
            .uri("/bluebubbles-webhook?password=bluebubbles-pass")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).expect("payload should serialize")))
            .expect("request should build");

        let response = app.oneshot(request).await.expect("webhook request should be served");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let message = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("message should arrive")
            .expect("stream should yield one message");
        assert_eq!(message.channel, "bluebubbles");
        assert_eq!(message.user_id, "alice@example.com");
        assert_eq!(message.content, "hello from iMessage");
        assert_eq!(message.metadata["chat_guid"], "chat-1");
        assert_eq!(message.metadata["conversation_kind"], "direct");
        assert_eq!(message.metadata["attachment_count"], 0);
    }

    #[tokio::test]
    async fn test_webhook_routes_rejects_unlisted_sender() {
        let config = BlueBubblesConfig {
            server_url: "http://127.0.0.1:1".to_string(),
            password: SecretString::from("bluebubbles-pass"),
            webhook_host: "127.0.0.1".to_string(),
            webhook_port: 8645,
            webhook_path: "/bluebubbles-webhook".to_string(),
            allow_from: vec!["trusted@example.com".to_string()],
            send_read_receipts: false,
        };

        let channel = BlueBubblesChannel::new(config).await.expect("channel should initialize");
        let mut stream = channel.start().await.expect("stream should start");
        let app = channel.webhook_routes();

        let payload = serde_json::json!({
            "type": "new-message",
            "text": "ignore this",
            "chatGuid": "chat-2",
            "isFromMe": false,
            "handle": { "address": "unknown@example.com" },
        });
        let request = Request::builder()
            .method("POST")
            .uri("/bluebubbles-webhook?guid=bluebubbles-pass")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).expect("payload should serialize")))
            .expect("request should build");

        let response = app.oneshot(request).await.expect("webhook request should be served");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        assert!(timeout(Duration::from_millis(200), stream.next())
            .await
            .is_err(), "unlisted sender should not emit a message");
    }

    #[test]
    fn test_redact_url() {
        assert_eq!(
            redact_url("http://192.168.1.50:1234/api/v1/ping"),
            "http://192.168.1.50:1234/***"
        );
        assert_eq!(
            redact_url("https://my.tunnel.dev"),
            "https://my.tunnel.dev"
        );
    }

    #[test]
    fn test_redact_pii() {
        assert_eq!(redact_pii("+4917612345678"), "[REDACTED]");
        assert_eq!(redact_pii("user@icloud.com"), "[REDACTED]");
        assert_eq!(redact_pii("hello"), "hello");
    }

    #[test]
    fn test_message_events_filter() {
        assert!(MESSAGE_EVENTS.contains(&"new-message"));
        assert!(MESSAGE_EVENTS.contains(&"updated-message"));
        assert!(!MESSAGE_EVENTS.contains(&"typing-indicator"));
    }

    #[test]
    fn test_tapback_codes_range() {
        // Added reactions: 2000-2005
        for code in 2000..=2005 {
            assert!(TAPBACK_CODES.contains(&code), "tapback code {code} should be present");
        }
        // Removed reactions: 3000-3005
        for code in 3000..=3005 {
            assert!(TAPBACK_CODES.contains(&code), "tapback code {code} should be present");
        }
        // Non-tapback codes should not match
        assert!(!TAPBACK_CODES.contains(&0));
        assert!(!TAPBACK_CODES.contains(&1999));
        assert!(!TAPBACK_CODES.contains(&2006));
    }

    #[tokio::test]
    async fn test_webhook_filters_tapback_reactions() {
        let config = BlueBubblesConfig {
            server_url: "http://127.0.0.1:1".to_string(),
            password: SecretString::from("bluebubbles-pass"),
            webhook_host: "127.0.0.1".to_string(),
            webhook_port: 8645,
            webhook_path: "/bluebubbles-webhook".to_string(),
            allow_from: vec![],
            send_read_receipts: false,
        };

        let channel = BlueBubblesChannel::new(config).await.expect("channel should initialize");
        let mut stream = channel.start().await.expect("stream should start");
        let app = channel.webhook_routes();

        // Send a tapback reaction (love = 2000)
        let payload = serde_json::json!({
            "type": "new-message",
            "data": {
                "text": "",
                "chatGuid": "chat-1",
                "isFromMe": false,
                "associatedMessageType": 2000,
                "handle": { "address": "alice@example.com" },
            }
        });
        let request = Request::builder()
            .method("POST")
            .uri("/bluebubbles-webhook?password=bluebubbles-pass")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).expect("serialize")))
            .expect("build request");

        let response = app.oneshot(request).await.expect("serve");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        // No message should arrive — tapbacks are filtered
        assert!(
            timeout(Duration::from_millis(200), stream.next()).await.is_err(),
            "tapback reaction should not produce a message"
        );
    }

    #[tokio::test]
    async fn test_webhook_form_encoded_fallback() {
        let config = BlueBubblesConfig {
            server_url: "http://127.0.0.1:1".to_string(),
            password: SecretString::from("bluebubbles-pass"),
            webhook_host: "127.0.0.1".to_string(),
            webhook_port: 8645,
            webhook_path: "/bluebubbles-webhook".to_string(),
            allow_from: vec![],
            send_read_receipts: false,
        };

        let channel = BlueBubblesChannel::new(config).await.expect("channel should initialize");
        let mut stream = channel.start().await.expect("stream should start");
        let app = channel.webhook_routes();

        // Send form-encoded body: payload=<url-encoded JSON>
        let json_str = serde_json::json!({
            "type": "new-message",
            "data": {
                "text": "form-encoded hello",
                "chatGuid": "chat-form",
                "isFromMe": false,
                "handle": { "address": "bob@example.com" },
            }
        }).to_string();
        let form_body = format!("payload={}", urlencoding::encode(&json_str));

        let request = Request::builder()
            .method("POST")
            .uri("/bluebubbles-webhook?password=bluebubbles-pass")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(form_body))
            .expect("build request");

        let response = app.oneshot(request).await.expect("serve");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let message = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("message should arrive")
            .expect("stream should yield a message");
        assert_eq!(message.channel, "bluebubbles");
        assert_eq!(message.content, "form-encoded hello");
        assert_eq!(message.user_id, "bob@example.com");
    }

    #[test]
    fn test_base64_decode_valid() {
        let result = base64_decode("SGVsbG8=").expect("should decode");
        assert_eq!(result, b"Hello");
    }

    #[test]
    fn test_base64_decode_invalid() {
        assert!(base64_decode("!!!invalid!!!").is_err());
    }

    #[tokio::test]
    async fn test_webhook_includes_reply_to_metadata() {
        let config = BlueBubblesConfig {
            server_url: "http://127.0.0.1:1".to_string(),
            password: SecretString::from("pass123"),
            webhook_host: "127.0.0.1".to_string(),
            webhook_port: 8645,
            webhook_path: "/bluebubbles-webhook".to_string(),
            allow_from: vec![],
            send_read_receipts: false,
        };

        let channel = BlueBubblesChannel::new(config).await.expect("channel should initialize");
        let mut stream = channel.start().await.expect("stream should start");
        let app = channel.webhook_routes();

        let payload = serde_json::json!({
            "type": "new-message",
            "data": {
                "text": "replying to something",
                "chatGuid": "chat-reply",
                "isFromMe": false,
                "handle": { "address": "carol@example.com" },
                "threadOriginatorGuid": "original-msg-guid-123",
                "guid": "this-msg-guid-456",
            }
        });
        let request = Request::builder()
            .method("POST")
            .uri("/bluebubbles-webhook?password=pass123")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).expect("serialize")))
            .expect("build request");

        let response = app.oneshot(request).await.expect("serve");
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        let message = timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("message should arrive")
            .expect("stream should yield");
        assert_eq!(message.metadata["message_id"], "this-msg-guid-456");
        assert_eq!(
            message.metadata["reply_to_message_id"],
            "original-msg-guid-123"
        );
    }
}
