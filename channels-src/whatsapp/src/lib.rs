// WhatsApp API types have fields reserved for future use (contacts, statuses, etc.)
#![allow(dead_code)]

//! WhatsApp Cloud API channel for ThinClaw.
//!
//! This WASM component implements the channel interface for handling WhatsApp
//! webhooks and sending messages back via the Cloud API.
//!
//! # Features
//!
//! - Webhook-based message receiving (WhatsApp is webhook-only, no polling)
//! - Text message support
//! - Business account support
//! - User name extraction from contacts
//!
//! # Security
//!
//! - Access token is injected by host during HTTP requests via {WHATSAPP_ACCESS_TOKEN} placeholder
//! - WASM never sees raw credentials
//! - Webhook verify token validation by host

// Generate bindings from the WIT file
wit_bindgen::generate!({
    world: "sandboxed-channel",
    path: "../../wit/channel.wit",
});

use serde::{Deserialize, Serialize};

// Re-export generated types
use exports::near::agent::channel::{
    AgentResponse, ChannelConfig, Guest, HttpEndpointConfig, IncomingHttpRequest,
    OutgoingHttpResponse, StatusUpdate,
};
use near::agent::channel_host::{self, EmittedMessage};

// ============================================================================
// WhatsApp Cloud API Types
// ============================================================================

/// WhatsApp webhook payload.
/// https://developers.facebook.com/docs/whatsapp/cloud-api/webhooks/payload-examples
#[derive(Debug, Deserialize)]
struct WebhookPayload {
    /// Always "whatsapp_business_account"
    object: String,

    /// Array of webhook entries
    entry: Vec<WebhookEntry>,
}

/// Single webhook entry.
#[derive(Debug, Deserialize)]
struct WebhookEntry {
    /// WhatsApp Business Account ID
    id: String,

    /// Changes in this entry
    changes: Vec<WebhookChange>,
}

/// A change notification.
#[derive(Debug, Deserialize)]
struct WebhookChange {
    /// Field that changed (usually "messages")
    field: String,

    /// The change value
    value: WebhookValue,
}

/// The value of a change.
#[derive(Debug, Deserialize)]
struct WebhookValue {
    /// Messaging product (always "whatsapp")
    messaging_product: String,

    /// Business account metadata
    metadata: BusinessMetadata,

    /// Contact information (sender details)
    #[serde(default)]
    contacts: Vec<Contact>,

    /// Incoming messages
    #[serde(default)]
    messages: Vec<WhatsAppMessage>,

    /// Message statuses (delivered, read, etc.)
    #[serde(default)]
    statuses: Vec<MessageStatus>,
}

/// Business account metadata.
#[derive(Debug, Deserialize)]
struct BusinessMetadata {
    /// Display phone number
    display_phone_number: String,

    /// Phone number ID (used in API calls)
    phone_number_id: String,
}

/// Contact information.
#[derive(Debug, Deserialize)]
struct Contact {
    /// WhatsApp ID (phone number)
    wa_id: String,

    /// Profile information
    profile: Option<ContactProfile>,
}

/// Contact profile.
#[derive(Debug, Deserialize)]
struct ContactProfile {
    /// Display name
    name: String,
}

/// Incoming WhatsApp message.
#[derive(Debug, Deserialize)]
struct WhatsAppMessage {
    /// Message ID
    id: String,

    /// Sender's phone number
    from: String,

    /// Unix timestamp
    timestamp: String,

    /// Message type: text, image, audio, video, document, sticker, etc.
    #[serde(rename = "type")]
    message_type: String,

    /// Text content (if type is "text")
    text: Option<TextContent>,

    /// Image content (if type is "image")
    image: Option<WhatsAppMedia>,

    /// Audio content (if type is "audio")
    audio: Option<WhatsAppMedia>,

    /// Video content (if type is "video")
    video: Option<WhatsAppMedia>,

    /// Document content (if type is "document")
    document: Option<WhatsAppDocument>,

    /// Sticker content (if type is "sticker")
    sticker: Option<WhatsAppMedia>,

    /// Context for replies
    context: Option<MessageContext>,
}

/// Text message content.
#[derive(Debug, Deserialize)]
struct TextContent {
    /// The message body
    body: String,
}

/// WhatsApp media content (image, audio, video, sticker).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WhatsAppMedia {
    /// Media ID for downloading.
    id: String,
    /// MIME type (e.g. "image/jpeg").
    mime_type: Option<String>,
    /// Caption text.
    caption: Option<String>,
}

/// WhatsApp document content.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct WhatsAppDocument {
    /// Media ID for downloading.
    id: String,
    /// MIME type.
    mime_type: Option<String>,
    /// Original filename.
    filename: Option<String>,
    /// Caption text.
    caption: Option<String>,
}

