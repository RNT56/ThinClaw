//! Native lifecycle channel primitives for provider-specific transports.
//!
//! These drivers are intentionally transport-neutral. Provider-specific clients
//! own Matrix sync, voice media webhooks, APNs HTTP/2, or Web Push delivery,
//! while this module owns ThinClaw channel semantics: shared ingress
//! normalization, session-key continuity, outbound routing, and diagnostics.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use subtle::ConstantTimeEq;
use tokio::sync::{Mutex, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::manager::{IncomingEvent, normalize_incoming_event};
use crate::native_lifecycle_clients::NativeEndpointRegistry;
use thinclaw_channels_core::{Channel, IncomingMessage, MessageStream, OutgoingResponse};
use thinclaw_types::error::ChannelError;

const MAX_NATIVE_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;
const MAX_NATIVE_WEBHOOK_CONCURRENCY: usize = 16;
const MAX_NATIVE_WEBHOOK_EVENTS: usize = 256;
const MAX_NATIVE_EVENT_TEXT_BYTES: usize = 256 * 1024;
const MAX_NATIVE_IDENTIFIER_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NativeLifecycleKind {
    Matrix,
    VoiceCall,
    Apns,
    BrowserPush,
}

impl NativeLifecycleKind {
    pub fn channel_name(self) -> &'static str {
        match self {
            Self::Matrix => "matrix",
            Self::VoiceCall => "voice-call",
            Self::Apns => "apns",
            Self::BrowserPush => "browser-push",
        }
    }

    pub fn default_chat_type(self) -> &'static str {
        match self {
            Self::Matrix => "room",
            Self::VoiceCall => "call",
            Self::Apns => "device",
            Self::BrowserPush => "subscription",
        }
    }

    pub fn formatting_hints(self) -> &'static str {
        match self {
            Self::Matrix => "Use concise Matrix-compatible Markdown. Avoid HTML-only formatting.",
            Self::VoiceCall => "Use short spoken sentences. Avoid tables and dense formatting.",
            Self::Apns => "Use brief push-notification text. Put the important action first.",
            Self::BrowserPush => "Use concise browser notification text with a clear action.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeOutboundMessage {
    pub channel: String,
    pub chat_type: String,
    pub chat_id: String,
    pub user_id: String,
    pub content: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait NativeLifecycleClient: Send + Sync {
    async fn validate(&self) -> Result<(), ChannelError>;

    async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError>;

    async fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

pub struct NativeLifecycleChannel {
    kind: NativeLifecycleKind,
    client: Arc<dyn NativeLifecycleClient>,
    tx: mpsc::Sender<IncomingMessage>,
    rx: Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
}

#[derive(Clone)]
pub struct NativeLifecycleIngress {
    kind: NativeLifecycleKind,
    tx: mpsc::Sender<IncomingMessage>,
}

impl NativeLifecycleIngress {
    pub async fn ingest_event(&self, event: NativeLifecycleEvent) -> Result<(), ChannelError> {
        if !valid_native_lifecycle_event(&event) {
            return Err(ChannelError::Disconnected {
                name: self.kind.channel_name().to_string(),
                reason: "native lifecycle event is malformed or oversized".to_string(),
            });
        }
        let message = normalize_incoming_event(event.into_incoming_event(self.kind));
        tokio::time::timeout(std::time::Duration::from_secs(5), self.tx.send(message))
            .await
            .map_err(|_| ChannelError::Disconnected {
                name: self.kind.channel_name().to_string(),
                reason: "native lifecycle event receiver is saturated".to_string(),
            })?
            .map_err(|_| ChannelError::Disconnected {
                name: self.kind.channel_name().to_string(),
                reason: "native lifecycle event receiver is closed".to_string(),
            })
    }
}

impl NativeLifecycleChannel {
    pub fn new(kind: NativeLifecycleKind, client: Arc<dyn NativeLifecycleClient>) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            kind,
            client,
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }

    pub fn matrix(client: Arc<dyn NativeLifecycleClient>) -> Self {
        Self::new(NativeLifecycleKind::Matrix, client)
    }

    pub fn voice_call(client: Arc<dyn NativeLifecycleClient>) -> Self {
        Self::new(NativeLifecycleKind::VoiceCall, client)
    }

    pub fn apns(client: Arc<dyn NativeLifecycleClient>) -> Self {
        Self::new(NativeLifecycleKind::Apns, client)
    }

    pub fn browser_push(client: Arc<dyn NativeLifecycleClient>) -> Self {
        Self::new(NativeLifecycleKind::BrowserPush, client)
    }

    pub fn ingress(&self) -> NativeLifecycleIngress {
        NativeLifecycleIngress {
            kind: self.kind,
            tx: self.tx.clone(),
        }
    }

    pub async fn ingest_event(&self, event: NativeLifecycleEvent) -> Result<(), ChannelError> {
        self.ingress().ingest_event(event).await
    }

    fn outbound_for(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> NativeOutboundMessage {
        let chat_type = msg
            .metadata
            .get("chat_type")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| self.kind.default_chat_type())
            .to_string();
        let chat_id = msg
            .metadata
            .get("chat_id")
            .and_then(|value| value.as_str())
            .or(msg.thread_id.as_deref())
            .unwrap_or(&msg.user_id)
            .to_string();

        NativeOutboundMessage {
            channel: self.kind.channel_name().to_string(),
            chat_type,
            chat_id,
            user_id: msg.user_id.clone(),
            content: response.content,
            metadata: response.metadata,
        }
    }
}

#[derive(Clone, Default)]
pub struct NativeLifecycleWebhookConfig {
    pub matrix: Option<NativeLifecycleIngress>,
    pub voice_call: Option<NativeLifecycleIngress>,
    pub browser_push: Option<NativeLifecycleIngress>,
    pub apns_registry: Option<NativeEndpointRegistry>,
    pub browser_push_registry: Option<NativeEndpointRegistry>,
    pub matrix_secret: Option<String>,
    pub voice_call_secret: Option<String>,
    pub apns_registration_secret: Option<String>,
    pub browser_push_secret: Option<String>,
}

pub fn native_lifecycle_webhook_routes(config: NativeLifecycleWebhookConfig) -> Router {
    let mut router = Router::new();
    if config.matrix.is_some() {
        router = router.route("/webhook/native/matrix", post(matrix_webhook_handler));
    }
    if config.voice_call.is_some() {
        router = router.route(
            "/webhook/native/voice-call",
            post(voice_call_webhook_handler),
        );
    }
    if config.browser_push.is_some() {
        router = router.route(
            "/webhook/native/browser-push",
            post(browser_push_webhook_handler),
        );
    }
    if config.apns_registry.is_some() {
        router = router.route(
            "/webhook/native/apns/register",
            post(apns_register_handler).delete(apns_unregister_handler),
        );
    }
    if config.browser_push_registry.is_some() {
        router = router.route(
            "/webhook/native/browser-push/register",
            post(browser_push_register_handler).delete(browser_push_unregister_handler),
        );
    }
    router
        .with_state(Arc::new(config))
        .layer(DefaultBodyLimit::max(MAX_NATIVE_WEBHOOK_BODY_BYTES))
        .layer(tower::limit::ConcurrencyLimitLayer::new(
            MAX_NATIVE_WEBHOOK_CONCURRENCY,
        ))
}

async fn matrix_webhook_handler(
    State(config): State<Arc<NativeLifecycleWebhookConfig>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(ingress) = config.matrix.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "matrix native lifecycle is not enabled",
        )
            .into_response();
    };
    // Reject forged room events when a secret is configured. Without this a
    // reachable `/webhook/native/matrix` lets anyone inject `m.room.message`
    // events as a spoofed trusted sender.
    if !header_secret_matches(&headers, "x-thinclaw-matrix-secret", &config.matrix_secret) {
        return (StatusCode::UNAUTHORIZED, "invalid matrix webhook secret").into_response();
    }
    let events = matrix_events_from_payload(&payload);
    if events.is_empty() {
        return (StatusCode::BAD_REQUEST, "no Matrix message events found").into_response();
    }
    for event in events {
        if let Err(error) = ingress.ingest_event(event).await {
            tracing::warn!(error = %error, "Matrix native lifecycle ingress unavailable");
            return (StatusCode::SERVICE_UNAVAILABLE, "ingress unavailable").into_response();
        }
    }
    (StatusCode::ACCEPTED, "accepted").into_response()
}

