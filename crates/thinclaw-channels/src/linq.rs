//! Optional Linq Partner API v3 messaging channel.
//!
//! Linq provides a managed, headless iMessage/RCS/SMS deployment mode. This
//! adapter deliberately pins the 2026-02-03 webhook contract, verifies the
//! Standard Webhooks signature over the raw request body, persists bounded
//! event-id deduplication, and supplies an idempotency key for every send.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Utc};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::{Client, Url};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use thinclaw_channels_core::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use thinclaw_types::MediaContent;
use thinclaw_types::error::ChannelError;

mod state;

use state::{
    AcceptResult, DeliveryHealth, DurableState, InboxWork, OutboundTarget, OutboxWork,
    StoredResponse,
};

pub const LINQ_API_KEY_SECRET: &str = "linq_api_key";
pub const LINQ_WEBHOOK_SECRET: &str = "linq_webhook_secret";
pub const LINQ_WEBHOOK_VERSION: &str = "2026-02-03";
pub const DEFAULT_LINQ_API_BASE_URL: &str = "https://api.linqapp.com/api/partner/v3/";

const NAME: &str = "linq";
const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;
const MAX_API_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 10_000;
const MAX_INBOUND_TEXT_BYTES: usize = 256 * 1024;
const MAX_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
const MAX_TOTAL_ATTACHMENT_BYTES: usize = 40 * 1024 * 1024;
const MAX_ATTACHMENTS: usize = 10;
const MAX_WEBHOOK_SKEW_SECS: i64 = 5 * 60;
const MAX_WEBHOOK_CONCURRENCY: usize = 16;
const CHANNEL_CAPACITY: usize = 128;
const API_ATTEMPTS: usize = 3;

type HmacSha256 = Hmac<Sha256>;

/// Explicit protocol selection. `IMessage` is the safe default: Linq cannot
/// silently fall back to carrier messaging unless the operator selects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinqPreferredService {
    IMessage,
    Auto,
    Rcs,
    Sms,
}

impl LinqPreferredService {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "imessage" | "i_message" | "i-message" => Ok(Self::IMessage),
            "auto" => Ok(Self::Auto),
            "rcs" => Ok(Self::Rcs),
            "sms" => Ok(Self::Sms),
            _ => Err("must be one of: imessage, auto, rcs, sms".to_string()),
        }
    }

    pub const fn as_api_value(self) -> Option<&'static str> {
        match self {
            Self::IMessage => Some("iMessage"),
            Self::Auto => None,
            Self::Rcs => Some("RCS"),
            Self::Sms => Some("SMS"),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IMessage => "imessage",
            Self::Auto => "auto",
            Self::Rcs => "rcs",
            Self::Sms => "sms",
        }
    }
}

/// Fully resolved channel configuration. Secrets are supplied by the runtime
/// after scoped retrieval from `SecretsStore` (or explicit environment input).
#[derive(Clone)]
pub struct LinqConfig {
    pub api_base_url: Url,
    pub api_key: SecretString,
    pub webhook_secret: SecretString,
    pub from_number: String,
    pub webhook_host: String,
    pub webhook_port: u16,
    pub webhook_path: String,
    pub allow_from: Vec<String>,
    pub preferred_service: LinqPreferredService,
    pub event_ledger_path: PathBuf,
}

impl std::fmt::Debug for LinqConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LinqConfig")
            .field("api_base_url", &redacted_origin(&self.api_base_url))
            .field("api_key", &"[REDACTED]")
            .field("webhook_secret", &"[REDACTED]")
            .field("from_number", &redact_handle(&self.from_number))
            .field("webhook_host", &self.webhook_host)
            .field("webhook_port", &self.webhook_port)
            .field("webhook_path", &self.webhook_path)
            .field("allow_from_count", &self.allow_from.len())
            .field("preferred_service", &self.preferred_service)
            .field("event_ledger_path", &self.event_ledger_path)
            .finish()
    }
}

impl LinqConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_base_url: Url,
        api_key: SecretString,
        webhook_secret: SecretString,
        from_number: String,
        webhook_host: String,
        webhook_port: u16,
        webhook_path: String,
        allow_from: Vec<String>,
        preferred_service: LinqPreferredService,
        event_ledger_path: PathBuf,
    ) -> Result<Self, ChannelError> {
        validate_api_base_url(&api_base_url)?;
        if api_key.expose_secret().trim().is_empty() || api_key.expose_secret().len() > 16 * 1024 {
            return Err(startup_error("API key is empty or oversized"));
        }
        decode_webhook_key(webhook_secret.expose_secret())
            .map_err(|_| startup_error("webhook secret is not a valid whsec_ value"))?;
        if !valid_e164(&from_number) {
            return Err(startup_error("from_number must use E.164 format"));
        }
        webhook_host
            .parse::<std::net::IpAddr>()
            .map_err(|_| startup_error("webhook host must be a numeric bind IP address"))?;
        if webhook_port == 0 {
            return Err(startup_error("webhook port must be between 1 and 65535"));
        }
        if webhook_path != "/webhook/linq" {
            return Err(startup_error("webhook path must be exactly /webhook/linq"));
        }
        if allow_from.len() > 4096
            || allow_from
                .iter()
                .any(|entry| !valid_allow_entry(entry.as_str()))
        {
            return Err(startup_error("allow_from contains an invalid entry"));
        }
        if event_ledger_path.file_name().is_none() {
            return Err(startup_error("event ledger path is invalid"));
        }
        Ok(Self {
            api_base_url,
            api_key,
            webhook_secret,
            from_number,
            webhook_host,
            webhook_port,
            webhook_path,
            allow_from,
            preferred_service,
            event_ledger_path,
        })
    }
}

#[derive(Clone)]
struct WebhookState {
    secret: SecretString,
    durable: Arc<DurableState>,
    wake_worker: Arc<Notify>,
}

#[derive(Clone)]
struct LinqApi {
    config: Arc<LinqConfig>,
    client: Client,
    durable: Arc<DurableState>,
}

pub struct LinqChannel {
    api: LinqApi,
    incoming_tx: Arc<StdMutex<Option<mpsc::Sender<IncomingMessage>>>>,
    incoming_rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    wake_worker: Arc<Notify>,
    worker: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl LinqChannel {
    pub fn new(config: LinqConfig) -> Result<Self, ChannelError> {
        let legacy_path = config
            .event_ledger_path
            .parent()
            .map(|parent| parent.join("linq-webhook-events.json"));
        let durable = Arc::new(
            DurableState::load_with_legacy(
                config.event_ledger_path.clone(),
                legacy_path.as_deref(),
            )
            .map_err(|error| startup_error(format!("durable state is unavailable: {error}")))?,
        );
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .user_agent("ThinClaw-Linq/1")
            .build()
            .map_err(|error| {
                startup_error(format!("HTTP client initialization failed: {error}"))
            })?;
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        Ok(Self {
            api: LinqApi {
                config: Arc::new(config),
                client,
                durable,
            },
            incoming_tx: Arc::new(StdMutex::new(Some(tx))),
            incoming_rx: Mutex::new(Some(rx)),
            wake_worker: Arc::new(Notify::new()),
            worker: Mutex::new(None),
        })
    }

    pub fn webhook_addr(&self) -> Result<std::net::SocketAddr, ChannelError> {
        let ip = self
            .api
            .config
            .webhook_host
            .parse::<std::net::IpAddr>()
            .map_err(|error| startup_error(format!("webhook bind IP is invalid: {error}")))?;
        Ok(std::net::SocketAddr::new(ip, self.api.config.webhook_port))
    }

    pub fn webhook_routes(&self) -> axum::Router {
        let state = WebhookState {
            secret: self.api.config.webhook_secret.clone(),
            durable: Arc::clone(&self.api.durable),
            wake_worker: Arc::clone(&self.wake_worker),
        };
        axum::Router::new()
            .route(&self.api.config.webhook_path, axum::routing::post(webhook))
            .with_state(state)
            .layer(DefaultBodyLimit::max(MAX_WEBHOOK_BODY_BYTES))
            .layer(tower::limit::ConcurrencyLimitLayer::new(
                MAX_WEBHOOK_CONCURRENCY,
            ))
    }

    async fn enqueue_delivery(
        &self,
        target: OutboundTarget,
        response: &OutgoingResponse,
    ) -> Result<(), ChannelError> {
        validate_outgoing_shape(response)?;
        if let Some(reason) = self.api.durable.delivery_block_reason(&target).await {
            return Err(send_error(reason));
        }
        let operation_id = response.delivery_id.to_string();
        self.api
            .durable
            .enqueue_outbox(
                &operation_id,
                target,
                StoredResponse::from_response(response),
            )
            .await
            .map_err(|error| send_error(format!("durable outbox enqueue failed: {error}")))?;
        let Some(work) = self
            .api
            .durable
            .claim_outbox(&operation_id)
            .await
            .map_err(|error| send_error(format!("durable outbox claim failed: {error}")))?
        else {
            if self.api.durable.outbox_completed(&operation_id).await {
                return Ok(());
            }
            return Err(send_error("delivery is durably queued for retry"));
        };
        let result = deliver_outbox_work(&self.api, &work).await;
        match result {
            Ok(()) => self
                .api
                .durable
                .complete_outbox(&operation_id)
                .await
                .map_err(|error| send_error(format!("durable outbox ack failed: {error}"))),
            Err(error) if is_terminal_delivery_error(&error) => {
                if is_opt_out_error(&error) {
                    let (chat, recipient) = target_identity(&work.target);
                    self.api
                        .durable
                        .mark_opted_out(chat, recipient)
                        .await
                        .map_err(|state_error| {
                            send_error(format!("opt-out persistence failed: {state_error}"))
                        })?;
                }
                self.api
                    .durable
                    .complete_outbox(&operation_id)
                    .await
                    .map_err(|state_error| {
                        send_error(format!("durable terminal ack failed: {state_error}"))
                    })?;
                Err(error)
            }
            Err(error) => {
                self.api
                    .durable
                    .retry_outbox(&operation_id, delivery_error_code(&error))
                    .await
                    .map_err(|state_error| {
                        send_error(format!("durable retry scheduling failed: {state_error}"))
                    })?;
                self.wake_worker.notify_one();
                Err(error)
            }
        }
    }
}

impl LinqApi {