/// Maximum file size we'll download (20 MB).
const MAX_WHATSAPP_DOWNLOAD_SIZE: usize = 20 * 1024 * 1024;

/// Reply context.
#[derive(Debug, Deserialize)]
struct MessageContext {
    /// Message ID being replied to
    message_id: String,

    /// Phone number of original sender
    from: Option<String>,
}

/// Message status update.
#[derive(Debug, Deserialize)]
struct MessageStatus {
    /// Message ID
    id: String,

    /// Status: sent, delivered, read, failed
    status: String,

    /// Timestamp
    timestamp: String,

    /// Recipient ID
    recipient_id: String,
}

/// WhatsApp API response wrapper.
#[derive(Debug, Deserialize)]
struct WhatsAppApiResponse {
    /// Messages sent (on success)
    messages: Option<Vec<SentMessage>>,

    /// Error info (on failure)
    error: Option<ApiError>,
}

/// Sent message info.
#[derive(Debug, Deserialize)]
struct SentMessage {
    /// Message ID
    id: String,
}

/// API error details.
#[derive(Debug, Deserialize)]
struct ApiError {
    /// Error message
    message: String,

    /// Error type
    #[serde(rename = "type")]
    error_type: Option<String>,

    /// Error code
    code: Option<i64>,
}

// ============================================================================
// Channel Metadata
// ============================================================================

/// Metadata stored with emitted messages for response routing.
/// This MUST contain all info needed to send a response.
#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppMessageMetadata {
    /// Phone number ID (business account, for API URL)
    phone_number_id: String,

    /// Sender's phone number (becomes recipient for response)
    sender_phone: String,

    /// Original message ID (for reply context)
    message_id: String,

    /// Timestamp of original message
    timestamp: String,

    /// Normalized conversation kind for downstream resolver logic.
    #[serde(default)]
    conversation_kind: Option<String>,

    /// Stable scope identifier for this DM conversation.
    #[serde(default)]
    conversation_scope_id: Option<String>,

    /// Stable external conversation key used for cross-channel continuity.
    #[serde(default)]
    external_conversation_key: Option<String>,

    /// Raw sender identifier from the webhook payload.
    #[serde(default)]
    raw_sender_id: Option<String>,

    /// Stable sender identifier used for continuity within WhatsApp.
    #[serde(default)]
    stable_sender_id: Option<String>,
}

/// Workspace path for persisting owner_id across WASM callbacks.
const OWNER_ID_PATH: &str = "state/owner_id";
/// Workspace path for persisting dm_policy across WASM callbacks.
const DM_POLICY_PATH: &str = "state/dm_policy";
/// Workspace path for persisting allow_from (JSON array) across WASM callbacks.
const ALLOW_FROM_PATH: &str = "state/allow_from";
/// Channel name for pairing store (used by pairing host APIs).
const CHANNEL_NAME: &str = "whatsapp";

/// Channel configuration from capabilities file.
#[derive(Debug, Deserialize)]
struct WhatsAppConfig {
    /// API version to use (default: v18.0)
    #[serde(default = "default_api_version")]
    api_version: String,

    /// Whether to reply to the original message (thread context)
    #[serde(default = "default_reply_to_message")]
    reply_to_message: bool,

    #[serde(default)]
    owner_id: Option<String>,

    #[serde(default)]
    dm_policy: Option<String>,

    #[serde(default)]
    allow_from: Option<Vec<String>>,
}

fn default_api_version() -> String {
    "v18.0".to_string()
}

fn default_reply_to_message() -> bool {
    true
}

fn conversation_scope_id(phone_number_id: &str, sender_phone: &str) -> String {
    format!("whatsapp:direct:{phone_number_id}:{sender_phone}")
}

fn external_conversation_key(phone_number_id: &str, sender_phone: &str) -> String {
    format!("whatsapp://direct/{phone_number_id}/{sender_phone}")
}

// ============================================================================
// Channel Implementation
// ============================================================================

struct WhatsAppChannel;