async fn voice_call_webhook_handler(
    State(config): State<Arc<NativeLifecycleWebhookConfig>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(ingress) = config.voice_call.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "voice-call native lifecycle is not enabled",
        )
            .into_response();
    };
    if !header_secret_matches(
        &headers,
        "x-thinclaw-voice-secret",
        &config.voice_call_secret,
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            "invalid voice-call webhook secret",
        )
            .into_response();
    }
    let Some(event) = voice_call_event_from_payload(&payload) else {
        return (
            StatusCode::BAD_REQUEST,
            "invalid voice-call webhook payload",
        )
            .into_response();
    };
    match ingress.ingest_event(event).await {
        Ok(()) => (StatusCode::ACCEPTED, "accepted").into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "Voice-call native lifecycle ingress unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, "ingress unavailable").into_response()
        }
    }
}

async fn browser_push_webhook_handler(
    State(config): State<Arc<NativeLifecycleWebhookConfig>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(ingress) = config.browser_push.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "browser-push native lifecycle is not enabled",
        )
            .into_response();
    };
    if !header_secret_matches(
        &headers,
        "x-thinclaw-browser-push-secret",
        &config.browser_push_secret,
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            "invalid browser-push webhook secret",
        )
            .into_response();
    }
    let Some(event) = browser_push_event_from_payload(&payload) else {
        return (
            StatusCode::BAD_REQUEST,
            "invalid browser-push webhook payload",
        )
            .into_response();
    };
    match ingress.ingest_event(event).await {
        Ok(()) => (StatusCode::ACCEPTED, "accepted").into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "Browser-push native lifecycle ingress unavailable");
            (StatusCode::SERVICE_UNAVAILABLE, "ingress unavailable").into_response()
        }
    }
}