    fn endpoint(&self, relative: &str) -> Result<Url, ChannelError> {
        self.config
            .api_base_url
            .join(relative)
            .map_err(|error| send_error(format!("API endpoint is invalid: {error}")))
    }

    async fn post_json(&self, relative: &str, payload: &Value) -> Result<Value, ChannelError> {
        let endpoint = self.endpoint(relative)?;
        for attempt in 0..API_ATTEMPTS {
            let response = self
                .client
                .post(endpoint.clone())
                .bearer_auth(self.config.api_key.expose_secret())
                .json(payload)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => {
                    return crate::response::bounded_json(response, MAX_API_RESPONSE_BYTES)
                        .await
                        .map_err(|_| send_error("api_retryable_invalid_response"));
                }
                Ok(response)
                    if (response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error())
                        && attempt + 1 < API_ATTEMPTS =>
                {
                    tokio::time::sleep(retry_delay_from_response(&response, attempt)).await;
                }
                Ok(response) => {
                    let status = response.status();
                    let retryable = status == StatusCode::TOO_MANY_REQUESTS
                        || status.is_server_error();
                    let body = crate::response::bounded_bytes(response, MAX_API_RESPONSE_BYTES)
                        .await
                        .unwrap_or_default();
                    if status == StatusCode::FORBIDDEN && provider_error_code(&body) == Some(2024) {
                        return Err(send_error("recipient_opted_out"));
                    }
                    if retryable {
                        return Err(send_error("api_retryable_status"));
                    }
                    return Err(send_error(format!("api_rejected_status_{}", status.as_u16())));
                }
                Err(error) if attempt + 1 < API_ATTEMPTS => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        error = %error.without_url(),
                        "Linq API request failed; retrying with the same idempotency key"
                    );
                    tokio::time::sleep(retry_delay(attempt)).await;
                }
                Err(error) => {
                    tracing::warn!(error = %error.without_url(), "Linq API retry budget exhausted");
                    return Err(send_error("api_retryable_network"));
                }
            }
        }
        Err(send_error("api_retryable_budget"))
    }

    async fn upload_attachment(&self, attachment: &MediaContent) -> Result<String, ChannelError> {
        validate_attachment(attachment)?;
        let filename = attachment
            .filename
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or("attachment");
        let init = self
            .post_json(
                "attachments",
                &json!({
                    "filename": filename,
                    "content_type": attachment.mime_type,
                    "size_bytes": attachment.data.len(),
                }),
            )
            .await?;
        let attachment_id = bounded_string(&init, "attachment_id", 128)?;
        uuid::Uuid::parse_str(&attachment_id)
            .map_err(|_| send_error("Linq upload response has an invalid attachment_id"))?;
        if init.get("http_method").and_then(Value::as_str) != Some("PUT") {
            return Err(send_error(
                "Linq upload response requires an unsupported HTTP method",
            ));
        }
        let upload_url = bounded_string(&init, "upload_url", 8192)?;
        let upload_url = Url::parse(&upload_url)
            .map_err(|_| send_error("Linq upload response has an invalid upload_url"))?;
        validate_upload_url(&upload_url)?;
        let upload_client = pinned_transfer_client(&upload_url, &[]).await?;
        let required = init
            .get("required_headers")
            .and_then(Value::as_object)
            .ok_or_else(|| send_error("Linq upload response omitted required_headers"))?;
        if required.len() > 32 {
            return Err(send_error(
                "Linq upload response has too many required headers",
            ));
        }
        let mut required_headers = Vec::with_capacity(required.len());
        for (name, value) in required {
            let lower = name.to_ascii_lowercase();
            if matches!(
                lower.as_str(),
                "authorization"
                    | "connection"
                    | "cookie"
                    | "host"
                    | "keep-alive"
                    | "proxy-authenticate"
                    | "proxy-authorization"
                    | "te"
                    | "trailer"
                    | "transfer-encoding"
                    | "upgrade"
            ) {
                return Err(send_error(
                    "Linq upload response requested a forbidden header",
                ));
            }
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| send_error("Linq upload response has an invalid header name"))?;
            let value = value
                .as_str()
                .filter(|value| value.len() <= 4096)
                .ok_or_else(|| send_error("Linq upload response has an invalid header value"))?;
            let value = HeaderValue::from_str(value)
                .map_err(|_| send_error("Linq upload response has an invalid header value"))?;
            required_headers.push((name, value));
        }
        if let Some((_, value)) = required_headers
            .iter()
            .find(|(name, _)| name.as_str() == "content-length")
            && value.to_str().ok().and_then(|value| value.parse().ok())
                != Some(attachment.data.len())
        {
            return Err(send_error(
                "Linq upload response has an inconsistent Content-Length",
            ));
        }
        for attempt in 0..API_ATTEMPTS {
            let mut request = upload_client
                .put(upload_url.clone())
                .body(attachment.data.clone());
            for (name, value) in &required_headers {
                request = request.header(name, value);
            }
            match request.send().await {
                Ok(response) if response.status().is_success() => return Ok(attachment_id),
                Ok(response)
                    if (response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error())
                        && attempt + 1 < API_ATTEMPTS =>
                {
                    tokio::time::sleep(retry_delay_from_response(&response, attempt)).await;
                }
                Ok(response) => {
                    let retryable = response.status() == StatusCode::TOO_MANY_REQUESTS
                        || response.status().is_server_error();
                    return Err(send_error(if retryable {
                        "attachment_upload_retryable_status"
                    } else {
                        "attachment_upload_rejected"
                    }));
                }
                Err(error) if attempt + 1 < API_ATTEMPTS => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        error = %error.without_url(),
                        "Linq attachment upload failed; retrying the same presigned PUT"
                    );
                    tokio::time::sleep(retry_delay(attempt)).await;
                }
                Err(error) => {
                    tracing::warn!(error = %error.without_url(), "Linq attachment upload retry budget exhausted");
                    return Err(send_error("attachment_upload_retryable_network"));
                }
            }
        }
        Err(send_error("attachment_upload_retryable_budget"))
    }

    async fn message_parts(
        &self,
        text: &str,
        attachments: &[MediaContent],
    ) -> Result<Vec<Value>, ChannelError> {
        if attachments.len() > MAX_ATTACHMENTS {
            return Err(send_error("too many Linq attachments"));
        }
        let total = attachments.iter().try_fold(0usize, |total, attachment| {
            total
                .checked_add(attachment.data.len())
                .ok_or_else(|| send_error("Linq attachment size overflow"))
        })?;
        if total > MAX_TOTAL_ATTACHMENT_BYTES {
            return Err(send_error("Linq attachments exceed the total size limit"));
        }
        let mut parts = Vec::with_capacity(attachments.len() + usize::from(!text.is_empty()));
        if !text.is_empty() {
            parts.push(json!({"type": "text", "value": text}));
        }
        for attachment in attachments {
            let id = self.upload_attachment(attachment).await?;
            parts.push(json!({"type": "media", "attachment_id": id}));
        }
        if parts.is_empty() {
            return Err(send_error("Linq message has no text or media"));
        }
        Ok(parts)
    }

    async fn send_existing_chat(
        &self,
        chat_id: &str,
        incoming_message_id: Option<&str>,
        response: &OutgoingResponse,
        idempotency_seed: &str,
    ) -> Result<(), ChannelError> {
        validate_uuid_component(chat_id, "chat id")?;
        if let Some(message_id) = incoming_message_id {
            validate_uuid_component(message_id, "reply message id")?;
        }
        validate_outgoing_shape(response)?;
        let chunks = split_text(&response.content, MAX_TEXT_CHARS);
        let chunk_count = chunks.len().max(1);
        for index in 0..chunk_count {
            let text = chunks.get(index).copied().unwrap_or("");
            let attachments = if index == 0 {
                response.attachments.as_slice()
            } else {
                &[]
            };
            let parts = self.message_parts(text, attachments).await?;
            let mut message = Map::new();
            message.insert("parts".to_string(), Value::Array(parts));
            message.insert(
                "idempotency_key".to_string(),
                Value::String(idempotency_key(idempotency_seed, index)),
            );
            if let Some(service) = self.config.preferred_service.as_api_value() {
                message.insert(
                    "preferred_service".to_string(),
                    Value::String(service.to_string()),
                );
            }
            if index == 0
                && let Some(message_id) = incoming_message_id
            {
                message.insert(
                    "reply_to".to_string(),
                    json!({"message_id": message_id, "part_index": 0}),
                );
            }
            self.post_json(
                &format!("chats/{chat_id}/messages"),
                &json!({"message": message}),
            )
            .await?;
        }
        Ok(())
    }

    async fn send_pooled(
        &self,
        recipient: &str,
        response: &OutgoingResponse,
        idempotency_seed: &str,
    ) -> Result<(), ChannelError> {
        if !valid_external_handle(recipient) {
            return Err(send_error(
                "Linq recipient must be an E.164 number or email address",
            ));
        }
        validate_outgoing_shape(response)?;
        let chunks = split_text(&response.content, MAX_TEXT_CHARS);
        let first_text = chunks.first().copied().unwrap_or("");
        let parts = self
            .message_parts(first_text, &response.attachments)
            .await?;
        let mut message = Map::new();
        message.insert("parts".to_string(), Value::Array(parts));
        message.insert(
            "idempotency_key".to_string(),
            Value::String(idempotency_key(idempotency_seed, 0)),
        );
        if let Some(service) = self.config.preferred_service.as_api_value() {
            message.insert(
                "preferred_service".to_string(),
                Value::String(service.to_string()),
            );
        }
        let created = self
            .post_json(
                "messages",
                &json!({
                    "to": [recipient],
                    "message": message,
                }),
            )
            .await?;
        if chunks.len() > 1 {
            let chat_id = bounded_string(&created, "chat_id", 128)
                .map_err(|_| send_error("Linq pooled-send response omitted chat_id"))?;
            validate_uuid_component(&chat_id, "created chat id")?;
            for (index, text) in chunks.iter().enumerate().skip(1) {
                let mut message = Map::new();
                message.insert(
                    "parts".to_string(),
                    Value::Array(self.message_parts(text, &[]).await?),
                );
                message.insert(
                    "idempotency_key".to_string(),
                    Value::String(idempotency_key(idempotency_seed, index)),
                );
                if let Some(service) = self.config.preferred_service.as_api_value() {
                    message.insert(
                        "preferred_service".to_string(),
                        Value::String(service.to_string()),
                    );
                }
                self.post_json(
                    &format!("chats/{chat_id}/messages"),
                    &json!({"message": message}),
                )
                .await?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Channel for LinqChannel {
    fn name(&self) -> &str {
        NAME
    }

    fn config_schema(&self) -> Option<thinclaw_channels_core::ConfigSchema> {
        use thinclaw_channels_core::{ConfigField, ConfigOption, ConfigSchema};
        Some(ConfigSchema {
            channel_id: NAME.to_string(),
            channel_name: "Linq managed messaging".to_string(),
            fields: vec![
                ConfigField {
                    id: "from_number".to_string(),
                    label: "Managed sender number".to_string(),
                    field_type: "text".to_string(),
                    required: true,
                    help_text: Some("Linq-managed E.164 sender, for example +12025550100.".to_string()),
                    default_value: Some(Value::String(self.api.config.from_number.clone())),
                    options: None,
                },
                ConfigField {
                    id: "allow_from".to_string(),
                    label: "Allowed inbound senders".to_string(),
                    field_type: "textarea".to_string(),
                    required: false,
                    help_text: Some(
                        "One E.164 number, email, or Linq handle UUID per line. Empty denies all; * explicitly allows all."
                            .to_string(),
                    ),
                    default_value: Some(Value::String(self.api.config.allow_from.join("\n"))),
                    options: None,
                },
                ConfigField {
                    id: "preferred_service".to_string(),
                    label: "Outbound service".to_string(),
                    field_type: "select".to_string(),
                    required: true,
                    help_text: Some(
                        "iMessage prevents silent carrier fallback; Auto lets Linq choose."
                            .to_string(),
                    ),
                    default_value: Some(Value::String(
                        self.api.config.preferred_service.as_str().to_string(),
                    )),
                    options: Some(vec![
                        ConfigOption {
                            value: "imessage".to_string(),
                            label: "iMessage only".to_string(),
                        },
                        ConfigOption {
                            value: "auto".to_string(),
                            label: "Automatic (fallback allowed)".to_string(),
                        },
                        ConfigOption {
                            value: "rcs".to_string(),
                            label: "RCS only".to_string(),
                        },
                        ConfigOption {
                            value: "sms".to_string(),
                            label: "SMS only".to_string(),
                        },
                    ]),
                },
                ConfigField {
                    id: "webhook_host".to_string(),
                    label: "Local webhook host".to_string(),
                    field_type: "text".to_string(),
                    required: true,
                    help_text: Some("Must match HTTP_HOST when sharing its listener.".to_string()),
                    default_value: Some(Value::String(self.api.config.webhook_host.clone())),
                    options: None,
                },
                ConfigField {
                    id: "webhook_port".to_string(),
                    label: "Local webhook port".to_string(),
                    field_type: "number".to_string(),
                    required: true,
                    help_text: Some("Must match HTTP_PORT when sharing its listener.".to_string()),
                    default_value: Some(json!(self.api.config.webhook_port)),
                    options: None,
                },
            ],
            help: Some(
                "Secrets remain in Provider Vault. Non-secret changes are validated and take effect after a channel restart."
                    .to_string(),
            ),
        })
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let receiver = {
            let mut guard = self.incoming_rx.lock().await;
            match guard.take() {
                Some(receiver) => receiver,
                None => {
                    let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
                    *self
                        .incoming_tx
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(sender);
                    receiver
                }
            }
        };
        let mut worker = self.worker.lock().await;
        if worker.is_none() {
            *worker = Some(tokio::spawn(run_durable_worker(
                self.api.clone(),
                Arc::clone(&self.incoming_tx),
                Arc::clone(&self.wake_worker),
            )));
            self.wake_worker.notify_one();
        }
        Ok(Box::pin(ReceiverStream::new(receiver)))
    }

    async fn respond(
        &self,
        message: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let chat_id = message
            .metadata
            .get("linq_chat_id")
            .and_then(Value::as_str)
            .or_else(|| {
                message
                    .metadata
                    .get("raw")
                    .and_then(|raw| raw.get("linq_chat_id"))
                    .and_then(Value::as_str)
            })
            .ok_or_else(|| send_error("inbound Linq message omitted its chat id"))?;
        let reply_to = message
            .metadata
            .get("linq_message_id")
            .and_then(Value::as_str)
            .or_else(|| {
                message
                    .metadata
                    .get("raw")
                    .and_then(|raw| raw.get("linq_message_id"))
                    .and_then(Value::as_str)
            });
        self.enqueue_delivery(
            OutboundTarget::ExistingChat {
                chat_id: chat_id.to_string(),
                reply_to: reply_to.map(str::to_string),
                recipient: Some(message.user_id.clone()),
            },
            &response,
        )
        .await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        if let Some(chat_id) = response
            .thread_id
            .as_deref()
            .and_then(linq_chat_id_from_thread)
        {
            self.enqueue_delivery(
                OutboundTarget::ExistingChat {
                    chat_id,
                    reply_to: None,
                    recipient: valid_external_handle(user_id).then(|| user_id.to_string()),
                },
                &response,
            )
            .await
        } else {
            if !valid_external_handle(user_id) {
                return Err(send_error(
                    "Linq recipient must be an E.164 number or email address",
                ));
            }
            self.enqueue_delivery(
                OutboundTarget::PooledRecipient {
                    recipient: user_id.to_string(),
                },
                &response,
            )
            .await
        }
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        let endpoint = self.api.endpoint("phone_numbers")?;
        let response = self
            .api
            .client
            .get(endpoint)
            .bearer_auth(self.api.config.api_key.expose_secret())
            .send()
            .await
            .map_err(|error| {
                send_error(format!(
                    "Linq health request failed: {}",
                    error.without_url()
                ))
            })?;
        if !response.status().is_success() {
            return Err(send_error(format!(
                "Linq health request returned status {}",
                response.status()
            )));
        }
        let body: Value = crate::response::bounded_json(response, MAX_API_RESPONSE_BYTES)
            .await
            .map_err(|_| send_error("Linq health response is invalid"))?;
        let numbers = body
            .get("phone_numbers")
            .and_then(Value::as_array)
            .filter(|numbers| numbers.len() <= 4096)
            .ok_or_else(|| send_error("Linq health response omitted phone_numbers"))?;
        let mut configured_found = false;
        let mut deliverable_found = false;
        for number in numbers {
            let Some(line) = phone_number_handle(number) else {
                continue;
            };
            let health = phone_number_health(number);
            self.api
                .durable
                .update_line_health(line, health)
                .await
                .map_err(|error| send_error(format!("line health persistence failed: {error}")))?;
            configured_found |= line == self.api.config.from_number;
            deliverable_found |= !health.blocks_delivery();
        }
        if !configured_found {
            Err(send_error(
                "configured Linq sender is not assigned to the authenticated partner",
            ))
        } else if !deliverable_found {
            Err(send_error("no healthy Linq sender is available"))
        } else {
            Ok(())
        }
    }

    async fn diagnostics(&self) -> Option<Value> {
        let stats = self.api.durable.stats().await;
        Some(json!({
            "api_origin": redacted_origin(&self.api.config.api_base_url),
            "preferred_service": self.api.config.preferred_service.as_str(),
            "webhook_path": self.api.config.webhook_path,
            "allow_from_count": self.api.config.allow_from.len(),
            "pending_inbox_count": stats.pending_inbox,
            "deduplicated_event_count": stats.completed_inbox,
            "pending_outbox_count": stats.pending_outbox,
            "completed_outbox_count": stats.completed_outbox,
            "opted_out_recipient_count": stats.opted_out_recipients,
            "blocked_chat_count": stats.blocked_chats,
            "blocked_line_count": stats.blocked_lines,
            "webhook_version": LINQ_WEBHOOK_VERSION,
        }))
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(
            "Linq messages render as conversational plain text. Avoid tables and heavy markdown; keep replies compact."
                .to_string(),
        )
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        if let Some(worker) = self.worker.lock().await.take() {
            worker.abort();
            let _ = worker.await;
        }
        self.incoming_tx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(())
    }
}

async fn webhook(State(state): State<WebhookState>, headers: HeaderMap, body: Bytes) -> Response {
    if verify_standard_webhook(&state.secret, &headers, &body, Utc::now().timestamp()).is_err() {
        return (StatusCode::UNAUTHORIZED, "invalid webhook signature").into_response();
    }
    let document: Value = match serde_json::from_slice(&body) {
        Ok(document) => document,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid webhook payload").into_response(),
    };
    let event_id = match envelope_event_id(&document, &headers) {
        Ok(event_id) => event_id,
        Err(()) => return (StatusCode::BAD_REQUEST, "invalid webhook envelope").into_response(),
    };
    if validate_webhook_envelope(&document).is_err() {
        return (StatusCode::BAD_REQUEST, "unsupported webhook envelope").into_response();
    }
    match state.durable.accept_inbox(&event_id, document).await {
        Ok(AcceptResult::Accepted) => state.wake_worker.notify_one(),
        Ok(AcceptResult::AlreadyAccepted) => {}
        Err(error) if error == "inbox_capacity" => {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
    StatusCode::OK.into_response()
}

async fn run_durable_worker(
    api: LinqApi,
    sender: Arc<StdMutex<Option<mpsc::Sender<IncomingMessage>>>>,
    wake_worker: Arc<Notify>,
) {
    loop {
        let mut progressed = false;
        while let Ok(Some(work)) = api.durable.claim_inbox().await {
            progressed = true;
            match process_inbox_work(&api, &sender, &work).await {
                Ok(()) => {
                    if let Err(error) = api.durable.complete_inbox(&work.event_id).await {
                        tracing::error!(error, "Linq durable inbox acknowledgement failed");
                        break;
                    }
                }
                Err(error_code) => {
                    tracing::warn!(event_id = %work.event_id, error_code, "Linq durable inbox work will retry");
                    if api
                        .durable
                        .retry_inbox(&work.event_id, error_code)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        while let Ok(Some(work)) = api.durable.claim_next_outbox().await {
            progressed = true;
            match deliver_outbox_work(&api, &work).await {
                Ok(()) => {
                    if api
                        .durable
                        .complete_outbox(&work.operation_id)
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) if is_terminal_delivery_error(&error) => {
                    if is_opt_out_error(&error) {
                        let (chat, recipient) = target_identity(&work.target);
                        let _ = api.durable.mark_opted_out(chat, recipient).await;
                    }
                    let _ = api.durable.complete_outbox(&work.operation_id).await;
                }
                Err(error) => {
                    let _ = api
                        .durable
                        .retry_outbox(&work.operation_id, delivery_error_code(&error))
                        .await;
                }
            }
        }
        if !progressed {
            tokio::select! {
                () = wake_worker.notified() => {},
                () = tokio::time::sleep(Duration::from_secs(1)) => {},
            }
        }
    }
}

async fn process_inbox_work(
    api: &LinqApi,
    sender: &Arc<StdMutex<Option<mpsc::Sender<IncomingMessage>>>>,
    work: &InboxWork,
) -> Result<(), &'static str> {
    let disposition = parse_incoming(api, &work.document, &work.event_id)
        .await
        .map_err(|status| {
            if status.is_server_error() {
                "inbound_provider_io"
            } else {
                "inbound_invalid_payload"
            }
        })?;
    let Some(message) = disposition else {
        return Ok(());
    };
    let sender = sender
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .ok_or("inbound_channel_stopped")?;
    tokio::time::timeout(Duration::from_secs(5), sender.send(message))
        .await
        .map_err(|_| "inbound_channel_backpressure")?
        .map_err(|_| "inbound_channel_stopped")
}

async fn deliver_outbox_work(api: &LinqApi, work: &OutboxWork) -> Result<(), ChannelError> {
    if let Some(reason) = api.durable.delivery_block_reason(&work.target).await {
        return Err(send_error(reason));
    }
    let operation_id = uuid::Uuid::parse_str(&work.operation_id)
        .map_err(|_| send_error("outbox_operation_id_invalid"))?;
    let response = work
        .response
        .to_response(operation_id)
        .map_err(send_error)?;
    match &work.target {
        OutboundTarget::ExistingChat {
            chat_id, reply_to, ..
        } => {
            api.send_existing_chat(chat_id, reply_to.as_deref(), &response, &work.operation_id)
                .await
        }
        OutboundTarget::PooledRecipient { recipient } => {
            api.send_pooled(recipient, &response, &work.operation_id).await
        }
    }
}

fn validate_webhook_envelope(document: &Value) -> Result<(), ()> {
    if document.get("api_version").and_then(Value::as_str) != Some("v3")
        || document.get("webhook_version").and_then(Value::as_str)
            != Some(LINQ_WEBHOOK_VERSION)
        || document
            .get("event_type")
            .and_then(Value::as_str)
            .is_none_or(|event_type| {
                !matches!(event_type, "message.received" | "phone_number.status_updated")
            })
    {
        return Err(());
    }
    Ok(())
}

fn provider_error_code(body: &[u8]) -> Option<i64> {
    let document: Value = serde_json::from_slice(body).ok()?;
    document
        .get("error")
        .and_then(|error| error.get("code"))
        .or_else(|| document.get("code"))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
}

fn target_identity(target: &OutboundTarget) -> (Option<&str>, Option<&str>) {
    match target {
        OutboundTarget::ExistingChat {
            chat_id,
            recipient,
            ..
        } => (Some(chat_id), recipient.as_deref()),
        OutboundTarget::PooledRecipient { recipient } => (None, Some(recipient)),
    }
}

fn delivery_error_reason(error: &ChannelError) -> &str {
    match error {
        ChannelError::SendFailed { reason, .. } => reason,
        _ => "delivery_failed",
    }
}

fn delivery_error_code(error: &ChannelError) -> &str {
    let reason = delivery_error_reason(error);
    if reason.contains("opted_out") {
        "recipient_opted_out"
    } else if reason.contains("blocked") || reason.contains("unhealthy") {
        "delivery_policy_blocked"
    } else if reason.contains("rejected") {
        "provider_rejected"
    } else if reason.contains("attachment") {
        "attachment_delivery_failed"
    } else {
        "provider_retryable"
    }
}

fn is_opt_out_error(error: &ChannelError) -> bool {
    delivery_error_reason(error).contains("opted_out")
}

fn is_terminal_delivery_error(error: &ChannelError) -> bool {
    let reason = delivery_error_reason(error);
    reason.contains("opted_out")
        || reason.contains("blocked")
        || reason.contains("unhealthy")
        || reason.contains("rejected")
        || reason.contains("invalid")
        || reason.contains("unsupported")
        || reason.contains("too many")
        || reason.contains("exceeds")
        || reason.contains("empty")
}

fn is_opt_out_message(content: &str) -> bool {
    let trimmed = content.trim();
    if matches!(
        trimmed,
        "STOP" | "UNSUBSCRIBE" | "OPTOUT" | "CANCEL" | "END" | "QUIT"
    ) {
        return true;
    }
    let collapsed = trimmed
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != '-')
        .flat_map(char::to_uppercase)
        .collect::<String>();
    if collapsed == "OPTOUT" {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    [
        "stop messaging me",
        "stop texting me",
        "do not message me",
        "don't message me",
        "do not text me",
        "don't text me",
        "remove me from",
        "leave me alone",
        "no more messages",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn phone_number_handle(number: &Value) -> Option<&str> {
    number
        .get("phone_number")
        .and_then(Value::as_str)
        .or_else(|| number.get("number").and_then(Value::as_str))
        .filter(|line| valid_e164(line))
}

fn phone_number_health(number: &Value) -> DeliveryHealth {
    let value = number
        .get("reputation")
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("status").and_then(Value::as_str))
        })
        .or_else(|| {
            number.get("health_status").and_then(|value| {
                value
                    .as_str()
                    .or_else(|| value.get("status").and_then(Value::as_str))
            })
        })
        .or_else(|| number.get("status").and_then(Value::as_str));
    DeliveryHealth::parse(value)
}

async fn parse_incoming(
    api: &LinqApi,
    document: &Value,
    event_id: &str,
) -> Result<Option<IncomingMessage>, StatusCode> {
    let root = document.as_object().ok_or(StatusCode::BAD_REQUEST)?;
    if root.get("api_version").and_then(Value::as_str) != Some("v3")
        || root.get("webhook_version").and_then(Value::as_str) != Some(LINQ_WEBHOOK_VERSION)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let event_type = root.get("event_type").and_then(Value::as_str);
    if event_type == Some("phone_number.status_updated") {
        return process_phone_status(api, root).await;
    }
    if event_type != Some("message.received") {
        return Ok(None);
    }
    let data = root
        .get("data")
        .and_then(Value::as_object)
        .ok_or(StatusCode::BAD_REQUEST)?;
    if data.get("direction").and_then(Value::as_str) != Some("inbound") {
        return Ok(None);
    }
    let chat = data
        .get("chat")
        .and_then(Value::as_object)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let chat_id = object_string(chat, "id", 128)?;
    let owner = chat
        .get("owner_handle")
        .and_then(Value::as_object)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let owner_id = object_string(owner, "id", 128)?;
    let owner_handle = object_string(owner, "handle", 320)?;
    if owner.get("is_me").and_then(Value::as_bool) != Some(true)
        || uuid::Uuid::parse_str(&owner_id).is_err()
        || !valid_e164(&owner_handle)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if owner_handle != api.config.from_number {
        // A partner subscription can cover more than one managed line. A
        // valid event for another line is acknowledged but never crosses the
        // configured channel boundary.
        return Ok(None);
    }
    let chat_health = DeliveryHealth::parse(
        chat.get("health_status")
            .and_then(Value::as_object)
            .and_then(|health| health.get("status"))
            .and_then(Value::as_str),
    );
    let message_id = object_string(data, "id", 128)?;
    let sender = data
        .get("sender_handle")
        .and_then(Value::as_object)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let sender_id = object_string(sender, "id", 128)?;
    let handle = object_string(sender, "handle", 320)?;
    if sender.get("is_me").and_then(Value::as_bool) != Some(false)
        || uuid::Uuid::parse_str(&chat_id).is_err()
        || uuid::Uuid::parse_str(&message_id).is_err()
        || uuid::Uuid::parse_str(&sender_id).is_err()
        || !valid_external_handle(&handle)
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !allowed_sender(&api.config.allow_from, &handle, &sender_id) {
        return Ok(None);
    }
    // Actor endpoints must remain directly deliverable: the identity registry
    // later passes this value back to `broadcast`. Linq's opaque handle UUID is
    // useful provenance but cannot address a new outbound chat.
    let canonical_handle = if valid_e164(&handle) {
        handle.clone()
    } else {
        handle.to_ascii_lowercase()
    };
    let parts = data
        .get("parts")
        .and_then(Value::as_array)
        .ok_or(StatusCode::BAD_REQUEST)?;
    if parts.len() > 100 {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let mut texts = Vec::new();
    let mut media = Vec::new();
    for part in parts {
        let part = part.as_object().ok_or(StatusCode::BAD_REQUEST)?;
        match part.get("type").and_then(Value::as_str) {
            Some("text") => {
                let value = object_string(part, "value", MAX_INBOUND_TEXT_BYTES)?;
                texts.push(value);
            }
            Some("media") => {
                if media.len() >= MAX_ATTACHMENTS {
                    return Err(StatusCode::PAYLOAD_TOO_LARGE);
                }
                media.push(MediaDescriptor {
                    url: object_string(part, "url", 8192)?,
                    mime_type: object_string(part, "mime_type", 128)?,
                    filename: part
                        .get("filename")
                        .and_then(Value::as_str)
                        .filter(|value| value.len() <= 255 && !value.chars().any(char::is_control))
                        .unwrap_or("attachment")
                        .to_string(),
                    declared_size: part.get("size_bytes").and_then(Value::as_u64),
                });
            }
            Some(_) | None => {}
        }
    }
    let content = texts.join("\n");
    if content.len() > MAX_INBOUND_TEXT_BYTES {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let opted_out = is_opt_out_message(&content);
    api.durable
        .observe_inbound(
            &chat_id,
            &canonical_handle,
            &owner_handle,
            chat_health,
            opted_out,
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if opted_out
        || chat_health == DeliveryHealth::OptedOut
        || data.get("reconciled_at").is_some_and(|value| !value.is_null())
    {
        return Ok(None);
    }
    let attachments = download_media(media).await?;
    if content.is_empty() && attachments.is_empty() {
        return Ok(None);
    }
    let sent_at = data
        .get("sent_at")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
        .or_else(|| {
            root.get("created_at")
                .and_then(Value::as_str)
                .and_then(parse_timestamp)
        })
        .unwrap_or_else(Utc::now);
    let service = data
        .get("service")
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "iMessage" | "RCS" | "SMS"))
        .unwrap_or("unknown");
    let is_group = chat
        .get("is_group")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut incoming = crate::manager::normalize_incoming_event(crate::manager::IncomingEvent {
        platform: NAME.to_string(),
        chat_type: if is_group { "group" } else { "direct" }.to_string(),
        chat_id: chat_id.clone(),
        user_id: canonical_handle,
        user_name: Some(handle.clone()),
        text: content,
        metadata: json!({
            "external_event_id": event_id,
            "linq_chat_id": chat_id,
            "linq_message_id": message_id,
            "linq_handle_id": sender_id,
            "sender_handle": handle,
            "service": service,
            "is_group": is_group,
            "trace_id": root.get("trace_id").and_then(Value::as_str).filter(|v| v.len() <= 128),
        }),
    })
    .with_attachments(attachments);
    incoming.id = uuid::Uuid::parse_str(&message_id).map_err(|_| StatusCode::BAD_REQUEST)?;
    incoming.received_at = sent_at;
    Ok(Some(incoming))
}

async fn process_phone_status(
    api: &LinqApi,
    root: &Map<String, Value>,
) -> Result<Option<IncomingMessage>, StatusCode> {
    let data = root
        .get("data")
        .and_then(Value::as_object)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let line = data
        .get("phone_number")
        .and_then(Value::as_str)
        .or_else(|| data.get("number").and_then(Value::as_str))
        .filter(|line| valid_e164(line))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let health = DeliveryHealth::parse(
        data.get("reputation")
            .and_then(Value::as_str)
            .or_else(|| data.get("status").and_then(Value::as_str)),
    );
    api.durable
        .update_line_health(line, health)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(None)
}

#[derive(Debug)]
struct MediaDescriptor {
    url: String,
    mime_type: String,
    filename: String,
    declared_size: Option<u64>,
}

async fn download_media(
    descriptors: Vec<MediaDescriptor>,
) -> Result<Vec<MediaContent>, StatusCode> {
    let mut attachments = Vec::with_capacity(descriptors.len());
    let mut total = 0usize;
    for descriptor in descriptors {
        if descriptor
            .declared_size
            .is_some_and(|size| size > MAX_ATTACHMENT_BYTES as u64)
            || !supported_mime(&descriptor.mime_type)
        {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        let url = Url::parse(&descriptor.url).map_err(|_| StatusCode::BAD_REQUEST)?;
        if url.scheme() != "https"
            || url.host_str() != Some("cdn.linqapp.com")
            || url.port_or_known_default() != Some(443)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(StatusCode::BAD_REQUEST);
        }
        let client = pinned_transfer_client(&url, &["cdn.linqapp.com"])
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let response = client
            .get(url)
            .send()
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        if !response.status().is_success() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let bytes = crate::response::bounded_bytes(response, MAX_ATTACHMENT_BYTES)
            .await
            .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
        total = total
            .checked_add(bytes.len())
            .filter(|value| *value <= MAX_TOTAL_ATTACHMENT_BYTES)
            .ok_or(StatusCode::PAYLOAD_TOO_LARGE)?;
        attachments.push(
            MediaContent::new(bytes.to_vec(), descriptor.mime_type)
                .with_filename(descriptor.filename),
        );
    }
    Ok(attachments)
}

fn envelope_event_id(document: &Value, headers: &HeaderMap) -> Result<String, ()> {
    let event_id = document
        .get("event_id")
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 128)
        .ok_or(())?;
    uuid::Uuid::parse_str(event_id).map_err(|_| ())?;
    let header_id = headers
        .get("webhook-id")
        .and_then(|value| value.to_str().ok())
        .ok_or(())?;
    if header_id != event_id {
        return Err(());
    }
    Ok(event_id.to_string())
}

fn verify_standard_webhook(
    secret: &SecretString,
    headers: &HeaderMap,
    body: &[u8],
    now: i64,
) -> Result<(), ()> {
    let message_id = headers
        .get("webhook-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 128)
        .ok_or(())?;
    let timestamp = headers
        .get("webhook-timestamp")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 32)
        .ok_or(())?;
    let parsed_timestamp = timestamp.parse::<i64>().map_err(|_| ())?;
    if now.saturating_sub(parsed_timestamp).unsigned_abs() > MAX_WEBHOOK_SKEW_SECS as u64 {
        return Err(());
    }
    let signatures = headers
        .get("webhook-signature")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 8192)
        .ok_or(())?;
    let key = decode_webhook_key(secret.expose_secret())?;
    let mut signed = Vec::with_capacity(message_id.len() + timestamp.len() + body.len() + 2);
    signed.extend_from_slice(message_id.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(timestamp.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(body);
    for signature in signatures.split_whitespace() {
        let Some(encoded) = signature.strip_prefix("v1,") else {
            continue;
        };
        let Ok(candidate) = BASE64.decode(encoded) else {
            continue;
        };
        let mut mac = HmacSha256::new_from_slice(&key).map_err(|_| ())?;
        mac.update(&signed);
        if mac.verify_slice(&candidate).is_ok() {
            return Ok(());
        }
    }
    Err(())
}

fn decode_webhook_key(secret: &str) -> Result<Vec<u8>, ()> {
    let encoded = secret.trim().strip_prefix("whsec_").ok_or(())?;
    let key = BASE64.decode(encoded).map_err(|_| ())?;
    if !(16..=256).contains(&key.len()) {
        return Err(());
    }
    Ok(key)
}

fn allowed_sender(allow_from: &[String], handle: &str, id: &str) -> bool {
    allow_from.iter().any(|allowed| {
        allowed == "*" || allowed.eq_ignore_ascii_case(handle) || allowed.eq_ignore_ascii_case(id)
    })
}

fn valid_allow_entry(value: &str) -> bool {
    value == "*" || uuid::Uuid::parse_str(value).is_ok() || valid_external_handle(value)
}

fn valid_external_handle(value: &str) -> bool {
    valid_e164(value)
        || (value.len() <= 320
            && value.contains('@')
            && !value.chars().any(char::is_control)
            && !value.chars().any(char::is_whitespace))
}

fn valid_e164(value: &str) -> bool {
    let Some(digits) = value.strip_prefix('+') else {
        return false;
    };
    (7..=15).contains(&digits.len())
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn object_string(
    object: &Map<String, Value>,
    key: &str,
    max_len: usize,
) -> Result<String, StatusCode> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= max_len)
        .filter(|value| !value.chars().any(char::is_control))
        .map(ToString::to_string)
        .ok_or(StatusCode::BAD_REQUEST)
}

fn bounded_string(value: &Value, key: &str, max: usize) -> Result<String, ChannelError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= max)
        .map(ToString::to_string)
        .ok_or_else(|| send_error(format!("Linq API response omitted {key}")))
}

fn validate_uuid_component(value: &str, label: &str) -> Result<(), ChannelError> {
    uuid::Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| send_error(format!("Linq {label} is invalid")))
}

fn linq_chat_id_from_thread(thread_id: &str) -> Option<String> {
    let candidate = thread_id
        .strip_prefix("agent:main:linq:direct:")
        .or_else(|| thread_id.strip_prefix("agent:main:linq:group:"))
        .unwrap_or(thread_id);
    uuid::Uuid::parse_str(candidate)
        .ok()
        .map(|value| value.to_string())
}

fn validate_api_base_url(url: &Url) -> Result<(), ChannelError> {
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with('/')
    {
        return Err(startup_error(
            "API base URL contains forbidden components or lacks a trailing slash",
        ));
    }
    if url.as_str() == DEFAULT_LINQ_API_BASE_URL {
        return Ok(());
    }
    #[cfg(test)]
    if url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"))
    {
        return Ok(());
    }
    Err(startup_error(
        "API base URL must be the fixed official Linq Partner v3 endpoint",
    ))
}

fn validate_upload_url(url: &Url) -> Result<(), ChannelError> {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(send_error("Linq upload URL contains forbidden components"));
    }
    // Linq documents the presigned upload URL as opaque and notes that its
    // hostname can vary by partner configuration. The async transfer path
    // therefore accepts any public HTTPS host only after DNS validation and
    // pins the request to those validated addresses.
    if url.scheme() == "https"
        && url.host_str().is_some()
        && url.port_or_known_default() == Some(443)
    {
        return Ok(());
    }
    #[cfg(test)]
    if url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"))
    {
        return Ok(());
    }
    Err(send_error("Linq upload URL must use public HTTPS"))
}

async fn pinned_transfer_client(url: &Url, allowlist: &[&str]) -> Result<Client, ChannelError> {
    #[cfg(test)]
    if url.scheme() == "http"
        && url
            .host_str()
            .is_some_and(|host| matches!(host, "127.0.0.1" | "localhost" | "::1"))
    {
        return Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| send_error("Linq transfer client initialization failed"));
    }

    let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
        url.as_str(),
        &thinclaw_tools_core::OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: false,
            allowlist: allowlist.iter().map(|host| (*host).to_string()).collect(),
        },
    )
    .await
    .map_err(|_| send_error("Linq transfer URL failed outbound network policy"))?;
    let host = guarded
        .url
        .host_str()
        .ok_or_else(|| send_error("Linq transfer URL omitted its host"))?;
    let mut builder = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !guarded.pinned_addrs.is_empty() {
        builder = builder.resolve_to_addrs(host, &guarded.pinned_addrs);
    }
    builder
        .build()
        .map_err(|_| send_error("Linq DNS-pinned transfer client initialization failed"))
}