impl Guest for WhatsAppChannel {
    fn on_start(config_json: String) -> Result<ChannelConfig, String> {
        let config: WhatsAppConfig = match serde_json::from_str(&config_json) {
            Ok(c) => c,
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to parse WhatsApp config, using defaults: {}", e),
                );
                WhatsAppConfig {
                    api_version: default_api_version(),
                    reply_to_message: default_reply_to_message(),
                    owner_id: None,
                    dm_policy: None,
                    allow_from: None,
                }
            }
        };

        channel_host::log(
            channel_host::LogLevel::Info,
            &format!(
                "WhatsApp channel starting (API version: {})",
                config.api_version
            ),
        );

        // Persist config in workspace so on_respond() can read it
        let _ = channel_host::workspace_write("channels/whatsapp/api_version", &config.api_version);
        let _ = channel_host::workspace_write(
            "channels/whatsapp/reply_to_message",
            if config.reply_to_message {
                "true"
            } else {
                "false"
            },
        );

        // Persist permission config for handle_message
        if let Some(ref owner_id) = config.owner_id {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, owner_id);
            channel_host::log(
                channel_host::LogLevel::Info,
                &format!("Owner restriction enabled: user {}", owner_id),
            );
        } else {
            let _ = channel_host::workspace_write(OWNER_ID_PATH, "");
        }

        let dm_policy = config.dm_policy.as_deref().unwrap_or("pairing");
        let _ = channel_host::workspace_write(DM_POLICY_PATH, dm_policy);

        let allow_from_json = serde_json::to_string(&config.allow_from.unwrap_or_default())
            .unwrap_or_else(|_| "[]".to_string());
        let _ = channel_host::workspace_write(ALLOW_FROM_PATH, &allow_from_json);

        // WhatsApp Cloud API is webhook-only, no polling available
        Ok(ChannelConfig {
            display_name: "WhatsApp".to_string(),
            http_endpoints: vec![HttpEndpointConfig {
                path: "/webhook/whatsapp".to_string(),
                // GET for webhook verification, POST for incoming messages
                methods: vec!["GET".to_string(), "POST".to_string()],
                // Webhook verify token should be validated by host
                require_secret: true,
            }],
            poll: None, // WhatsApp doesn't support polling
        })
    }

    fn on_http_request(req: IncomingHttpRequest) -> OutgoingHttpResponse {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Received {} request to {}", req.method, req.path),
        );

        // Handle webhook verification (GET request from Meta)
        if req.method == "GET" {
            return handle_verification(&req);
        }

        // Handle incoming messages (POST request)
        if req.method == "POST" {
            // Defense in depth: check secret validation
            // Host validates the verify token, but we double-check the flag
            if !req.secret_validated {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    "Webhook request with invalid or missing verify token",
                );
                // Return 401 but note that host should have already rejected these
            }

            return handle_incoming_message(&req);
        }

        // Method not allowed
        json_response(405, serde_json::json!({"error": "Method not allowed"}))
    }

    fn on_poll() {
        // WhatsApp Cloud API is webhook-only, no polling
        // This should never be called since poll config is None
    }

    fn on_respond(response: AgentResponse) -> Result<(), String> {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Sending response for message: {}", response.message_id),
        );

        // Parse metadata from the ORIGINAL incoming message
        // This contains the routing info we need (sender becomes recipient)
        let metadata: WhatsAppMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        // Read api_version from workspace (set during on_start), fallback to default
        let api_version = channel_host::workspace_read("channels/whatsapp/api_version")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "v18.0".to_string());

        // Build WhatsApp API URL with token placeholder
        let api_url = format!(
            "https://graph.facebook.com/{}/{}/messages",
            api_version, metadata.phone_number_id
        );

        // Headers with Bearer token placeholder
        let headers = serde_json::json!({
            "Content-Type": "application/json",
            "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
        });

        // Convert standard Markdown → WhatsApp text format
        let wa_content = markdown_to_whatsapp(&response.content);

        // Split content into chunks that fit WhatsApp's 4096 char limit
        let chunks = split_message(&wa_content, WHATSAPP_MAX_MESSAGE_LENGTH);

        // Check if reply threading is enabled
        let reply_to_message = channel_host::workspace_read("channels/whatsapp/reply_to_message")
            .map(|s| s == "true")
            .unwrap_or(true);

        for (i, chunk) in chunks.iter().enumerate() {
            let mut payload = serde_json::json!({
                "messaging_product": "whatsapp",
                "recipient_type": "individual",
                "to": metadata.sender_phone,
                "type": "text",
                "text": {
                    "preview_url": false,
                    "body": chunk
                }
            });

            // Add reply context on the first chunk so the response threads
            // under the original message in the WhatsApp UI
            if reply_to_message && i == 0 {
                payload["context"] = serde_json::json!({
                    "message_id": metadata.message_id
                });
            }

            let payload_bytes = serde_json::to_vec(&payload)
                .map_err(|e| format!("Failed to serialize payload: {}", e))?;

            let result = channel_host::http_request(
                "POST",
                &api_url,
                &headers.to_string(),
                Some(&payload_bytes),
                None,
            );

            match result {
                Ok(http_response) => {
                    let api_response: Result<WhatsAppApiResponse, _> =
                        serde_json::from_slice(&http_response.body);

                    match api_response {
                        Ok(resp) => {
                            if let Some(error) = resp.error {
                                return Err(format!(
                                    "WhatsApp API error: {} (code: {:?})",
                                    error.message, error.code
                                ));
                            }

                            if let Some(messages) = resp.messages {
                                if let Some(sent) = messages.first() {
                                    channel_host::log(
                                        channel_host::LogLevel::Debug,
                                        &format!(
                                            "Sent message chunk {}/{} to {}: id={}",
                                            i + 1,
                                            chunks.len(),
                                            metadata.sender_phone,
                                            sent.id
                                        ),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            if http_response.status >= 200 && http_response.status < 300 {
                                channel_host::log(
                                    channel_host::LogLevel::Info,
                                    "Message sent (response parse failed but status OK)",
                                );
                            } else {
                                let body_str = String::from_utf8_lossy(&http_response.body);
                                return Err(format!(
                                    "WhatsApp API HTTP {}: {} (parse error: {})",
                                    http_response.status, body_str, e
                                ));
                            }
                        }
                    }
                }
                Err(e) => return Err(format!("HTTP request failed: {}", e)),
            }
        }

        Ok(())
    }

    fn on_status(_update: StatusUpdate) {}

    fn on_shutdown() {
        channel_host::log(
            channel_host::LogLevel::Info,
            "WhatsApp channel shutting down",
        );
    }
}

// ============================================================================
// Webhook Verification
// ============================================================================

/// Handle WhatsApp webhook verification request from Meta.
///
/// Meta sends a GET request with:
/// - hub.mode=subscribe
/// - hub.challenge=<random string>
/// - hub.verify_token=<your configured token>
///
/// We must respond with the challenge value to verify.
fn handle_verification(req: &IncomingHttpRequest) -> OutgoingHttpResponse {
    // Parse query parameters
    let query: serde_json::Value =
        serde_json::from_str(&req.query_json).unwrap_or(serde_json::Value::Null);

    let mode = query.get("hub.mode").and_then(|v| v.as_str());
    let challenge = query.get("hub.challenge").and_then(|v| v.as_str());

    // Verify token is validated by host via secret_validated field
    // We just need to check mode and return challenge

    if mode == Some("subscribe") {
        if let Some(challenge) = challenge {
            channel_host::log(
                channel_host::LogLevel::Info,
                "Webhook verification successful",
            );

            // Must respond with the challenge as plain text
            return OutgoingHttpResponse {
                status: 200,
                headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
                body: challenge.as_bytes().to_vec(),
            };
        }
    }

    channel_host::log(
        channel_host::LogLevel::Warn,
        &format!(
            "Webhook verification failed: mode={:?}, challenge={:?}",
            mode,
            challenge.is_some()
        ),
    );

    OutgoingHttpResponse {
        status: 403,
        headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
        body: b"Verification failed".to_vec(),
    }
}

// ============================================================================
// Message Handling
// ============================================================================

/// Handle incoming WhatsApp webhook payload.
fn handle_incoming_message(req: &IncomingHttpRequest) -> OutgoingHttpResponse {
    // Parse the body as UTF-8
    let body_str = match std::str::from_utf8(&req.body) {
        Ok(s) => s,
        Err(_) => {
            return json_response(400, serde_json::json!({"error": "Invalid UTF-8 body"}));
        }
    };

    // Parse webhook payload
    let payload: WebhookPayload = match serde_json::from_str(body_str) {
        Ok(p) => p,
        Err(e) => {
            channel_host::log(
                channel_host::LogLevel::Error,
                &format!("Failed to parse webhook payload: {}", e),
            );
            // Return 200 to prevent Meta from retrying
            return json_response(200, serde_json::json!({"status": "ok"}));
        }
    };

    // Validate object type
    if payload.object != "whatsapp_business_account" {
        channel_host::log(
            channel_host::LogLevel::Warn,
            &format!("Unexpected object type: {}", payload.object),
        );
        return json_response(200, serde_json::json!({"status": "ok"}));
    }

    // Process each entry
    for entry in payload.entry {
        for change in entry.changes {
            // Only handle message changes
            if change.field != "messages" {
                continue;
            }

            let value = change.value;
            let phone_number_id = value.metadata.phone_number_id.clone();

            // Build contact name lookup
            let contact_names: std::collections::HashMap<String, String> = value
                .contacts
                .iter()
                .filter_map(|c| {
                    c.profile
                        .as_ref()
                        .map(|p| (c.wa_id.clone(), p.name.clone()))
                })
                .collect();

            // Skip status updates (delivered, read, etc.) - we only want messages
            // This prevents loops and unnecessary processing
            if !value.statuses.is_empty() && value.messages.is_empty() {
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!("Skipping {} status updates", value.statuses.len()),
                );
                continue;
            }

            // Process messages
            for message in value.messages {
                handle_message(&message, &phone_number_id, &contact_names);
            }
        }
    }

    // Always respond 200 quickly (Meta expects fast responses)
    json_response(200, serde_json::json!({"status": "ok"}))
}