async fn apns_register_handler(
    State(config): State<Arc<NativeLifecycleWebhookConfig>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(registry) = config.apns_registry.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "APNs native lifecycle registration is not enabled",
        )
            .into_response();
    };
    if !header_secret_matches_required(
        &headers,
        "x-thinclaw-apns-registration-secret",
        &config.apns_registration_secret,
    ) {
        return (StatusCode::UNAUTHORIZED, "invalid APNs registration secret").into_response();
    }
    let Some((user_id, device_token)) =
        registration_fields(&payload, &["device_token", "token", "apns_token"])
    else {
        return (StatusCode::BAD_REQUEST, "invalid APNs registration payload").into_response();
    };
    if !valid_native_identifier(&user_id)
        || device_token.len() > 512
        || device_token.chars().any(char::is_control)
    {
        return (StatusCode::BAD_REQUEST, "invalid APNs registration payload").into_response();
    }
    match registry.register(user_id, device_token).await {
        Ok(()) => (StatusCode::ACCEPTED, "registered").into_response(),
        Err(_) => (StatusCode::BAD_REQUEST, "registration rejected").into_response(),
    }
}

async fn apns_unregister_handler(
    State(config): State<Arc<NativeLifecycleWebhookConfig>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(registry) = config.apns_registry.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "APNs native lifecycle registration is not enabled",
        )
            .into_response();
    };
    if !header_secret_matches_required(
        &headers,
        "x-thinclaw-apns-registration-secret",
        &config.apns_registration_secret,
    ) {
        return (StatusCode::UNAUTHORIZED, "invalid APNs registration secret").into_response();
    }
    let Some((user_id, device_token)) =
        registration_fields(&payload, &["device_token", "token", "apns_token"])
    else {
        return (StatusCode::BAD_REQUEST, "invalid APNs registration payload").into_response();
    };
    if !valid_native_identifier(&user_id)
        || device_token.len() > 512
        || device_token.chars().any(char::is_control)
    {
        return (StatusCode::BAD_REQUEST, "invalid APNs registration payload").into_response();
    }
    if registry.unregister(&user_id, &device_token).await {
        (StatusCode::OK, "unregistered").into_response()
    } else {
        (StatusCode::NOT_FOUND, "registration not found").into_response()
    }
}

async fn browser_push_register_handler(
    State(config): State<Arc<NativeLifecycleWebhookConfig>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(registry) = config.browser_push_registry.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "browser-push native lifecycle registration is not enabled",
        )
            .into_response();
    };
    if !header_secret_matches_required(
        &headers,
        "x-thinclaw-browser-push-secret",
        &config.browser_push_secret,
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            "invalid browser-push webhook secret",
        )
            .into_response();
    }
    let Some((user_id, endpoint)) =
        registration_fields(&payload, &["endpoint", "subscription.endpoint", "chat_id"])
    else {
        return (
            StatusCode::BAD_REQUEST,
            "invalid browser-push registration payload",
        )
            .into_response();
    };
    if !valid_native_identifier(&user_id) || !valid_browser_push_endpoint(&endpoint) {
        return (
            StatusCode::BAD_REQUEST,
            "invalid browser-push registration payload",
        )
            .into_response();
    }
    match registry.register(user_id, endpoint).await {
        Ok(()) => (StatusCode::ACCEPTED, "registered").into_response(),
        Err(_) => (StatusCode::BAD_REQUEST, "registration rejected").into_response(),
    }
}

