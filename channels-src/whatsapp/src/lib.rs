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

/// Handle WhatsApp webhook verification request from Meta.
///
/// Meta sends a GET request with:
/// - hub.mode=subscribe
/// - hub.challenge=<random string>
/// - hub.verify_token=<your configured token>
///
/// We must respond with the challenge value to verify.
fn handle_verification(req: &IncomingHttpRequest) -> OutgoingHttpResponse {
    if !req.secret_validated {
        channel_host::log(
            channel_host::LogLevel::Warn,
            "Webhook verification request failed host-side token validation",
        );
        return OutgoingHttpResponse {
            status: 403,
            headers_json: r#"{"Content-Type": "text/plain"}"#.to_string(),
            body: b"Verification failed".to_vec(),
        };
    }

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

            if !value.statuses.is_empty() {
                log_status_updates(&value.statuses, &phone_number_id);
                if value.messages.is_empty() {
                    continue;
                }
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

fn log_status_updates(statuses: &[MessageStatus], phone_number_id: &str) {
    for status in statuses {
        let details = serde_json::json!({
            "phone_number_id": phone_number_id,
            "message_id": status.id,
            "status": status.status,
            "timestamp": status.timestamp,
            "recipient_id": status.recipient_id,
        });
        channel_host::log(
            channel_host::LogLevel::Debug,
            &format!("WhatsApp status update: {}", details),
        );
    }
}

fn normalize_location_message(location: &WhatsAppLocation) -> (String, serde_json::Value) {
    let mut summary = format!(
        "Shared location: {}, {}",
        location.latitude, location.longitude
    );
    if let Some(name) = location.name.as_deref().filter(|value| !value.is_empty()) {
        summary = format!("Shared location: {}", name);
    }
    if let Some(address) = location
        .address
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        summary.push_str(&format!(" ({})", address));
    }

    (
        summary,
        serde_json::json!({
            "type": "location",
            "latitude": location.latitude,
            "longitude": location.longitude,
            "name": location.name,
            "address": location.address,
            "url": location.url,
        }),
    )
}

fn normalize_contacts_message(contacts: &[WhatsAppContactCard]) -> (String, serde_json::Value) {
    let summaries: Vec<String> = contacts
        .iter()
        .map(|contact| {
            if let Some(name) = contact
                .name
                .as_ref()
                .and_then(|name| name.formatted_name.clone())
                .or_else(|| {
                    contact.name.as_ref().and_then(|name| {
                        let mut parts = Vec::new();
                        if let Some(first) =
                            name.first_name.as_ref().filter(|value| !value.is_empty())
                        {
                            parts.push(first.clone());
                        }
                        if let Some(last) =
                            name.last_name.as_ref().filter(|value| !value.is_empty())
                        {
                            parts.push(last.clone());
                        }
                        if parts.is_empty() {
                            None
                        } else {
                            Some(parts.join(" "))
                        }
                    })
                })
            {
                name
            } else if let Some(phone) = contact
                .phones
                .iter()
                .find_map(|phone| phone.phone.clone().or_else(|| phone.wa_id.clone()))
            {
                phone
            } else {
                "contact".to_string()
            }
        })
        .collect();

    (
        format!("Shared contact cards: {}", summaries.join(", ")),
        serde_json::json!({
            "type": "contacts",
            "count": contacts.len(),
            "contacts": contacts,
        }),
    )
}

fn normalize_interactive_message(interactive: &WhatsAppInteractive) -> (String, serde_json::Value) {
    let summary = match interactive.interactive_type.as_str() {
        "button_reply" => {
            let title = interactive
                .button_reply
                .as_ref()
                .and_then(|reply| reply.title.as_deref())
                .unwrap_or("button reply");
            format!("Selected button reply: {}", title)
        }
        "list_reply" => {
            let title = interactive
                .list_reply
                .as_ref()
                .and_then(|reply| reply.title.as_deref())
                .unwrap_or("list reply");
            format!("Selected list reply: {}", title)
        }
        "nfm_reply" => {
            let name = interactive
                .nfm_reply
                .as_ref()
                .and_then(|reply| reply.name.as_deref())
                .unwrap_or("flow reply");
            format!("Submitted flow reply: {}", name)
        }
        other => format!("Interactive reply received: {}", other),
    };

    (
        summary,
        serde_json::json!({
            "type": "interactive",
            "interactive_type": interactive.interactive_type,
            "button_reply": interactive.button_reply,
            "list_reply": interactive.list_reply,
            "nfm_reply": interactive.nfm_reply,
        }),
    )
}

fn normalize_reaction_message(reaction: &WhatsAppReaction) -> (String, serde_json::Value) {
    let emoji = reaction.emoji.as_deref().unwrap_or("(removed reaction)");
    (
        format!("Reacted with {}", emoji),
        serde_json::json!({
            "type": "reaction",
            "emoji": reaction.emoji,
            "message_id": reaction.message_id,
        }),
    )
}

fn normalized_message_content(message: &WhatsAppMessage) -> (String, Option<serde_json::Value>) {
    if let Some(text) = message
        .text
        .as_ref()
        .filter(|text| !text.body.trim().is_empty())
        .map(|text| text.body.clone())
    {
        return (text, None);
    }

    if let Some(caption) = message
        .image
        .as_ref()
        .and_then(|media| media.caption.clone())
        .or_else(|| {
            message
                .video
                .as_ref()
                .and_then(|media| media.caption.clone())
        })
        .or_else(|| {
            message
                .document
                .as_ref()
                .and_then(|document| document.caption.clone())
        })
        .filter(|caption| !caption.trim().is_empty())
    {
        return (caption, None);
    }

    if let Some(location) = message.location.as_ref() {
        let (summary, details) = normalize_location_message(location);
        return (summary, Some(details));
    }

    if let Some(contacts) = message
        .contacts
        .as_ref()
        .filter(|contacts| !contacts.is_empty())
    {
        let (summary, details) = normalize_contacts_message(contacts);
        return (summary, Some(details));
    }

    if let Some(interactive) = message.interactive.as_ref() {
        let (summary, details) = normalize_interactive_message(interactive);
        return (summary, Some(details));
    }

    if let Some(reaction) = message.reaction.as_ref() {
        let (summary, details) = normalize_reaction_message(reaction);
        return (summary, Some(details));
    }

    if matches!(
        message.message_type.as_str(),
        "image" | "audio" | "video" | "document" | "sticker"
    ) {
        return (
            String::new(),
            Some(serde_json::json!({
                "type": "media",
                "message_type": message.message_type,
            })),
        );
    }

    (
        format!("[WhatsApp {} message received]", message.message_type),
        Some(serde_json::json!({
            "type": "unsupported",
            "message_type": message.message_type,
        })),
    )
}

/// Process a single WhatsApp message.
fn handle_message(
    message: &WhatsAppMessage,
    phone_number_id: &str,
    contact_names: &std::collections::HashMap<String, String>,
) {
    let (text, event_details) = normalized_message_content(message);

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

    // Look up sender's name from contacts
    let user_name = contact_names.get(&message.from).cloned();

    // Permission check (WhatsApp is always DM)
    if !check_sender_permission(&message.from, user_name.as_deref(), phone_number_id) {
        return;
    }

    // Download media attachments
    let headers_json =
        serde_json::json!({"Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"}).to_string();
    let api_version = current_api_version();
    let mut attachments = Vec::new();

    for (media_id, mime_type, filename) in &media_descriptors {
        match download_whatsapp_media(media_id, &headers_json, &api_version) {
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
        "[WhatsApp media received - please analyze the attachment]".to_string()
    } else if text.is_empty() && has_media {
        "[WhatsApp media received, but the attachment could not be downloaded]".to_string()
    } else {
        text
    };

    // Build metadata for response routing
    let metadata = WhatsAppMessageMetadata {
        phone_number_id: phone_number_id.to_string(),
        sender_phone: message.from.clone(),
        message_id: Some(message.id.clone()),
        reply_to_message_id: Some(message.id.clone()),
        timestamp: Some(message.timestamp.clone()),
        conversation_kind: Some("direct".to_string()),
        conversation_scope_id: Some(conversation_scope_id(phone_number_id, &message.from)),
        external_conversation_key: Some(external_conversation_key(phone_number_id, &message.from)),
        raw_sender_id: Some(message.from.clone()),
        stable_sender_id: Some(message.from.clone()),
        inbound_message_type: Some(message.message_type.clone()),
        context_message_id: message
            .context
            .as_ref()
            .map(|context| context.message_id.clone()),
        event_details,
        response_attachments: Vec::new(),
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

#[derive(Debug, Clone)]
struct OutboundMediaAttachment {
    mime_type: String,
    filename: Option<String>,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutboundMediaKind {
    Image,
    Audio,
    Video,
    Document,
    Sticker,
}

impl OutboundMediaKind {
    fn as_str(&self) -> &'static str {
        match self {
            OutboundMediaKind::Image => "image",
            OutboundMediaKind::Audio => "audio",
            OutboundMediaKind::Video => "video",
            OutboundMediaKind::Document => "document",
            OutboundMediaKind::Sticker => "sticker",
        }
    }
}

fn current_api_version() -> String {
    channel_host::workspace_read("channels/whatsapp/api_version")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_api_version)
}

fn reply_to_message_enabled() -> bool {
    channel_host::workspace_read("channels/whatsapp/reply_to_message")
        .map(|s| s == "true")
        .unwrap_or_else(default_reply_to_message)
}

fn decode_response_attachments(
    attachments: &[ResponseAttachmentEnvelope],
) -> Vec<OutboundMediaAttachment> {
    use base64::Engine;

    attachments
        .iter()
        .filter_map(|attachment| {
            match base64::engine::general_purpose::STANDARD.decode(&attachment.data) {
                Ok(data) => Some(OutboundMediaAttachment {
                    mime_type: attachment.mime_type.clone(),
                    filename: attachment.filename.clone(),
                    data,
                }),
                Err(error) => {
                    channel_host::log(
                        channel_host::LogLevel::Warn,
                        &format!(
                            "Skipping outbound attachment with invalid base64 payload: {}",
                            error
                        ),
                    );
                    None
                }
            }
        })
        .collect()
}

fn outbound_media_kind_for_mime(mime_type: &str) -> OutboundMediaKind {
    let mime = mime_type.to_ascii_lowercase();
    if mime == "image/webp" {
        OutboundMediaKind::Sticker
    } else if mime.starts_with("image/") && mime != "image/svg+xml" {
        OutboundMediaKind::Image
    } else if mime.starts_with("audio/") {
        OutboundMediaKind::Audio
    } else if mime.starts_with("video/") {
        OutboundMediaKind::Video
    } else {
        OutboundMediaKind::Document
    }
}

fn filename_for_attachment(attachment: &OutboundMediaAttachment) -> String {
    attachment.filename.clone().unwrap_or_else(|| {
        let ext = attachment
            .mime_type
            .split('/')
            .next_back()
            .unwrap_or("bin")
            .replace('+', "_");
        format!("attachment.{}", ext)
    })
}

fn classify_graph_api_error_details(
    kind: &str,
    status: u16,
    code: Option<i64>,
    message: &str,
    error_type: Option<&str>,
) -> String {
    let lowered = message.to_ascii_lowercase();
    let mut detail_parts = Vec::new();
    if let Some(code) = code {
        detail_parts.push(format!("code {}", code));
    }
    if let Some(error_type) = error_type.filter(|value| !value.is_empty()) {
        detail_parts.push(format!("type {}", error_type));
    }
    let detail_suffix = if detail_parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", detail_parts.join(", "))
    };

    if lowered.contains("24-hour")
        || lowered.contains("24 hours")
        || lowered.contains("customer care window")
        || lowered.contains("outside the allowed window")
        || lowered.contains("re-engagement")
        || (lowered.contains("template") && lowered.contains("required"))
        || code == Some(131047)
    {
        format!(
            "WhatsApp {} failed: template required outside the 24-hour window ({}){}",
            kind, message, detail_suffix
        )
    } else if status == 401
        || status == 403
        || matches!(code, Some(10 | 190 | 200 | 368))
        || lowered.contains("permission")
        || lowered.contains("access token")
        || lowered.contains("not authorized")
    {
        format!(
            "WhatsApp {} failed: auth or permission error ({}){}",
            kind, message, detail_suffix
        )
    } else if kind == "media upload" || code == Some(131053) || lowered.contains("media") {
        format!(
            "WhatsApp media upload failed ({}){}",
            message, detail_suffix
        )
    } else if lowered.contains("recipient")
        || lowered.contains("phone_number_id")
        || lowered.contains("phone number")
        || lowered.contains("invalid parameter")
        || lowered.contains("unsupported post request")
    {
        format!(
            "WhatsApp {} failed: invalid recipient or routing metadata ({}){}",
            kind, message, detail_suffix
        )
    } else {
        format!(
            "WhatsApp {} failed with HTTP {} ({}){}",
            kind, status, message, detail_suffix
        )
    }
}

fn classify_graph_api_error(kind: &str, status: u16, body: &[u8]) -> String {
    let parsed = serde_json::from_slice::<GraphApiErrorResponse>(body)
        .ok()
        .map(|response| response.error);
    let body_str = String::from_utf8_lossy(body).trim().to_string();

    if let Some(error) = parsed.as_ref() {
        classify_graph_api_error_details(
            kind,
            status,
            error.code,
            &error.message,
            error.error_type.as_deref(),
        )
    } else {
        classify_graph_api_error_details(
            kind,
            status,
            None,
            if body_str.is_empty() {
                "empty error response"
            } else {
                &body_str
            },
            None,
        )
    }
}

fn send_whatsapp_json_request(
    kind: &str,
    api_version: &str,
    phone_number_id: &str,
    payload: &serde_json::Value,
) -> Result<String, String> {
    if phone_number_id.trim().is_empty() {
        return Err(
            "WhatsApp message send failed: invalid recipient or routing metadata (missing phone_number_id)"
                .to_string(),
        );
    }

    let api_url = format!(
        "https://graph.facebook.com/{}/{}/messages",
        api_version, phone_number_id
    );
    let headers = serde_json::json!({
        "Content-Type": "application/json",
        "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}"
    });
    let payload_bytes =
        serde_json::to_vec(payload).map_err(|e| format!("Failed to serialize payload: {}", e))?;

    let response = channel_host::http_request(
        "POST",
        &api_url,
        &headers.to_string(),
        Some(&payload_bytes),
        None,
    )
    .map_err(|e| format!("HTTP request failed: {}", e))?;

    if response.status < 200 || response.status >= 300 {
        return Err(classify_graph_api_error(
            kind,
            response.status,
            &response.body,
        ));
    }

    let parsed: Result<WhatsAppApiResponse, _> = serde_json::from_slice(&response.body);
    match parsed {
        Ok(api_response) => {
            if let Some(error) = api_response.error.as_ref() {
                return Err(classify_graph_api_error_details(
                    kind,
                    response.status,
                    error.code,
                    &error.message,
                    error.error_type.as_deref(),
                ));
            }

            Ok(api_response
                .messages
                .and_then(|mut messages| messages.pop())
                .map(|message| message.id)
                .unwrap_or_else(|| "unknown".to_string()))
        }
        Err(_) => Ok("unknown".to_string()),
    }
}

fn send_text_message(
    api_version: &str,
    phone_number_id: &str,
    recipient_phone: &str,
    reply_to_message_id: Option<&str>,
    body: &str,
) -> Result<String, String> {
    if recipient_phone.trim().is_empty() {
        return Err(
            "WhatsApp message send failed: invalid recipient or routing metadata (missing recipient_phone)"
                .to_string(),
        );
    }

    let mut payload = serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient_phone,
        "type": "text",
        "text": {
            "preview_url": false,
            "body": body
        }
    });

    if let Some(reply_to_message_id) = reply_to_message_id.filter(|value| !value.is_empty()) {
        payload["context"] = serde_json::json!({
            "message_id": reply_to_message_id
        });
    }

    send_whatsapp_json_request("message send", api_version, phone_number_id, &payload)
}