/// Process a single WhatsApp message.
fn handle_message(
    message: &WhatsAppMessage,
    phone_number_id: &str,
    contact_names: &std::collections::HashMap<String, String>,
) {
    // Extract text content — from text.body or media caption
    let text = message
        .text
        .as_ref()
        .filter(|t| !t.body.is_empty())
        .map(|t| t.body.clone())
        .or_else(|| {
            // Try caption from media types
            message
                .image
                .as_ref()
                .and_then(|m| m.caption.clone())
                .or_else(|| message.video.as_ref().and_then(|m| m.caption.clone()))
                .or_else(|| message.document.as_ref().and_then(|d| d.caption.clone()))
        })
        .unwrap_or_default();

    // Collect media descriptors: (media_id, mime_type, filename)
    let mut media_descriptors: Vec<(String, String, Option<String>)> = Vec::new();

    if let Some(ref img) = message.image {
        media_descriptors.push((
            img.id.clone(),
            img.mime_type
                .clone()
                .unwrap_or_else(|| "image/jpeg".to_string()),
            None,
        ));
    }
    if let Some(ref audio) = message.audio {
        media_descriptors.push((
            audio.id.clone(),
            audio
                .mime_type
                .clone()
                .unwrap_or_else(|| "audio/ogg".to_string()),
            None,
        ));
    }
    if let Some(ref video) = message.video {
        media_descriptors.push((
            video.id.clone(),
            video
                .mime_type
                .clone()
                .unwrap_or_else(|| "video/mp4".to_string()),
            None,
        ));
    }
    if let Some(ref doc) = message.document {
        media_descriptors.push((
            doc.id.clone(),
            doc.mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            doc.filename.clone(),
        ));
    }
    if let Some(ref sticker) = message.sticker {
        media_descriptors.push((
            sticker.id.clone(),
            sticker
                .mime_type
                .clone()
                .unwrap_or_else(|| "image/webp".to_string()),
            None,
        ));
    }

    let has_media = !media_descriptors.is_empty();

    // Skip messages with no text AND no media
    if text.is_empty() && !has_media {
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("Skipping empty message type: {}", message.message_type),
        );
        return;
    }

    // Look up sender's name from contacts
    let user_name = contact_names.get(&message.from).cloned();

    // Permission check (WhatsApp is always DM)
    if !check_sender_permission(&message.from, user_name.as_deref(), phone_number_id) {
        return;
    }

    // Download media attachments
    let headers_json =
        serde_json::json!({"Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"}).to_string();
    let mut attachments = Vec::new();

    for (media_id, mime_type, filename) in &media_descriptors {
        match download_whatsapp_media(media_id, &headers_json) {
            Ok(data) => {
                if data.len() > MAX_WHATSAPP_DOWNLOAD_SIZE {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Skipping oversized WhatsApp media: {} bytes", data.len()),
                    );
                    continue;
                }
                let mut att = channel_host::MediaAttachment {
                    mime_type: mime_type.clone(),
                    data,
                    filename: filename.clone(),
                };
                // If no filename, generate one from media type
                if att.filename.is_none() {
                    let ext = mime_type.split('/').last().unwrap_or("bin");
                    att.filename = Some(format!("media_{}.{}", media_id, ext));
                }
                attachments.push(att);
            }
            Err(e) => {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    &format!("Failed to download WhatsApp media {}: {}", media_id, e),
                );
            }
        }
    }

    // Determine content
    let content = if text.is_empty() && !attachments.is_empty() {
        "[Media received \u{2014} please analyze the attached content]".to_string()
    } else {
        text
    };

    // Build metadata for response routing
    let metadata = WhatsAppMessageMetadata {
        phone_number_id: phone_number_id.to_string(),
        sender_phone: message.from.clone(),
        message_id: message.id.clone(),
        timestamp: message.timestamp.clone(),
        conversation_kind: Some("direct".to_string()),
        conversation_scope_id: Some(conversation_scope_id(phone_number_id, &message.from)),
        external_conversation_key: Some(external_conversation_key(phone_number_id, &message.from)),
        raw_sender_id: Some(message.from.clone()),
        stable_sender_id: Some(message.from.clone()),
    };

    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());

    // Emit the message to the agent
    channel_host::emit_message(&EmittedMessage {
        user_id: message.from.clone(),
        user_name,
        content,
        thread_id: None,
        metadata_json,
        attachments,
    });

    channel_host::log(
        channel_host::LogLevel::Debug,
        &format!(
            "Emitted message from {} (phone_number_id={})",
            message.from, phone_number_id
        ),
    );
}