async fn browser_push_unregister_handler(
    State(config): State<Arc<NativeLifecycleWebhookConfig>>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> impl IntoResponse {
    let Some(registry) = config.browser_push_registry.as_ref() else {
        return (
            StatusCode::NOT_FOUND,
            "browser-push native lifecycle registration is not enabled",
        )
            .into_response();
    };
    if !header_secret_matches_required(
        &headers,
        "x-thinclaw-browser-push-secret",
        &config.browser_push_secret,
    ) {
        return (
            StatusCode::UNAUTHORIZED,
            "invalid browser-push webhook secret",
        )
            .into_response();
    }
    let Some((user_id, endpoint)) =
        registration_fields(&payload, &["endpoint", "subscription.endpoint", "chat_id"])
    else {
        return (
            StatusCode::BAD_REQUEST,
            "invalid browser-push registration payload",
        )
            .into_response();
    };
    if !valid_native_identifier(&user_id) || !valid_browser_push_endpoint(&endpoint) {
        return (
            StatusCode::BAD_REQUEST,
            "invalid browser-push registration payload",
        )
            .into_response();
    }
    if registry.unregister(&user_id, &endpoint).await {
        (StatusCode::OK, "unregistered").into_response()
    } else {
        (StatusCode::NOT_FOUND, "registration not found").into_response()
    }
}

fn header_secret_matches(headers: &HeaderMap, name: &str, expected: &Option<String>) -> bool {
    header_secret_matches_required(headers, name, expected)
}

fn header_secret_matches_required(
    headers: &HeaderMap,
    name: &str,
    expected: &Option<String>,
) -> bool {
    let Some(expected) = expected.as_ref().filter(|secret| !secret.is_empty()) else {
        return false;
    };
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|actual| bool::from(actual.as_bytes().ct_eq(expected.as_bytes())))
}

fn registration_fields(payload: &Value, endpoint_paths: &[&str]) -> Option<(String, String)> {
    let user_id = first_payload_string(payload, &["user_id", "device_id", "principal_id"])?;
    let endpoint = first_payload_string(payload, endpoint_paths)?;
    Some((user_id, endpoint))
}

pub fn matrix_events_from_payload(payload: &Value) -> Vec<NativeLifecycleEvent> {
    let mut events = Vec::new();
    if let Some(event) = matrix_event_from_payload(payload) {
        events.push(event);
    }
    if let Some(array) = payload.get("events").and_then(Value::as_array) {
        events.extend(array.iter().filter_map(matrix_event_from_payload));
    }
    if let Some(rooms) = payload.pointer("/rooms/join").and_then(Value::as_object) {
        for (room_id, room) in rooms {
            let Some(timeline) = room.pointer("/timeline/events").and_then(Value::as_array) else {
                continue;
            };
            for event in timeline {
                if let Some(mut parsed) = matrix_event_from_payload(event) {
                    if parsed.chat_id.is_empty() {
                        parsed.chat_id = room_id.clone();
                    }
                    events.push(parsed);
                }
            }
        }
    }
    events.truncate(MAX_NATIVE_WEBHOOK_EVENTS);
    events
}