fn validate_attachment(attachment: &MediaContent) -> Result<(), ChannelError> {
    if attachment.data.is_empty() || attachment.data.len() > MAX_ATTACHMENT_BYTES {
        return Err(send_error("Linq attachment is empty or oversized"));
    }
    if !supported_mime(&attachment.mime_type) {
        return Err(send_error("Linq attachment MIME type is unsupported"));
    }
    if attachment.filename.as_deref().is_some_and(|filename| {
        filename.is_empty() || filename.len() > 255 || filename.chars().any(char::is_control)
    }) {
        return Err(send_error("Linq attachment filename is invalid"));
    }
    Ok(())
}

fn validate_outgoing_shape(response: &OutgoingResponse) -> Result<(), ChannelError> {
    if response.content.len() > MAX_INBOUND_TEXT_BYTES {
        return Err(send_error(
            "Linq outbound text exceeds the total size limit",
        ));
    }
    if response.attachments.len() > MAX_ATTACHMENTS {
        return Err(send_error("too many Linq attachments"));
    }
    let total = response
        .attachments
        .iter()
        .try_fold(0usize, |total, attachment| {
            total
                .checked_add(attachment.data.len())
                .ok_or_else(|| send_error("Linq attachment size overflow"))
        })?;
    if total > MAX_TOTAL_ATTACHMENT_BYTES {
        return Err(send_error("Linq attachments exceed the total size limit"));
    }
    if response.content.is_empty() && response.attachments.is_empty() {
        return Err(send_error("Linq message has no text or media"));
    }
    Ok(())
}