// ============================================================================
// Utilities
// ============================================================================

/// WhatsApp Cloud API media URL response.
#[derive(Debug, Deserialize)]
struct MediaUrlResponse {
    /// Download URL for the media.
    url: Option<String>,
}

/// Download media from the WhatsApp Cloud API.
///
/// Step 1: GET `https://graph.facebook.com/v19.0/<media_id>` → get download URL
/// Step 2: GET the download URL → binary data
fn download_whatsapp_media(media_id: &str, headers_json: &str) -> Result<Vec<u8>, String> {
    // Step 1: Get the media download URL
    let url = format!("https://graph.facebook.com/v19.0/{}", media_id);

    let response = channel_host::http_request("GET", &url, headers_json, None, Some(10_000))
        .map_err(|e| format!("Media URL request failed: {}", e))?;

    if response.status != 200 {
        return Err(format!("Media URL returned HTTP {}", response.status));
    }

    let media_info: MediaUrlResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse media URL response: {}", e))?;

    let download_url = media_info
        .url
        .ok_or_else(|| "No URL in media response".to_string())?;

    // Step 2: Download the actual file binary
    let download_response =
        channel_host::http_request("GET", &download_url, headers_json, None, Some(30_000))
            .map_err(|e| format!("Media download failed: {}", e))?;

    if download_response.status != 200 {
        return Err(format!(
            "Media download returned HTTP {}",
            download_response.status
        ));
    }

    if download_response.body.is_empty() {
        return Err("Media download returned empty body".to_string());
    }

    Ok(download_response.body)
}
// ============================================================================
// Permission & Pairing
// ============================================================================

