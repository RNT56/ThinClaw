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

    /// Location content (if type is "location")
    location: Option<WhatsAppLocation>,

    /// Contact cards (if type is "contacts")
    contacts: Option<Vec<WhatsAppContactCard>>,

    /// Interactive reply content (if type is "interactive")
    interactive: Option<WhatsAppInteractive>,

    /// Reaction content (if type is "reaction")
    reaction: Option<WhatsAppReaction>,

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

/// Location payload.
#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppLocation {
    latitude: f64,
    longitude: f64,
    name: Option<String>,
    address: Option<String>,
    url: Option<String>,
}

/// Contact-card payload.
#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppContactCard {
    name: Option<WhatsAppContactCardName>,
    #[serde(default)]
    phones: Vec<WhatsAppContactPhone>,
    #[serde(default)]
    emails: Vec<WhatsAppContactEmail>,
    org: Option<WhatsAppContactOrg>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppContactCardName {
    formatted_name: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppContactPhone {
    phone: Option<String>,
    wa_id: Option<String>,
    r#type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppContactEmail {
    email: Option<String>,
    r#type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppContactOrg {
    company: Option<String>,
    department: Option<String>,
    title: Option<String>,
}

/// Interactive payload.
#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppInteractive {
    #[serde(rename = "type")]
    interactive_type: String,
    button_reply: Option<WhatsAppInteractiveButtonReply>,
    list_reply: Option<WhatsAppInteractiveListReply>,
    nfm_reply: Option<WhatsAppInteractiveFlowReply>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppInteractiveButtonReply {
    id: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppInteractiveListReply {
    id: Option<String>,
    title: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppInteractiveFlowReply {
    name: Option<String>,
    body: Option<String>,
    response_json: Option<String>,
}

/// Reaction payload.
#[derive(Debug, Serialize, Deserialize)]
struct WhatsAppReaction {
    message_id: Option<String>,
    emoji: Option<String>,
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

/// Media upload response.
#[derive(Debug, Deserialize)]
struct MediaUploadResponse {
    id: String,
}

/// Common Graph API error envelope.
#[derive(Debug, Deserialize)]
struct GraphApiErrorResponse {
    error: ApiError,
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

    /// Sender's phone number (becomes recipient for response).
    ///
    /// Also accepts `recipient_phone` for proactive sends.
    #[serde(alias = "recipient_phone")]
    sender_phone: String,

    /// Original message ID (for reply context).
    #[serde(default)]
    message_id: Option<String>,

    /// Explicit reply target for proactive sends.
    #[serde(default)]
    reply_to_message_id: Option<String>,

    /// Timestamp of original message
    #[serde(default)]
    timestamp: Option<String>,

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

    /// Original inbound WhatsApp message type.
    #[serde(default)]
    inbound_message_type: Option<String>,

    /// ID of the message this inbound event referenced, when present.
    #[serde(default)]
    context_message_id: Option<String>,

    /// Structured inbound event details for downstream consumers.
    #[serde(default)]
    event_details: Option<serde_json::Value>,

    /// Structured outbound attachments supplied by the host response bridge.
    #[serde(default)]
    response_attachments: Vec<ResponseAttachmentEnvelope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ResponseAttachmentEnvelope {
    mime_type: String,
    #[serde(default)]
    filename: Option<String>,
    data: String,
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
            if !req.secret_validated {
                channel_host::log(
                    channel_host::LogLevel::Warn,
                    "Webhook request with invalid or missing signature",
                );
                return json_response(
                    401,
                    serde_json::json!({"error": "Invalid webhook signature"}),
                );
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

        let metadata: WhatsAppMessageMetadata = serde_json::from_str(&response.metadata_json)
            .map_err(|e| format!("Failed to parse metadata: {}", e))?;

        let api_version = current_api_version();
        let reply_to_message_id = if reply_to_message_enabled() {
            metadata
                .reply_to_message_id
                .clone()
                .or_else(|| metadata.message_id.clone())
        } else {
            None
        };

        let outbound_attachments = decode_response_attachments(&metadata.response_attachments);
        let mut delivered_any = false;
        let mut attachment_failures = Vec::new();

        for attachment in outbound_attachments {
            match send_outbound_attachment(
                &api_version,
                &metadata.phone_number_id,
                &metadata.sender_phone,
                reply_to_message_id.as_deref(),
                &attachment,
            ) {
                Ok(sent_id) => {
                    delivered_any = true;
                    channel_host::log(
                        channel_host::LogLevel::Debug,
                        &format!(
                            "Sent WhatsApp attachment to {}: id={}",
                            metadata.sender_phone, sent_id
                        ),
                    );
                }
                Err(error) => {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!("Failed to send WhatsApp attachment: {}", error),
                    );
                    attachment_failures.push(error);
                }
            }
        }

        let wa_content = markdown_to_whatsapp(&response.content);
        let has_text = !wa_content.trim().is_empty();

        if has_text {
            let chunks = split_message(&wa_content, WHATSAPP_MAX_MESSAGE_LENGTH);

            for (i, chunk) in chunks.iter().enumerate() {
                let sent_id = send_text_message(
                    &api_version,
                    &metadata.phone_number_id,
                    &metadata.sender_phone,
                    reply_to_message_id.as_deref(),
                    chunk,
                )?;
                delivered_any = true;
                channel_host::log(
                    channel_host::LogLevel::Debug,
                    &format!(
                        "Sent WhatsApp message chunk {}/{} to {}: id={}",
                        i + 1,
                        chunks.len(),
                        metadata.sender_phone,
                        sent_id
                    ),
                );
            }
        }

        if delivered_any {
            Ok(())
        } else if has_text {
            Err("No WhatsApp content was delivered".to_string())
        } else if !attachment_failures.is_empty() {
            Err(format!(
                "No WhatsApp attachments were delivered: {}",
                attachment_failures.join("; ")
            ))
        } else {
            Err("No deliverable WhatsApp response content".to_string())
        }
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

mod handlers;
pub(crate) use handlers::*;

#[cfg(test)]
mod tests;

export!(WhatsAppChannel);