fn matrix_event_from_payload(event: &Value) -> Option<NativeLifecycleEvent> {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("m.room.message");
    if event_type != "m.room.message" {
        return None;
    }
    let text = event
        .pointer("/content/body")
        .or_else(|| event.get("body"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    let chat_id = event
        .get("room_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let user_id = event
        .get("sender")
        .or_else(|| event.get("user_id"))
        .and_then(Value::as_str)?;
    if text.is_empty()
        || text.len() > MAX_NATIVE_EVENT_TEXT_BYTES
        || (!chat_id.is_empty() && !valid_native_identifier(chat_id))
        || !valid_native_identifier(user_id)
    {
        return None;
    }
    Some(NativeLifecycleEvent {
        chat_id: chat_id.to_string(),
        chat_type: Some("room".to_string()),
        user_id: user_id.to_string(),
        user_name: None,
        text,
        metadata: serde_json::json!({
            "event_id": event.get("event_id").and_then(Value::as_str),
            "origin_server_ts": event.get("origin_server_ts").and_then(Value::as_i64),
        }),
    })
}

pub fn voice_call_event_from_payload(payload: &Value) -> Option<NativeLifecycleEvent> {
    let text = first_payload_string(payload, &["text", "transcript", "speech"])?
        .trim()
        .to_string();
    if text.is_empty() || text.len() > MAX_NATIVE_EVENT_TEXT_BYTES {
        return None;
    }
    let call_id = first_payload_string(payload, &["call_id", "CallSid", "callSid"])?;
    let user_id = first_payload_string(payload, &["user_id", "from", "From", "caller"])
        .unwrap_or_else(|| call_id.clone());
    if !valid_native_identifier(&call_id) || !valid_native_identifier(&user_id) {
        return None;
    }
    Some(NativeLifecycleEvent {
        chat_id: call_id,
        chat_type: Some("call".to_string()),
        user_id,
        user_name: first_payload_string(payload, &["user_name", "caller_name", "CallerName"]),
        text,
        metadata: serde_json::json!({
            "provider": "voice-call",
            "call_state": first_payload_string(payload, &["state", "call_state", "CallStatus"]),
        }),
    })
}

pub fn browser_push_event_from_payload(payload: &Value) -> Option<NativeLifecycleEvent> {
    let text = first_payload_string(payload, &["text", "message", "action"])?
        .trim()
        .to_string();
    if text.is_empty() || text.len() > MAX_NATIVE_EVENT_TEXT_BYTES {
        return None;
    }
    let endpoint =
        first_payload_string(payload, &["endpoint", "subscription.endpoint", "chat_id"])?;
    let user_id = first_payload_string(payload, &["user_id", "device_id"])
        .unwrap_or_else(|| endpoint.clone());
    if !valid_browser_push_endpoint(&endpoint) || !valid_native_identifier(&user_id) {
        return None;
    }
    Some(NativeLifecycleEvent {
        chat_id: endpoint,
        chat_type: Some("subscription".to_string()),
        user_id,
        user_name: None,
        text,
        metadata: serde_json::json!({
            "provider": "browser-push",
            "action": first_payload_string(payload, &["action"]),
        }),
    })
}

fn first_payload_string(payload: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| value_at_path(payload, path))
}

fn value_at_path(payload: &Value, path: &str) -> Option<String> {
    let mut current = payload;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    match current {
        Value::String(value)
            if !value.trim().is_empty()
                && value.len() <= MAX_NATIVE_EVENT_TEXT_BYTES
                && !value.chars().any(|character| character == '\0') =>
        {
            Some(value.trim().to_string())
        }
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn valid_native_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NATIVE_IDENTIFIER_BYTES
        && !value.chars().any(char::is_control)
}

fn valid_browser_push_endpoint(value: &str) -> bool {
    if value.len() > 16 * 1024 {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_none_or(thinclaw_tools_core::is_public_outbound_ip)
}

fn valid_native_lifecycle_event(event: &NativeLifecycleEvent) -> bool {
    valid_native_identifier(&event.chat_id)
        && valid_native_identifier(&event.user_id)
        && event
            .chat_type
            .as_deref()
            .is_none_or(valid_native_identifier)
        && event
            .user_name
            .as_deref()
            .is_none_or(valid_native_identifier)
        && !event.text.trim().is_empty()
        && event.text.len() <= MAX_NATIVE_EVENT_TEXT_BYTES
        && serde_json::to_vec(&event.metadata)
            .is_ok_and(|encoded| encoded.len() <= MAX_NATIVE_WEBHOOK_BODY_BYTES)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeLifecycleEvent {
    pub chat_id: String,
    #[serde(default)]
    pub chat_type: Option<String>,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    pub text: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl NativeLifecycleEvent {
    pub fn into_incoming_event(self, kind: NativeLifecycleKind) -> IncomingEvent {
        IncomingEvent {
            platform: kind.channel_name().to_string(),
            chat_type: self
                .chat_type
                .unwrap_or_else(|| kind.default_chat_type().to_string()),
            chat_id: self.chat_id,
            user_id: self.user_id,
            user_name: self.user_name,
            text: self.text,
            metadata: self.metadata,
        }
    }
}

#[async_trait]
impl Channel for NativeLifecycleChannel {
    fn name(&self) -> &str {
        self.kind.channel_name()
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        self.client.validate().await?;
        let rx = self
            .rx
            .lock()
            .await
            .take()
            .ok_or_else(|| ChannelError::StartupFailed {
                name: self.kind.channel_name().to_string(),
                reason: "native lifecycle channel was already started".to_string(),
            })?;
        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let outbound = self.outbound_for(msg, response);
        self.client.send(outbound).await
    }

    async fn broadcast(
        &self,
        user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        self.client
            .send(NativeOutboundMessage {
                channel: self.kind.channel_name().to_string(),
                chat_type: self.kind.default_chat_type().to_string(),
                chat_id: user_id.to_string(),
                user_id: user_id.to_string(),
                content: response.content,
                metadata: response.metadata,
            })
            .await
    }

    fn formatting_hints(&self) -> Option<String> {
        Some(self.kind.formatting_hints().to_string())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        self.client.validate().await
    }

    async fn diagnostics(&self) -> Option<serde_json::Value> {
        Some(self.client.diagnostics().await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use futures::StreamExt;
    use tokio::sync::Mutex as TokioMutex;
    use tower::ServiceExt;

    #[derive(Default)]
    struct MockNativeClient {
        sent: TokioMutex<Vec<NativeOutboundMessage>>,
        fail_validate: bool,
    }

    #[async_trait]
    impl NativeLifecycleClient for MockNativeClient {
        async fn validate(&self) -> Result<(), ChannelError> {
            if self.fail_validate {
                Err(ChannelError::AuthFailed {
                    name: "native".to_string(),
                    reason: "mock validation failed".to_string(),
                })
            } else {
                Ok(())
            }
        }

        async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError> {
            self.sent.lock().await.push(message);
            Ok(())
        }

        async fn diagnostics(&self) -> serde_json::Value {
            serde_json::json!({"mock": true})
        }
    }

    #[tokio::test]
    async fn native_lifecycle_ingress_uses_shared_session_normalization() {
        let client = Arc::new(MockNativeClient::default());
        let channel = NativeLifecycleChannel::matrix(client);
        let mut stream = channel.start().await.expect("start should pass");

        channel
            .ingest_event(NativeLifecycleEvent {
                chat_id: "!room:example.org".to_string(),
                chat_type: None,
                user_id: "@user:example.org".to_string(),
                user_name: Some("User".to_string()),
                text: "/status now".to_string(),
                metadata: serde_json::json!({"event_id": "$1"}),
            })
            .await
            .expect("ingest should pass");

        let message = stream.next().await.expect("message should arrive");
        assert_eq!(message.channel, "matrix");
        assert_eq!(message.user_id, "@user:example.org");
        assert_eq!(message.user_name.as_deref(), Some("User"));
        assert_eq!(message.content, "/status now");
        assert_eq!(
            message.thread_id.as_deref(),
            Some("agent:main:matrix:room:!room_example.org")
        );
        assert_eq!(
            message
                .metadata
                .get("legacy_session_key_aliases")
                .and_then(|value| value.as_array())
                .map(Vec::len),
            Some(3)
        );
    }

    #[tokio::test]
    async fn native_lifecycle_respond_routes_to_original_conversation() {
        let client = Arc::new(MockNativeClient::default());
        let channel = NativeLifecycleChannel::browser_push(client.clone());
        let msg = NativeLifecycleEvent {
            chat_id: "endpoint-1".to_string(),
            chat_type: None,
            user_id: "device-user".to_string(),
            user_name: None,
            text: "hello".to_string(),
            metadata: serde_json::json!({}),
        }
        .into_incoming_event(NativeLifecycleKind::BrowserPush);
        let msg = normalize_incoming_event(msg);

        channel
            .respond(&msg, OutgoingResponse::text("reply"))
            .await
            .expect("respond should pass");

        let sent = client.sent.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].channel, "browser-push");
        assert_eq!(sent[0].chat_type, "subscription");
        assert_eq!(sent[0].chat_id, "endpoint-1");
        assert_eq!(sent[0].content, "reply");
    }

    #[tokio::test]
    async fn native_lifecycle_health_uses_client_validation() {
        let client = Arc::new(MockNativeClient {
            sent: TokioMutex::new(Vec::new()),
            fail_validate: true,
        });
        let channel = NativeLifecycleChannel::voice_call(client);

        let err = channel
            .health_check()
            .await
            .expect_err("validation failure should surface");
        assert!(err.to_string().contains("mock validation failed"));
    }

    #[tokio::test]
    async fn native_lifecycle_exposes_diagnostics_and_formatting_hints() {
        let channel = NativeLifecycleChannel::apns(Arc::new(MockNativeClient::default()));
        assert!(
            channel
                .formatting_hints()
                .unwrap()
                .contains("push-notification")
        );
        assert_eq!(
            channel
                .diagnostics()
                .await
                .and_then(|value| value.get("mock").and_then(|flag| flag.as_bool())),
            Some(true)
        );
    }

    #[test]
    fn native_lifecycle_parses_matrix_sync_batches() {
        let payload = serde_json::json!({
            "rooms": {
                "join": {
                    "!room:example.org": {
                        "timeline": {
                            "events": [
                                {
                                    "type": "m.room.message",
                                    "sender": "@alice:example.org",
                                    "event_id": "$event",
                                    "content": {"body": "hello"}
                                },
                                {
                                    "type": "m.reaction",
                                    "sender": "@alice:example.org",
                                    "content": {"body": "ignored"}
                                }
                            ]
                        }
                    }
                }
            }
        });

        let events = matrix_events_from_payload(&payload);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].chat_id, "!room:example.org");
        assert_eq!(events[0].user_id, "@alice:example.org");
        assert_eq!(events[0].text, "hello");
    }

    #[test]
    fn native_lifecycle_parses_voice_and_browser_payloads() {
        let voice = voice_call_event_from_payload(&serde_json::json!({
            "CallSid": "call-1",
            "From": "+15551234567",
            "transcript": "hello by phone",
            "CallStatus": "in-progress"
        }))
        .expect("voice event should parse");
        assert_eq!(voice.chat_id, "call-1");
        assert_eq!(voice.user_id, "+15551234567");
        assert_eq!(voice.text, "hello by phone");

        let browser = browser_push_event_from_payload(&serde_json::json!({
            "subscription": {"endpoint": "https://push.example/sub"},
            "user_id": "device-user",
            "action": "open-thread"
        }))
        .expect("browser event should parse");
        assert_eq!(browser.chat_id, "https://push.example/sub");
        assert_eq!(browser.user_id, "device-user");
        assert_eq!(browser.text, "open-thread");
    }

    #[tokio::test]
    async fn native_lifecycle_webhook_routes_emit_matrix_messages() {
        let channel = NativeLifecycleChannel::matrix(Arc::new(MockNativeClient::default()));
        let mut stream = channel.start().await.expect("start should pass");
        let app = native_lifecycle_webhook_routes(NativeLifecycleWebhookConfig {
            matrix: Some(channel.ingress()),
            matrix_secret: Some("matrix-secret".to_string()),
            ..Default::default()
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/matrix")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-matrix-secret", "matrix-secret")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "room_id": "!room:example.org",
                            "sender": "@alice:example.org",
                            "content": {"body": "hello Matrix"}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let message = stream.next().await.expect("message should arrive");
        assert_eq!(message.channel, "matrix");
        assert_eq!(message.user_id, "@alice:example.org");
        assert_eq!(message.content, "hello Matrix");
        assert_eq!(
            message.thread_id.as_deref(),
            Some("agent:main:matrix:room:!room_example.org")
        );
    }

    #[tokio::test]
    async fn native_lifecycle_webhook_routes_validate_voice_secret() {
        let channel = NativeLifecycleChannel::voice_call(Arc::new(MockNativeClient::default()));
        let mut stream = channel.start().await.expect("start should pass");
        let app = native_lifecycle_webhook_routes(NativeLifecycleWebhookConfig {
            voice_call: Some(channel.ingress()),
            voice_call_secret: Some("voice-secret".to_string()),
            ..Default::default()
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "call_id": "call-1",
            "user_id": "caller",
            "text": "hello"
        }))
        .unwrap();

        let rejected = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/voice-call")
                    .header("content-type", "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let accepted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/voice-call")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-voice-secret", "voice-secret")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);

        let message = stream.next().await.expect("message should arrive");
        assert_eq!(message.channel, "voice-call");
        assert_eq!(message.user_id, "caller");
        assert_eq!(message.content, "hello");
    }

    #[tokio::test]
    async fn native_lifecycle_webhook_routes_validate_matrix_secret() {
        let channel = NativeLifecycleChannel::matrix(Arc::new(MockNativeClient::default()));
        let mut stream = channel.start().await.expect("start should pass");
        let app = native_lifecycle_webhook_routes(NativeLifecycleWebhookConfig {
            matrix: Some(channel.ingress()),
            matrix_secret: Some("matrix-secret".to_string()),
            ..Default::default()
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "room_id": "!room:example.org",
            "sender": "@mallory:evil.example",
            "content": {"body": "forged"}
        }))
        .unwrap();

        // No secret header → rejected (previously accepted forged events).
        let rejected = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/matrix")
                    .header("content-type", "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        // Correct secret → accepted.
        let accepted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/matrix")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-matrix-secret", "matrix-secret")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);

        let message = stream.next().await.expect("message should arrive");
        assert_eq!(message.channel, "matrix");
        assert_eq!(message.user_id, "@mallory:evil.example");
    }

    #[tokio::test]
    async fn native_lifecycle_webhook_routes_emit_browser_push_actions() {
        let channel = NativeLifecycleChannel::browser_push(Arc::new(MockNativeClient::default()));
        let mut stream = channel.start().await.expect("start should pass");
        let app = native_lifecycle_webhook_routes(NativeLifecycleWebhookConfig {
            browser_push: Some(channel.ingress()),
            browser_push_secret: Some("push-secret".to_string()),
            ..Default::default()
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/browser-push")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-browser-push-secret", "push-secret")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "subscription": {"endpoint": "https://push.example/sub"},
                            "device_id": "device-1",
                            "message": "wake"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        let message = stream.next().await.expect("message should arrive");
        assert_eq!(message.channel, "browser-push");
        assert_eq!(message.user_id, "device-1");
        assert_eq!(message.content, "wake");
    }

    #[tokio::test]
    async fn native_lifecycle_webhook_routes_register_apns_device_tokens() {
        let registry = NativeEndpointRegistry::default();
        let app = native_lifecycle_webhook_routes(NativeLifecycleWebhookConfig {
            apns_registry: Some(registry.clone()),
            apns_registration_secret: Some("registration-secret".to_string()),
            ..Default::default()
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "user_id": "user-1",
            "device_token": "device-token-1"
        }))
        .unwrap();

        let rejected = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/apns/register")
                    .header("content-type", "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let accepted = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/apns/register")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-apns-registration-secret", "registration-secret")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(accepted.status(), StatusCode::ACCEPTED);
        assert_eq!(
            registry.endpoints_for("user-1").await,
            vec!["device-token-1".to_string()]
        );

        let removed = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/webhook/native/apns/register")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-apns-registration-secret", "registration-secret")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "user_id": "user-1",
                            "device_token": "device-token-1"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(removed.status(), StatusCode::OK);
        assert!(registry.endpoints_for("user-1").await.is_empty());
    }

    #[tokio::test]
    async fn native_lifecycle_webhook_routes_register_browser_push_subscriptions() {
        let registry = NativeEndpointRegistry::default();
        let app = native_lifecycle_webhook_routes(NativeLifecycleWebhookConfig {
            browser_push_registry: Some(registry.clone()),
            browser_push_secret: Some("push-secret".to_string()),
            ..Default::default()
        });

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/native/browser-push/register")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-browser-push-secret", "push-secret")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "user_id": "user-1",
                            "subscription": {"endpoint": "https://push.example/sub"}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert_eq!(
            registry.endpoints_for("user-1").await,
            vec!["https://push.example/sub".to_string()]
        );

        let removed = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/webhook/native/browser-push/register")
                    .header("content-type", "application/json")
                    .header("x-thinclaw-browser-push-secret", "push-secret")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "user_id": "user-1",
                            "subscription": {"endpoint": "https://push.example/sub"}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .expect("request should be served");
        assert_eq!(removed.status(), StatusCode::OK);
        assert!(registry.endpoints_for("user-1").await.is_empty());
    }

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
        headers
    }

    #[test]
    fn header_secret_matches_required_rejects_when_no_secret_configured() {
        let headers = headers_with("x-thinclaw-apns-registration-secret", "anything");
        assert!(!header_secret_matches_required(
            &headers,
            "x-thinclaw-apns-registration-secret",
            &None
        ));
        assert!(!header_secret_matches_required(
            &headers,
            "x-thinclaw-apns-registration-secret",
            &Some(String::new())
        ));
    }

    #[test]
    fn header_secret_matches_required_rejects_missing_header() {
        let headers = HeaderMap::new();
        assert!(!header_secret_matches_required(
            &headers,
            "x-thinclaw-apns-registration-secret",
            &Some("expected-secret".to_string())
        ));
    }

    #[test]
    fn header_secret_matches_required_rejects_mismatched_value() {
        let headers = headers_with("x-thinclaw-apns-registration-secret", "wrong-secret");
        assert!(!header_secret_matches_required(
            &headers,
            "x-thinclaw-apns-registration-secret",
            &Some("expected-secret".to_string())
        ));
    }

    #[test]
    fn header_secret_matches_required_rejects_mismatched_length() {
        let headers = headers_with("x-thinclaw-apns-registration-secret", "short");
        assert!(!header_secret_matches_required(
            &headers,
            "x-thinclaw-apns-registration-secret",
            &Some("a-much-longer-expected-secret".to_string())
        ));
    }

    #[test]
    fn header_secret_matches_required_accepts_exact_match() {
        let headers = headers_with("x-thinclaw-apns-registration-secret", "expected-secret");
        assert!(header_secret_matches_required(
            &headers,
            "x-thinclaw-apns-registration-secret",
            &Some("expected-secret".to_string())
        ));
    }

    #[test]
    fn header_secret_matches_rejects_when_no_secret_configured() {
        let headers = HeaderMap::new();
        assert!(!header_secret_matches(
            &headers,
            "x-thinclaw-browser-push-secret",
            &None
        ));
        assert!(!header_secret_matches(
            &headers,
            "x-thinclaw-browser-push-secret",
            &Some(String::new())
        ));
    }

    #[test]
    fn header_secret_matches_rejects_mismatched_or_missing_value() {
        let headers = HeaderMap::new();
        assert!(!header_secret_matches(
            &headers,
            "x-thinclaw-browser-push-secret",
            &Some("expected-secret".to_string())
        ));

        let headers = headers_with("x-thinclaw-browser-push-secret", "wrong-secret");
        assert!(!header_secret_matches(
            &headers,
            "x-thinclaw-browser-push-secret",
            &Some("expected-secret".to_string())
        ));
    }

    #[test]
    fn header_secret_matches_accepts_exact_match() {
        let headers = headers_with("x-thinclaw-browser-push-secret", "expected-secret");
        assert!(header_secret_matches(
            &headers,
            "x-thinclaw-browser-push-secret",
            &Some("expected-secret".to_string())
        ));
    }
}