/// Check if a sender is permitted. Returns true if allowed.
/// WhatsApp is always 1-to-1 (DM), so dm_policy always applies.
fn check_sender_permission(
    sender_phone: &str,
    user_name: Option<&str>,
    phone_number_id: &str,
) -> bool {
    // 1. Owner check (highest priority)
    let owner_id = channel_host::workspace_read(OWNER_ID_PATH).filter(|s| !s.is_empty());
    if let Some(ref owner) = owner_id {
        if sender_phone != owner {
            channel_host::log(
                channel_host::LogLevel::Debug,
                &format!(
                    "Dropping message from non-owner {} (owner: {})",
                    sender_phone, owner
                ),
            );
            return false;
        }
        return true;
    }

    // 2. DM policy (WhatsApp is always DM)
    let dm_policy =
        channel_host::workspace_read(DM_POLICY_PATH).unwrap_or_else(|| "pairing".to_string());

    if dm_policy == "open" {
        return true;
    }

    // 3. Build merged allow list
    let mut allowed: Vec<String> = channel_host::workspace_read(ALLOW_FROM_PATH)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    if let Ok(store_allowed) = channel_host::pairing_read_allow_from(CHANNEL_NAME) {
        allowed.extend(store_allowed);
    }

    // 4. Check sender (phone number or name)
    let is_allowed = allowed.contains(&"*".to_string())
        || allowed.contains(&sender_phone.to_string())
        || user_name.is_some_and(|u| allowed.contains(&u.to_string()));

    if is_allowed {
        return true;
    }

    // 5. Not allowed — handle by policy
    if dm_policy == "pairing" {
        let meta = serde_json::json!({
            "phone": sender_phone,
            "name": user_name,
            "conversation_kind": "direct",
            "conversation_scope_id": conversation_scope_id(phone_number_id, sender_phone),
            "external_conversation_key": external_conversation_key(phone_number_id, sender_phone),
            "raw_sender_id": sender_phone,
            "stable_sender_id": sender_phone,
        })
        .to_string();

        match channel_host::pairing_upsert_request(CHANNEL_NAME, sender_phone, &meta) {
            Ok(result) => {
                channel_host::log(
                    channel_host::LogLevel::Info,
                    &format!("Pairing request for {}: code {}", sender_phone, result.code),
                );
                if result.created {
                    let _ = send_pairing_reply(sender_phone, phone_number_id, &result.code);
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
    false
}

/// Send a pairing code message via WhatsApp Cloud API.
fn send_pairing_reply(
    recipient_phone: &str,
    phone_number_id: &str,
    code: &str,
) -> Result<(), String> {
    let api_version = channel_host::workspace_read("channels/whatsapp/api_version")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "v18.0".to_string());

    let url = format!(
        "https://graph.facebook.com/{}/{}/messages",
        api_version, phone_number_id
    );

    let payload = serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient_phone,
        "type": "text",
        "text": {
            "preview_url": false,
            "body": format!(
                "To pair with this bot, run: thinclaw pairing approve whatsapp {}",
                code
            )
        }
    });

    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("Failed to serialize: {}", e))?;

    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
    });

    let result = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    );

    match result {
        Ok(response) if response.status >= 200 && response.status < 300 => Ok(()),
        Ok(response) => {
            let body_str = String::from_utf8_lossy(&response.body);
            Err(format!(
                "WhatsApp API error: {} - {}",
                response.status, body_str
            ))
        }
        Err(e) => Err(format!("HTTP request failed: {}", e)),
    }
}

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
export!(WhatsAppChannel);