fn supported_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg"
            | "image/png"
            | "image/gif"
            | "image/heic"
            | "image/heif"
            | "image/tiff"
            | "image/bmp"
            | "video/mp4"
            | "video/quicktime"
            | "video/mpeg"
            | "video/x-m4v"
            | "video/x-msvideo"
            | "video/3gpp"
            | "audio/mpeg"
            | "audio/x-m4a"
            | "audio/x-caf"
            | "audio/x-wav"
            | "audio/x-aiff"
            | "audio/aac"
            | "audio/midi"
            | "audio/amr"
            | "application/pdf"
            | "application/vnd.apple.pkpass"
            | "text/plain"
            | "text/markdown"
            | "text/vcard"
            | "text/rtf"
            | "text/csv"
            | "text/calendar"
            | "application/msword"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.ms-excel"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-powerpoint"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            | "application/zip"
    )
}

fn idempotency_key(seed: &str, index: usize) -> String {
    let mut digest = Sha256::new();
    digest.update(seed.as_bytes());
    digest.update(index.to_le_bytes());
    format!("thinclaw-{}", hex::encode(digest.finalize()))
}

fn split_text(value: &str, limit: usize) -> Vec<&str> {
    if value.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (index, _) in value.char_indices() {
        if count == limit {
            chunks.push(&value[start..index]);
            start = index;
            count = 0;
        }
        count += 1;
    }
    chunks.push(&value[start..]);
    chunks
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn retry_delay(attempt: usize) -> Duration {
    #[cfg(test)]
    return Duration::from_millis((attempt as u64 + 1) * 5);
    #[cfg(not(test))]
    Duration::from_millis((attempt as u64 + 1) * 250)
}

fn retry_delay_from_response(response: &reqwest::Response, attempt: usize) -> Duration {
    let bounded = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(30)));
    bounded.unwrap_or_else(|| retry_delay(attempt))
}