fn build_media_upload_body(boundary: &str, attachment: &OutboundMediaAttachment) -> Vec<u8> {
    let filename = filename_for_attachment(attachment).replace('"', "_");
    let mut body = Vec::new();

    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"messaging_product\"\r\n\r\nwhatsapp\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {}\r\n\r\n",
            attachment.mime_type
        )
        .as_bytes(),
    );
    body.extend_from_slice(&attachment.data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

fn upload_outbound_media(
    api_version: &str,
    phone_number_id: &str,
    attachment: &OutboundMediaAttachment,
) -> Result<String, String> {
    if phone_number_id.trim().is_empty() {
        return Err(
            "WhatsApp media upload failed: invalid recipient or routing metadata (missing phone_number_id)"
                .to_string(),
        );
    }

    let boundary = format!("thinclaw-whatsapp-{}", channel_host::now_millis());
    let body = build_media_upload_body(&boundary, attachment);
    let url = format!(
        "https://graph.facebook.com/{}/{}/media",
        api_version, phone_number_id
    );
    let headers = serde_json::json!({
        "Authorization": "Bearer {WHATSAPP_ACCESS_TOKEN}",
        "Content-Type": format!("multipart/form-data; boundary={}", boundary)
    });

    let response = channel_host::http_request(
        "POST",
        &url,
        &headers.to_string(),
        Some(&body),
        Some(60_000),
    )
    .map_err(|e| format!("HTTP request failed: {}", e))?;

    if response.status < 200 || response.status >= 300 {
        return Err(classify_graph_api_error(
            "media upload",
            response.status,
            &response.body,
        ));
    }

    let parsed: MediaUploadResponse = serde_json::from_slice(&response.body)
        .map_err(|e| format!("Failed to parse media upload response: {}", e))?;
    Ok(parsed.id)
}

fn send_outbound_attachment(
    api_version: &str,
    phone_number_id: &str,
    recipient_phone: &str,
    reply_to_message_id: Option<&str>,
    attachment: &OutboundMediaAttachment,
) -> Result<String, String> {
    if recipient_phone.trim().is_empty() {
        return Err(
            "WhatsApp message send failed: invalid recipient or routing metadata (missing recipient_phone)"
                .to_string(),
        );
    }

    let media_id = upload_outbound_media(api_version, phone_number_id, attachment)?;
    let send_kind = outbound_media_kind_for_mime(&attachment.mime_type);
    let filename = filename_for_attachment(attachment);

    let media_object = match send_kind {
        OutboundMediaKind::Image => serde_json::json!({ "id": media_id }),
        OutboundMediaKind::Audio => serde_json::json!({ "id": media_id }),
        OutboundMediaKind::Video => serde_json::json!({ "id": media_id }),
        OutboundMediaKind::Document => serde_json::json!({
            "id": media_id,
            "filename": filename
        }),
        OutboundMediaKind::Sticker => serde_json::json!({ "id": media_id }),
    };

    let mut payload = serde_json::json!({
        "messaging_product": "whatsapp",
        "recipient_type": "individual",
        "to": recipient_phone,
        "type": send_kind.as_str(),
    });
    payload[send_kind.as_str()] = media_object;

    if let Some(reply_to_message_id) = reply_to_message_id.filter(|value| !value.is_empty()) {
        payload["context"] = serde_json::json!({
            "message_id": reply_to_message_id
        });
    }

    send_whatsapp_json_request("message send", api_version, phone_number_id, &payload)
}

fn media_lookup_url(api_version: &str, media_id: &str) -> String {
    format!("https://graph.facebook.com/{}/{}", api_version, media_id)
}

/// Download media from the WhatsApp Cloud API.
///
/// Step 1: GET `https://graph.facebook.com/<api_version>/<media_id>` → get download URL
/// Step 2: GET the download URL → binary data
fn download_whatsapp_media(
    media_id: &str,
    headers_json: &str,
    api_version: &str,
) -> Result<Vec<u8>, String> {
    // Step 1: Get the media download URL
    let url = media_lookup_url(api_version, media_id);

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
    let api_version = current_api_version();

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
    if text.chars().count() <= max_len {
        return vec![text.to_string()];
    }

    fn byte_index_for_char_limit(text: &str, max_chars: usize) -> usize {
        text.char_indices()
            .nth(max_chars)
            .map(|(index, _)| index)
            .unwrap_or(text.len())
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.chars().count() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let search_end = byte_index_for_char_limit(remaining, max_len);
        let search_area = &remaining[..search_end];

        let split_at = search_area
            .rfind("\n\n")
            .map(|pos| pos + 1)
            .or_else(|| search_area.rfind('\n'))
            .or_else(|| search_area.rfind(' '))
            .unwrap_or(search_end);

        if split_at == 0 {
            chunks.push(search_area.to_string());
            remaining = remaining[search_end..].trim_start();
            continue;
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

    fn blank_message(message_type: &str) -> WhatsAppMessage {
        WhatsAppMessage {
            id: "wamid.test".to_string(),
            from: "15551234567".to_string(),
            timestamp: "1234567890".to_string(),
            message_type: message_type.to_string(),
            text: None,
            image: None,
            audio: None,
            video: None,
            document: None,
            sticker: None,
            location: None,
            contacts: None,
            interactive: None,
            reaction: None,
            context: None,
        }
    }

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
            message_id: Some("wamid.abc".to_string()),
            reply_to_message_id: Some("wamid.abc".to_string()),
            timestamp: Some("1234567890".to_string()),
            conversation_kind: Some("direct".to_string()),
            conversation_scope_id: Some("whatsapp:direct:123456:15551234567".to_string()),
            external_conversation_key: Some("whatsapp://direct/123456/15551234567".to_string()),
            raw_sender_id: Some("15551234567".to_string()),
            stable_sender_id: Some("15551234567".to_string()),
            inbound_message_type: Some("text".to_string()),
            context_message_id: None,
            event_details: None,
            response_attachments: Vec::new(),
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

    #[test]
    fn test_split_message_is_unicode_safe() {
        let text = "🙂".repeat(5000);
        let chunks = split_message(&text, WHATSAPP_MAX_MESSAGE_LENGTH);

        assert_eq!(chunks.len(), 2);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.chars().count())
                .sum::<usize>(),
            5000
        );
    }

    #[test]
    fn test_normalize_location_message() {
        let mut message = blank_message("location");
        message.location = Some(WhatsAppLocation {
            latitude: 52.52,
            longitude: 13.405,
            name: Some("Berlin".to_string()),
            address: Some("Alexanderplatz".to_string()),
            url: None,
        });

        let (summary, details) = normalized_message_content(&message);
        assert_eq!(summary, "Shared location: Berlin (Alexanderplatz)");
        assert_eq!(details.unwrap()["type"], "location");
    }

    #[test]
    fn test_normalize_contacts_message() {
        let mut message = blank_message("contacts");
        message.contacts = Some(vec![WhatsAppContactCard {
            name: Some(WhatsAppContactCardName {
                formatted_name: Some("Ada Lovelace".to_string()),
                first_name: None,
                last_name: None,
            }),
            phones: vec![],
            emails: vec![],
            org: None,
        }]);

        let (summary, details) = normalized_message_content(&message);
        assert_eq!(summary, "Shared contact cards: Ada Lovelace");
        assert_eq!(details.unwrap()["type"], "contacts");
    }

    #[test]
    fn test_normalize_interactive_reply() {
        let mut message = blank_message("interactive");
        message.interactive = Some(WhatsAppInteractive {
            interactive_type: "button_reply".to_string(),
            button_reply: Some(WhatsAppInteractiveButtonReply {
                id: Some("yes".to_string()),
                title: Some("Yes".to_string()),
            }),
            list_reply: None,
            nfm_reply: None,
        });

        let (summary, details) = normalized_message_content(&message);
        assert_eq!(summary, "Selected button reply: Yes");
        assert_eq!(details.unwrap()["type"], "interactive");
    }

    #[test]
    fn test_normalize_reaction_message() {
        let mut message = blank_message("reaction");
        message.reaction = Some(WhatsAppReaction {
            message_id: Some("wamid.original".to_string()),
            emoji: Some("👍".to_string()),
        });

        let (summary, details) = normalized_message_content(&message);
        assert_eq!(summary, "Reacted with 👍");
        assert_eq!(details.unwrap()["type"], "reaction");
    }

    #[test]
    fn test_media_without_caption_uses_media_placeholder_path() {
        let mut message = blank_message("image");
        message.image = Some(WhatsAppMedia {
            id: "media123".to_string(),
            mime_type: Some("image/jpeg".to_string()),
            caption: None,
        });

        let (summary, details) = normalized_message_content(&message);
        assert!(summary.is_empty());
        assert_eq!(details.unwrap()["type"], "media");
    }

    #[test]
    fn test_unknown_message_type_has_safe_fallback() {
        let message = blank_message("order");
        let (summary, details) = normalized_message_content(&message);

        assert_eq!(summary, "[WhatsApp order message received]");
        assert_eq!(details.unwrap()["type"], "unsupported");
    }

    #[test]
    fn test_outbound_media_kind_mapping() {
        assert_eq!(
            outbound_media_kind_for_mime("image/png"),
            OutboundMediaKind::Image
        );
        assert_eq!(
            outbound_media_kind_for_mime("audio/ogg"),
            OutboundMediaKind::Audio
        );
        assert_eq!(
            outbound_media_kind_for_mime("video/mp4"),
            OutboundMediaKind::Video
        );
        assert_eq!(
            outbound_media_kind_for_mime("image/webp"),
            OutboundMediaKind::Sticker
        );
        assert_eq!(
            outbound_media_kind_for_mime("application/pdf"),
            OutboundMediaKind::Document
        );
    }

    #[test]
    fn test_graph_api_error_classification_template_required() {
        let error = classify_graph_api_error_details(
            "message send",
            400,
            Some(131047),
            "More than 24 hours have passed since the customer last replied to this number.",
            Some("OAuthException"),
        );
        assert!(error.contains("template required"));
    }

    #[test]
    fn test_graph_api_error_classification_auth() {
        let error = classify_graph_api_error_details(
            "message send",
            401,
            Some(190),
            "Invalid OAuth access token.",
            Some("OAuthException"),
        );
        assert!(error.contains("auth or permission"));
    }

    #[test]
    fn test_graph_api_error_classification_media_upload() {
        let error = classify_graph_api_error_details(
            "media upload",
            400,
            Some(131053),
            "Unable to upload the media used in the message",
            None,
        );
        assert!(error.contains("media upload failed"));
    }

    #[test]
    fn test_graph_api_error_classification_invalid_routing() {
        let error = classify_graph_api_error_details(
            "message send",
            400,
            Some(100),
            "Invalid parameter: phone_number_id",
            None,
        );
        assert!(error.contains("invalid recipient or routing metadata"));
    }

    #[test]
    fn test_media_lookup_url_uses_configured_version() {
        assert_eq!(
            media_lookup_url("v23.0", "media-123"),
            "https://graph.facebook.com/v23.0/media-123"
        );
    }

    #[test]
    fn test_metadata_accepts_proactive_route_aliases() {
        let json = serde_json::json!({
            "phone_number_id": "123456",
            "recipient_phone": "15551234567",
            "reply_to_message_id": "wamid.reply",
            "response_attachments": [{
                "mime_type": "image/png",
                "filename": "image.png",
                "data": "aGVsbG8="
            }]
        })
        .to_string();

        let parsed: WhatsAppMessageMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.sender_phone, "15551234567");
        assert_eq!(parsed.reply_to_message_id.as_deref(), Some("wamid.reply"));
        assert_eq!(parsed.response_attachments.len(), 1);
    }
}