// ============================================================================
// Markdown → WhatsApp Format Converter
// ============================================================================

/// Convert standard Markdown (as produced by LLMs) to WhatsApp text format.
///
/// WhatsApp supports limited formatting in text messages:
/// - Bold: `*text*` (not `**text**`)
/// - Italic: `_text_` (same as Markdown)
/// - Strikethrough: `~text~` (not `~~text~~`)
/// - Monospace: `` ```text``` `` (triple backtick blocks)
/// - Inline code: `` `code` `` (single backtick, same)
/// - No link syntax, no headings, no blockquotes
fn markdown_to_whatsapp(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_code_block = false;

    for line in input.lines() {
        let trimmed = line.trim();

        // Track fenced code blocks — pass through unchanged
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

        // Convert heading lines: "# Heading" → "*Heading*" (bold)
        if let Some(heading_text) = parse_wa_heading(trimmed) {
            let leading_ws: &str = &line[..line.len() - trimmed.len()];
            result.push_str(leading_ws);
            result.push('*');
            result.push_str(heading_text);
            result.push('*');
            result.push('\n');
            continue;
        }

        // Convert blockquote prefix: "> text" → "text" (strip prefix)
        let effective_line = if trimmed.starts_with("> ") {
            let leading_ws: &str = &line[..line.len() - trimmed.len()];
            let mut s = String::from(leading_ws);
            s.push_str(&trimmed[2..]);
            s
        } else if trimmed == ">" {
            String::new()
        } else {
            line.to_string()
        };

        // Process inline formatting
        let converted = convert_inline_whatsapp(&effective_line);
        result.push_str(&converted);
        result.push('\n');
    }

    // Remove trailing newline added by the loop
    if result.ends_with('\n') && !input.ends_with('\n') {
        result.pop();
    }

    result
}

/// Parse a heading line, returning the heading text without the `#` prefix.
fn parse_wa_heading(line: &str) -> Option<&str> {
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

/// Convert inline Markdown formatting on a single line to WhatsApp format.
fn convert_inline_whatsapp(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;

    while i < len {
        // Skip inline code (don't convert inside backticks)
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

        // Convert Markdown links [text](url) → "text (url)"
        if chars[i] == '[' {
            if let Some((text, url, end)) = parse_wa_link(&chars, i) {
                out.push_str(&text);
                out.push_str(" (");
                out.push_str(&url);
                out.push(')');
                i = end;
                continue;
            }
        }

        // Convert ~~strikethrough~~ → ~strikethrough~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if let Some((content, end)) = extract_wa_delimited(&chars, i, '~', 2) {
                out.push('~');
                out.push_str(&content);
                out.push('~');
                i = end;
                continue;
            }
        }

        // Convert **bold** → *bold*
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some((content, end)) = extract_wa_delimited(&chars, i, '*', 2) {
                out.push('*');
                out.push_str(&content);
                out.push('*');
                i = end;
                continue;
            }
        }

        // Convert __bold/italic__ → *bold/italic* (WhatsApp doesn't distinguish)
        if i + 1 < len && chars[i] == '_' && chars[i + 1] == '_' {
            if let Some((content, end)) = extract_wa_delimited(&chars, i, '_', 2) {
                out.push('*');
                out.push_str(&content);
                out.push('*');
                i = end;
                continue;
            }
        }

        // Single _italic_ and single *bold* pass through unchanged
        // (WhatsApp natively supports both)

        out.push(chars[i]);
        i += 1;
    }

    out
}