fn redacted_origin(url: &Url) -> String {
    match (url.scheme(), url.host_str(), url.port()) {
        (scheme, Some(host), Some(port)) => format!("{scheme}://{host}:{port}"),
        (scheme, Some(host), None) => format!("{scheme}://{host}"),
        _ => "invalid".to_string(),
    }
}

fn redact_handle(value: &str) -> String {
    let suffix = value.chars().rev().take(2).collect::<Vec<_>>();
    let suffix = suffix.into_iter().rev().collect::<String>();
    format!("***{suffix}")
}

fn startup_error(reason: impl Into<String>) -> ChannelError {
    ChannelError::StartupFailed {
        name: NAME.to_string(),
        reason: reason.into(),
    }
}

fn send_error(reason: impl Into<String>) -> ChannelError {
    ChannelError::SendFailed {
        name: NAME.to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::body::Body;
    use axum::http::Request;
    use futures::StreamExt as _;
    use tempfile::TempDir;
    use tower::ServiceExt as _;

    use super::*;

    const EVENT_ID: &str = "2915e81c-5068-4796-ace2-21d2c94ad298";
    const CHAT_ID: &str = "8f392755-6865-4b18-880a-227f9d8b458f";
    const MESSAGE_ID: &str = "89e3566e-1d13-49e5-a8ee-48490d5bfeb7";
    const HANDLE_ID: &str = "e604375a-5913-483a-8278-c631e8f0ffda";

    fn secret() -> SecretString {
        SecretString::from(format!(
            "whsec_{}",
            BASE64.encode(b"0123456789abcdef0123456789abcdef")
        ))
    }

    fn config(temp: &TempDir, api: Url) -> LinqConfig {
        LinqConfig::new(
            api,
            SecretString::from("api-secret"),
            secret(),
            "+12025551234".to_string(),
            "127.0.0.1".to_string(),
            8080,
            "/webhook/linq".to_string(),
            vec!["+12025559876".to_string()],
            LinqPreferredService::IMessage,
            temp.path().join("linq-events.json"),
        )
        .unwrap()
    }

    fn payload() -> Vec<u8> {
        include_bytes!("../tests/fixtures/linq_message_received_v3.json").to_vec()
    }

    fn signed_request(body: Vec<u8>, timestamp: i64) -> Request<Body> {
        let mut mac = HmacSha256::new_from_slice(b"0123456789abcdef0123456789abcdef").unwrap();
        mac.update(EVENT_ID.as_bytes());
        mac.update(b".");
        mac.update(timestamp.to_string().as_bytes());
        mac.update(b".");
        mac.update(&body);
        let signature = BASE64.encode(mac.finalize().into_bytes());
        Request::post("/webhook/linq")
            .header("content-type", "application/json")
            .header("webhook-id", EVENT_ID)
            .header("webhook-timestamp", timestamp.to_string())
            .header("webhook-signature", format!("v1,{signature}"))
            .body(Body::from(body))
            .unwrap()
    }

    #[test]
    fn protocol_defaults_can_be_forced_without_silent_fallback() {
        assert_eq!(
            LinqPreferredService::parse("imessage").unwrap(),
            LinqPreferredService::IMessage
        );
        assert_eq!(
            LinqPreferredService::IMessage.as_api_value(),
            Some("iMessage")
        );
        assert_eq!(LinqPreferredService::Auto.as_api_value(), None);
        assert!(LinqPreferredService::parse("mystery").is_err());
        assert_eq!(
            linq_chat_id_from_thread(&format!("agent:main:linq:direct:{CHAT_ID}")),
            Some(CHAT_ID.to_string())
        );
    }

    #[test]
    fn secrets_are_redacted_from_debug() {
        let temp = TempDir::new().unwrap();
        let config = config(&temp, Url::parse("http://127.0.0.1:1/").unwrap());
        let debug = format!("{config:?}");
        assert!(!debug.contains("api-secret"));
        assert!(!debug.contains("whsec_"));
    }

    #[test]
    fn credential_and_listener_destinations_fail_closed() {
        let temp = TempDir::new().unwrap();
        let external = LinqConfig::new(
            Url::parse("https://attacker.example/api/partner/v3/").unwrap(),
            SecretString::from("api-secret"),
            secret(),
            "+12025551234".to_string(),
            "127.0.0.1".to_string(),
            8080,
            "/webhook/linq".to_string(),
            Vec::new(),
            LinqPreferredService::IMessage,
            temp.path().join("linq-events.json"),
        );
        assert!(external.is_err());

        let mut local = config(&temp, Url::parse("http://127.0.0.1:1/").unwrap());
        local.webhook_host = "localhost".to_string();
        assert!(
            LinqConfig::new(
                local.api_base_url,
                local.api_key,
                local.webhook_secret,
                local.from_number,
                local.webhook_host,
                local.webhook_port,
                local.webhook_path,
                local.allow_from,
                local.preferred_service,
                local.event_ledger_path,
            )
            .is_err()
        );
    }

    #[test]
    fn standard_webhook_rejects_tampering_and_replay() {
        let body = payload();
        let now = Utc::now().timestamp();
        let request = signed_request(body.clone(), now);
        assert!(verify_standard_webhook(&secret(), request.headers(), &body, now).is_ok());
        assert!(verify_standard_webhook(&secret(), request.headers(), b"tampered", now).is_err());
        assert!(verify_standard_webhook(&secret(), request.headers(), &body, now + 301).is_err());
    }

    #[tokio::test]
    async fn signed_inbound_maps_stable_identity_thread_and_deduplicates_durably() {
        let temp = TempDir::new().unwrap();
        let channel =
            LinqChannel::new(config(&temp, Url::parse("http://127.0.0.1:1/").unwrap())).unwrap();
        let mut stream = channel.start().await.unwrap();
        let app = channel.webhook_routes();
        let now = Utc::now().timestamp();
        let first = app
            .clone()
            .oneshot(signed_request(payload(), now))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let message = stream.next().await.unwrap();
        assert_eq!(message.id.to_string(), MESSAGE_ID);
        assert_eq!(message.user_id, "+12025559876");
        assert!(valid_external_handle(&message.user_id));
        assert_eq!(
            message.thread_id.as_deref().unwrap(),
            format!("agent:main:linq:direct:{CHAT_ID}")
        );
        assert_eq!(message.content, "Hello!");
        assert_eq!(
            message.metadata["raw"]["linq_chat_id"].as_str(),
            Some(CHAT_ID)
        );
        let identity = message.resolved_identity();
        assert_eq!(
            identity.principal_id,
            thinclaw_identity::external_principal_id(NAME, "+12025559876")
        );
        let duplicate = app.oneshot(signed_request(payload(), now)).await.unwrap();
        assert_eq!(duplicate.status(), StatusCode::OK);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.next())
                .await
                .is_err()
        );

        drop(channel);
        let reloaded =
            LinqChannel::new(config(&temp, Url::parse("http://127.0.0.1:1/").unwrap())).unwrap();
        let mut reloaded_stream = reloaded.start().await.unwrap();
        let response = reloaded
            .webhook_routes()
            .oneshot(signed_request(payload(), now))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), reloaded_stream.next())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unsigned_and_unlisted_senders_are_not_delivered() {
        let temp = TempDir::new().unwrap();
        let mut config = config(&temp, Url::parse("http://127.0.0.1:1/").unwrap());
        config.allow_from = vec!["+19995550123".to_string()];
        let channel = LinqChannel::new(config).unwrap();
        let mut stream = channel.start().await.unwrap();
        let app = channel.webhook_routes();
        let unsigned = Request::post("/webhook/linq")
            .body(Body::from(payload()))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(unsigned).await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
        let signed = signed_request(payload(), Utc::now().timestamp());
        assert_eq!(app.oneshot(signed).await.unwrap().status(), StatusCode::OK);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.next())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn signed_events_for_another_managed_line_are_ignored() {
        let temp = TempDir::new().unwrap();
        let channel =
            LinqChannel::new(config(&temp, Url::parse("http://127.0.0.1:1/").unwrap())).unwrap();
        let mut stream = channel.start().await.unwrap();
        let mut document: Value = serde_json::from_slice(&payload()).unwrap();
        document["data"]["chat"]["owner_handle"]["handle"] = json!("+12025550000");
        let body = serde_json::to_vec(&document).unwrap();
        let response = channel
            .webhook_routes()
            .oneshot(signed_request(body, Utc::now().timestamp()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.next())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn outbound_retry_reuses_idempotency_key_and_forces_imessage() {
        #[derive(Clone)]
        struct ApiState {
            attempts: Arc<AtomicUsize>,
            bodies: Arc<Mutex<Vec<Value>>>,
        }
        async fn send(State(state): State<ApiState>, body: axum::Json<Value>) -> Response {
            state.bodies.lock().await.push(body.0);
            let attempt = state.attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            } else {
                axum::Json(json!({"id": MESSAGE_ID})).into_response()
            }
        }
        let state = ApiState {
            attempts: Arc::new(AtomicUsize::new(0)),
            bodies: Arc::new(Mutex::new(Vec::new())),
        };
        let app = axum::Router::new()
            .route(
                &format!("/chats/{CHAT_ID}/messages"),
                axum::routing::post(send),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let temp = TempDir::new().unwrap();
        let channel = LinqChannel::new(config(
            &temp,
            Url::parse(&format!("http://{addr}/")).unwrap(),
        ))
        .unwrap();
        let mut incoming = IncomingMessage::new(NAME, HANDLE_ID, "hello").with_thread(CHAT_ID);
        incoming.id = uuid::Uuid::parse_str(MESSAGE_ID).unwrap();
        incoming.metadata = json!({"linq_chat_id": CHAT_ID, "linq_message_id": MESSAGE_ID});
        channel
            .respond(&incoming, OutgoingResponse::text("response"))
            .await
            .unwrap();
        assert_eq!(state.attempts.load(Ordering::SeqCst), 2);
        let bodies = state.bodies.lock().await;
        let first = &bodies[0]["message"];
        let second = &bodies[1]["message"];
        assert_eq!(first["idempotency_key"], second["idempotency_key"]);
        assert_eq!(first["preferred_service"], "iMessage");
        assert_eq!(first["reply_to"]["message_id"], MESSAGE_ID);
    }

    #[tokio::test]
    async fn outbound_media_uses_bounded_preupload_contract() {
        #[derive(Clone, Default)]
        struct MediaApiState {
            uploaded: Arc<Mutex<Vec<u8>>>,
            sent: Arc<Mutex<Vec<Value>>>,
            upload_attempts: Arc<AtomicUsize>,
        }
        async fn attachment(State(base): State<(MediaApiState, String)>) -> axum::Json<Value> {
            axum::Json(json!({
                "attachment_id": EVENT_ID,
                "upload_url": format!("{}/upload", base.1),
                "download_url": "https://cdn.linqapp.com/attachments/test",
                "http_method": "PUT",
                "expires_at": "2026-08-13T12:00:00Z",
                "required_headers": {
                    "Content-Type": "image/png",
                    "Content-Length": "4"
                }
            }))
        }
        async fn upload(
            State((state, _)): State<(MediaApiState, String)>,
            headers: HeaderMap,
            body: Bytes,
        ) -> StatusCode {
            assert_eq!(
                headers.get("content-type").and_then(|v| v.to_str().ok()),
                Some("image/png")
            );
            let attempt = state.upload_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                *state.uploaded.lock().await = body.to_vec();
                StatusCode::OK
            }
        }
        async fn send(
            State((state, _)): State<(MediaApiState, String)>,
            body: axum::Json<Value>,
        ) -> axum::Json<Value> {
            state.sent.lock().await.push(body.0);
            axum::Json(json!({"id": MESSAGE_ID}))
        }

        let state = MediaApiState::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let app_state = (state.clone(), base.clone());
        let app = axum::Router::new()
            .route("/attachments", axum::routing::post(attachment))
            .route("/upload", axum::routing::put(upload))
            .route(
                &format!("/chats/{CHAT_ID}/messages"),
                axum::routing::post(send),
            )
            .with_state(app_state);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let temp = TempDir::new().unwrap();
        let channel =
            LinqChannel::new(config(&temp, Url::parse(&format!("{base}/")).unwrap())).unwrap();
        let mut incoming = IncomingMessage::new(NAME, HANDLE_ID, "hello").with_thread(CHAT_ID);
        incoming.id = uuid::Uuid::parse_str(MESSAGE_ID).unwrap();
        incoming.metadata = json!({"linq_chat_id": CHAT_ID, "linq_message_id": MESSAGE_ID});
        channel
            .respond(
                &incoming,
                OutgoingResponse::text("caption").with_attachments(vec![
                    MediaContent::new(vec![1, 2, 3, 4], "image/png").with_filename("image.png"),
                ]),
            )
            .await
            .unwrap();

        assert_eq!(*state.uploaded.lock().await, [1, 2, 3, 4]);
        assert_eq!(state.upload_attempts.load(Ordering::SeqCst), 2);
        let sent = state.sent.lock().await;
        let parts = sent[0]["message"]["parts"].as_array().unwrap();
        assert_eq!(parts[0], json!({"type": "text", "value": "caption"}));
        assert_eq!(
            parts[1],
            json!({"type": "media", "attachment_id": EVENT_ID})
        );
    }
}