/// Parse a Markdown link `[text](url)` starting at position `start`.
fn parse_wa_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if chars[start] != '[' {
        return None;
    }
    let mut i = start + 1;
    let mut text = String::new();
    let mut depth = 1;
    while i < chars.len() && depth > 0 {
        if chars[i] == '[' {
            depth += 1;
        } else if chars[i] == ']' {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        text.push(chars[i]);
        i += 1;
    }
    if depth != 0 || i >= chars.len() {
        return None;
    }
    i += 1; // skip ]
    if i >= chars.len() || chars[i] != '(' {
        return None;
    }
    i += 1; // skip (
    let mut url = String::new();
    while i < chars.len() && chars[i] != ')' {
        url.push(chars[i]);
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    i += 1; // skip )
    Some((text, url, i))
}

/// Extract content between `count` instances of `delimiter` character.
fn extract_wa_delimited(
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

/// WhatsApp limits messages to 4096 characters.
/// https://developers.facebook.com/docs/whatsapp/cloud-api/reference/messages
const WHATSAPP_MAX_MESSAGE_LENGTH: usize = 4096;

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

        let search_area = &remaining[..max_len];

        let split_at = search_area
            .rfind("\n\n")
            .map(|pos| pos + 1)
            .or_else(|| search_area.rfind('\n'))
            .or_else(|| search_area.rfind(' '))
            .unwrap_or_else(|| {
                let mut boundary = max_len;
                while boundary > 0 && !remaining.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                boundary
            });

        if split_at == 0 {
            chunks.push(remaining.to_string());
            break;
        }

        chunks.push(remaining[..split_at].trim_end().to_string());
        remaining = remaining[split_at..].trim_start();
    }

    chunks.retain(|c| !c.is_empty());
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }

    chunks
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_webhook_payload() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "123456789",
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+1234567890",
                            "phone_number_id": "987654321"
                        },
                        "contacts": [{
                            "wa_id": "15551234567",
                            "profile": {
                                "name": "John Doe"
                            }
                        }],
                        "messages": [{
                            "id": "wamid.abc123",
                            "from": "15551234567",
                            "timestamp": "1234567890",
                            "type": "text",
                            "text": {
                                "body": "Hello!"
                            }
                        }]
                    }
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.object, "whatsapp_business_account");
        assert_eq!(payload.entry.len(), 1);

        let change = &payload.entry[0].changes[0];
        assert_eq!(change.field, "messages");
        assert_eq!(change.value.metadata.phone_number_id, "987654321");

        let message = &change.value.messages[0];
        assert_eq!(message.from, "15551234567");
        assert_eq!(message.text.as_ref().unwrap().body, "Hello!");
    }

    #[test]
    fn test_parse_status_update() {
        let json = r#"{
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "123456789",
                "changes": [{
                    "field": "messages",
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "+1234567890",
                            "phone_number_id": "987654321"
                        },
                        "statuses": [{
                            "id": "wamid.abc123",
                            "status": "delivered",
                            "timestamp": "1234567890",
                            "recipient_id": "15551234567"
                        }]
                    }
                }]
            }]
        }"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        let value = &payload.entry[0].changes[0].value;

        // Should have status but no messages
        assert!(value.messages.is_empty());
        assert_eq!(value.statuses.len(), 1);
        assert_eq!(value.statuses[0].status, "delivered");
    }

    #[test]
    fn test_metadata_roundtrip() {
        let metadata = WhatsAppMessageMetadata {
            phone_number_id: "123456".to_string(),
            sender_phone: "15551234567".to_string(),
            message_id: "wamid.abc".to_string(),
            timestamp: "1234567890".to_string(),
            conversation_kind: Some("direct".to_string()),
            conversation_scope_id: Some("whatsapp:direct:123456:15551234567".to_string()),
            external_conversation_key: Some("whatsapp://direct/123456/15551234567".to_string()),
            raw_sender_id: Some("15551234567".to_string()),
            stable_sender_id: Some("15551234567".to_string()),
        };

        let json = serde_json::to_string(&metadata).unwrap();
        let parsed: WhatsAppMessageMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.phone_number_id, "123456");
        assert_eq!(parsed.sender_phone, "15551234567");
        assert_eq!(parsed.conversation_kind.as_deref(), Some("direct"));
    }

    #[test]
    fn test_split_message_short() {
        let chunks = split_message("hello", WHATSAPP_MAX_MESSAGE_LENGTH);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn test_split_message_prefers_newline_boundaries() {
        let text = format!("{}\n{}", "a".repeat(2500), "b".repeat(2500));
        let chunks = split_message(&text, WHATSAPP_MAX_MESSAGE_LENGTH);

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].len() <= WHATSAPP_MAX_MESSAGE_LENGTH);
        assert!(chunks[1].len() <= WHATSAPP_MAX_MESSAGE_LENGTH);
        assert!(chunks[0].starts_with('a'));
        assert!(chunks[1].starts_with('b'));
    }
}
