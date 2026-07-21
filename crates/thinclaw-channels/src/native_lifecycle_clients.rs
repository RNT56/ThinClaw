//! Provider-specific native lifecycle clients.
//!
//! These clients own provider HTTP request shapes while
//! [`crate::native_lifecycle::NativeLifecycleChannel`] owns ThinClaw channel
//! semantics. The HTTP transport is injectable so CI can exercise real provider
//! paths without live credentials.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thinclaw_types::error::ChannelError;
use tokio::sync::RwLock;
use url::Url;
use uuid::Uuid;

use crate::apns_push::{ApnsPushSpec, ApnsPusher};
use crate::native_lifecycle::{NativeLifecycleClient, NativeLifecycleEvent, NativeOutboundMessage};

const MAX_NATIVE_HTTP_URL_BYTES: usize = 16 * 1024;
const MAX_NATIVE_HTTP_HEADERS: usize = 64;
const MAX_NATIVE_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_NATIVE_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_NATIVE_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_NATIVE_REGISTRY_USERS: usize = 10_000;
const MAX_NATIVE_REGISTRY_ENDPOINTS_PER_USER: usize = 32;
const MAX_NATIVE_REGISTRY_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_NATIVE_HTTP_DNS_ADDRESSES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeHttpDestinationPolicy {
    /// Owner-configured endpoint; HTTP is permitted for local/self-hosted use.
    Configured,
    /// Untrusted or provider endpoint; require public HTTPS and pin DNS.
    PublicHttps,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub destination_policy: NativeHttpDestinationPolicy,
}

impl std::fmt::Debug for NativeHttpRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeHttpRequest")
            .field("method", &self.method)
            .field("url", &"[REDACTED]")
            .field("header_count", &self.headers.len())
            .field("body_bytes", &self.body.len())
            .field("destination_policy", &self.destination_policy)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NativeHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for NativeHttpResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeHttpResponse")
            .field("status", &self.status)
            .field("body_bytes", &self.body.len())
            .finish()
    }
}

#[async_trait]
pub trait NativeHttpClient: Send + Sync {
    async fn send(&self, request: NativeHttpRequest) -> Result<NativeHttpResponse, ChannelError>;
}

#[derive(Debug)]
pub struct ReqwestNativeHttpClient;

impl Default for ReqwestNativeHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ReqwestNativeHttpClient {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NativeHttpClient for ReqwestNativeHttpClient {
    async fn send(&self, request: NativeHttpRequest) -> Result<NativeHttpResponse, ChannelError> {
        if request.url.is_empty()
            || request.url.len() > MAX_NATIVE_HTTP_URL_BYTES
            || request.headers.len() > MAX_NATIVE_HTTP_HEADERS
            || request.body.len() > MAX_NATIVE_HTTP_BODY_BYTES
            || request.headers.iter().fold(0usize, |total, (name, value)| {
                total.saturating_add(name.len()).saturating_add(value.len())
            }) > MAX_NATIVE_HTTP_HEADER_BYTES
            || request.headers.iter().any(|(name, value)| {
                name.is_empty()
                    || name.len() > 256
                    || value.len() > 16 * 1024
                    || name.chars().any(char::is_control)
                    || value.contains(['\r', '\n', '\0'])
            })
        {
            return Err(ChannelError::SendFailed {
                name: "native-http".to_string(),
                reason: "native HTTP request is malformed or oversized".to_string(),
            });
        }
        let method = request.method.parse::<reqwest::Method>().map_err(|error| {
            ChannelError::SendFailed {
                name: "native-http".to_string(),
                reason: format!("invalid HTTP method: {error}"),
            }
        })?;
        if !matches!(
            method.as_str(),
            "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "HEAD"
        ) {
            return Err(ChannelError::SendFailed {
                name: "native-http".to_string(),
                reason: "native HTTP method is not supported".to_string(),
            });
        }
        let mut headers = reqwest::header::HeaderMap::with_capacity(request.headers.len());
        for (name, value) in &request.headers {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP header name is invalid".to_string(),
                }
            })?;
            if matches!(
                name.as_str(),
                "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
                    | "proxy-connection"
                    | "upgrade"
                    | "te"
                    | "trailer"
            ) {
                return Err(ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP request contains a transport-controlled header"
                        .to_string(),
                });
            }
            let value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
                ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP header value is invalid".to_string(),
                }
            })?;
            if headers.insert(name, value).is_some() {
                return Err(ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP request contains duplicate headers".to_string(),
                });
            }
        }

        let parsed = Url::parse(&request.url).map_err(|_| ChannelError::SendFailed {
            name: "native-http".to_string(),
            reason: "native HTTP URL is malformed".to_string(),
        })?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ChannelError::SendFailed {
                name: "native-http".to_string(),
                reason: "native HTTP URL is not trusted".to_string(),
            });
        }

        let (client, target) = match request.destination_policy {
            NativeHttpDestinationPolicy::Configured => {
                let host = parsed.host_str().ok_or_else(|| ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP URL has no host".to_string(),
                })?;
                let port =
                    parsed
                        .port_or_known_default()
                        .ok_or_else(|| ChannelError::SendFailed {
                            name: "native-http".to_string(),
                            reason: "native HTTP URL has no port".to_string(),
                        })?;
                let resolved = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio::net::lookup_host((host, port)),
                )
                .await
                .map_err(|_| ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP DNS lookup timed out".to_string(),
                })?
                .map_err(|_| ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP DNS lookup failed".to_string(),
                })?;
                let mut addresses = resolved.collect::<Vec<_>>();
                addresses.sort_unstable();
                addresses.dedup();
                if addresses.is_empty()
                    || addresses.len() > MAX_NATIVE_HTTP_DNS_ADDRESSES
                    || addresses.iter().any(|address| {
                        let ip = address.ip();
                        ip.is_unspecified()
                            || ip.is_multicast()
                            || matches!(ip, std::net::IpAddr::V4(ip) if ip.is_broadcast())
                            || parsed.scheme() == "http"
                                && thinclaw_tools_core::is_public_outbound_ip(ip)
                    })
                {
                    return Err(ChannelError::SendFailed {
                        name: "native-http".to_string(),
                        reason: "native HTTP destination resolved to an invalid address"
                            .to_string(),
                    });
                }
                // A bounded client is required: without timeouts a blackholed
                // endpoint can wedge channel health checks. Rebuilding here lets
                // us pin the exact DNS answers validated above.
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy()
                    .resolve_to_addrs(host, &addresses)
                    .build()
                    .map_err(|_| ChannelError::SendFailed {
                        name: "native-http".to_string(),
                        reason: "failed to build native HTTP client".to_string(),
                    })?;
                (client, parsed)
            }
            NativeHttpDestinationPolicy::PublicHttps => {
                let guarded = thinclaw_tools_core::validate_outbound_url_pinned_async(
                    parsed.as_str(),
                    &thinclaw_tools_core::OutboundUrlGuardOptions {
                        require_https: true,
                        upgrade_http_to_https: false,
                        allowlist: Vec::new(),
                    },
                )
                .await
                .map_err(|_| ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: "native HTTP destination is not a public HTTPS endpoint".to_string(),
                })?;
                let mut client_builder = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .connect_timeout(std::time::Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .no_proxy();
                if !guarded.pinned_addrs.is_empty()
                    && let Some(host) = guarded.url.host_str()
                {
                    client_builder = client_builder.resolve_to_addrs(host, &guarded.pinned_addrs);
                }
                let client = client_builder
                    .build()
                    .map_err(|_| ChannelError::SendFailed {
                        name: "native-http".to_string(),
                        reason: "failed to build native HTTP client".to_string(),
                    })?;
                (client, guarded.url)
            }
        };

        let builder = client.request(method, target).headers(headers);
        let response =
            builder
                .body(request.body)
                .send()
                .await
                .map_err(|error| ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    // `without_url` strips the request URL from the error text.
                    // Provider URLs embed secrets (APNs device token in the
                    // path, BlueBubbles password in the query), and this error
                    // is logged upstream.
                    reason: error.without_url().to_string(),
                })?;
        let status = response.status().as_u16();
        let body = crate::response::bounded_bytes(response, MAX_NATIVE_HTTP_RESPONSE_BYTES)
            .await
            .map_err(|error| ChannelError::SendFailed {
                name: "native-http".to_string(),
                reason: format!("native HTTP response is invalid: {error}"),
            })?;
        Ok(NativeHttpResponse { status, body })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct MatrixNativeConfig {
    pub homeserver: String,
    pub access_token: String,
}

impl std::fmt::Debug for MatrixNativeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MatrixNativeConfig")
            .field("homeserver", &self.homeserver)
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

pub struct MatrixNativeClient {
    config: MatrixNativeConfig,
    http: Arc<dyn NativeHttpClient>,
}

impl MatrixNativeClient {
    pub fn new(config: MatrixNativeConfig, http: Arc<dyn NativeHttpClient>) -> Self {
        Self { config, http }
    }

    fn endpoint(&self, path: &str) -> Result<String, ChannelError> {
        let base = validate_configured_origin(&self.config.homeserver, "matrix")?;
        base.join(path.trim_start_matches('/'))
            .map(|url| url.to_string())
            .map_err(|error| ChannelError::StartupFailed {
                name: "matrix".to_string(),
                reason: format!("failed to build Matrix endpoint: {error}"),
            })
    }

    pub fn event_from_room_message(event: &Value) -> Option<NativeLifecycleEvent> {
        let text = event
            .pointer("/content/body")
            .and_then(Value::as_str)?
            .trim()
            .to_string();
        let chat_id = event.get("room_id")?.as_str()?;
        let user_id = event.get("sender")?.as_str()?;
        if text.is_empty()
            || text.len() > 256 * 1024
            || !valid_native_identifier(chat_id, 4096)
            || !valid_native_identifier(user_id, 4096)
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
}

#[async_trait]
impl NativeLifecycleClient for MatrixNativeClient {
    async fn validate(&self) -> Result<(), ChannelError> {
        validate_secret_value(&self.config.access_token, "matrix", "access token")?;
        let response = self
            .http
            .send(NativeHttpRequest {
                method: "GET".to_string(),
                url: self.endpoint("/_matrix/client/v3/account/whoami")?,
                headers: bearer_headers(&self.config.access_token),
                body: Vec::new(),
                destination_policy: NativeHttpDestinationPolicy::Configured,
            })
            .await?;
        ensure_success("matrix", "whoami", response.status)
    }

    async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError> {
        if !valid_native_identifier(&message.chat_id, 4096) || message.content.len() > 256 * 1024 {
            return Err(ChannelError::SendFailed {
                name: "matrix".to_string(),
                reason: "Matrix message is malformed or oversized".to_string(),
            });
        }
        let txn = Uuid::new_v4().simple().to_string();
        let path = format!(
            "/_matrix/client/v3/rooms/{}/send/m.room.message/{}",
            urlencoding::encode(&message.chat_id),
            txn
        );
        let body = serde_json::json!({
            "msgtype": "m.text",
            "body": message.content,
        });
        let response = self
            .http
            .send(NativeHttpRequest {
                method: "PUT".to_string(),
                url: self.endpoint(&path)?,
                headers: json_bearer_headers(&self.config.access_token),
                body: serde_json::to_vec(&body).unwrap_or_default(),
                destination_policy: NativeHttpDestinationPolicy::Configured,
            })
            .await?;
        ensure_success("matrix", "send message", response.status)
    }

    async fn diagnostics(&self) -> Value {
        serde_json::json!({
            "provider": "matrix",
            "homeserver": self.config.homeserver,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VoiceCallNativeConfig {
    pub response_url: String,
    pub webhook_secret: Option<String>,
}

impl std::fmt::Debug for VoiceCallNativeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VoiceCallNativeConfig")
            .field("response_url", &redacted_url_origin(&self.response_url))
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

fn redacted_url_origin(raw: &str) -> String {
    let Ok(url) = Url::parse(raw) else {
        return "<invalid-url>".to_string();
    };
    let Some(host) = url.host_str() else {
        return "<invalid-url>".to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    }
}

pub struct VoiceCallNativeClient {
    config: VoiceCallNativeConfig,
    http: Arc<dyn NativeHttpClient>,
}

impl VoiceCallNativeClient {
    pub fn new(config: VoiceCallNativeConfig, http: Arc<dyn NativeHttpClient>) -> Self {
        Self { config, http }
    }
}

#[async_trait]
impl NativeLifecycleClient for VoiceCallNativeClient {
    async fn validate(&self) -> Result<(), ChannelError> {
        validate_public_https_url(&self.config.response_url, "voice-call")?;
        if let Some(secret) = self.config.webhook_secret.as_deref() {
            validate_secret_value(secret, "voice-call", "webhook secret")?;
        }
        Ok(())
    }

    async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError> {
        self.validate().await?;
        if !valid_native_identifier(&message.chat_id, 4096)
            || !valid_native_identifier(&message.user_id, 4096)
            || message.content.len() > 256 * 1024
        {
            return Err(ChannelError::SendFailed {
                name: "voice-call".to_string(),
                reason: "Voice response is malformed or oversized".to_string(),
            });
        }
        let mut headers =
            HashMap::from([("Content-Type".to_string(), "application/json".to_string())]);
        if let Some(secret) = &self.config.webhook_secret {
            headers.insert("X-ThinClaw-Voice-Secret".to_string(), secret.clone());
        }
        let response = self
            .http
            .send(NativeHttpRequest {
                method: "POST".to_string(),
                url: self.config.response_url.clone(),
                headers,
                body: serde_json::to_vec(&serde_json::json!({
                    "call_id": message.chat_id,
                    "user_id": message.user_id,
                    "text": message.content,
                    "metadata": message.metadata,
                }))
                .unwrap_or_default(),
                destination_policy: NativeHttpDestinationPolicy::PublicHttps,
            })
            .await?;
        ensure_success("voice-call", "send response", response.status)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ApnsNativeConfig {
    pub team_id: String,
    pub key_id: String,
    pub bundle_id: String,
    pub private_key_pem: String,
    pub sandbox: bool,
}

impl std::fmt::Debug for ApnsNativeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApnsNativeConfig")
            .field("team_id", &self.team_id)
            .field("key_id", &self.key_id)
            .field("bundle_id", &self.bundle_id)
            .field("private_key_pem", &"[REDACTED]")
            .field("sandbox", &self.sandbox)
            .finish()
    }
}

pub struct ApnsNativeClient {
    config: ApnsNativeConfig,
    http: Arc<dyn NativeHttpClient>,
    registry: NativeEndpointRegistry,
}

impl ApnsNativeClient {
    pub fn new(config: ApnsNativeConfig, http: Arc<dyn NativeHttpClient>) -> Self {
        Self {
            config,
            http,
            registry: NativeEndpointRegistry::default(),
        }
    }

    pub fn with_registry(
        config: ApnsNativeConfig,
        http: Arc<dyn NativeHttpClient>,
        registry: NativeEndpointRegistry,
    ) -> Self {
        Self {
            config,
            http,
            registry,
        }
    }

    fn provider_token(&self) -> Result<String, ChannelError> {
        apns_provider_token(&self.config)
    }

    fn pusher(&self) -> ApnsPusher {
        ApnsPusher::new(self.config.clone(), self.http.clone())
    }

    async fn send_to_device_token(
        &self,
        device_token: &str,
        message: &NativeOutboundMessage,
    ) -> Result<(), ChannelError> {
        if !valid_native_identifier(device_token, 512)
            || message.content.len() > 256 * 1024
            || !valid_native_identifier(&message.user_id, 4096)
        {
            return Err(ChannelError::SendFailed {
                name: "apns".to_string(),
                reason: "APNs destination or payload is malformed or oversized".to_string(),
            });
        }
        let payload = serde_json::json!({
            "aps": {
                "alert": message.content,
                "sound": "default"
            },
            "thinclaw": {
                "channel": message.channel,
                "user_id": message.user_id,
                "metadata": message.metadata,
            }
        });
        self.pusher()
            .send(device_token, ApnsPushSpec::alert(payload))
            .await
            .map(|_| ())
    }
}

/// Sign an APNs provider authentication token (ES256 JWT) for `config`.
pub(crate) fn apns_provider_token(config: &ApnsNativeConfig) -> Result<String, ChannelError> {
    jwt_es256(
        "apns",
        config.key_id.clone(),
        ApnsClaims {
            iss: config.team_id.clone(),
            iat: unix_now(),
        },
        &config.private_key_pem,
    )
}

#[async_trait]
impl NativeLifecycleClient for ApnsNativeClient {
    async fn validate(&self) -> Result<(), ChannelError> {
        for (label, value) in [
            ("team ID", self.config.team_id.as_str()),
            ("key ID", self.config.key_id.as_str()),
            ("bundle ID", self.config.bundle_id.as_str()),
        ] {
            if !valid_native_identifier(value, 256) {
                return Err(ChannelError::AuthFailed {
                    name: "apns".to_string(),
                    reason: format!("APNs {label} is malformed or oversized"),
                });
            }
        }
        if self.config.private_key_pem.len() > 64 * 1024 {
            return Err(ChannelError::AuthFailed {
                name: "apns".to_string(),
                reason: "APNs private key is oversized".to_string(),
            });
        }
        let _ = self.provider_token()?;
        Ok(())
    }

    async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError> {
        let targets = self.registry.endpoints_for(&message.chat_id).await;
        if targets.is_empty() {
            self.send_to_device_token(&message.chat_id, &message).await
        } else {
            for device_token in targets {
                self.send_to_device_token(&device_token, &message).await?;
            }
            Ok(())
        }
    }

    async fn diagnostics(&self) -> Value {
        serde_json::json!({
            "provider": "apns",
            "registered_users": self.registry.user_count().await,
            "registered_endpoints": self.registry.endpoint_count().await,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BrowserPushNativeConfig {
    pub vapid_public_key: String,
    pub vapid_private_key_pem: String,
    pub subject: String,
    pub ttl_seconds: u32,
}

impl std::fmt::Debug for BrowserPushNativeConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserPushNativeConfig")
            .field("vapid_public_key", &self.vapid_public_key)
            .field("vapid_private_key_pem", &"[REDACTED]")
            .field("subject", &self.subject)
            .field("ttl_seconds", &self.ttl_seconds)
            .finish()
    }
}

pub struct BrowserPushNativeClient {
    config: BrowserPushNativeConfig,
    http: Arc<dyn NativeHttpClient>,
    registry: NativeEndpointRegistry,
}

impl BrowserPushNativeClient {
    pub fn new(config: BrowserPushNativeConfig, http: Arc<dyn NativeHttpClient>) -> Self {
        Self {
            config,
            http,
            registry: NativeEndpointRegistry::default(),
        }
    }

    pub fn with_registry(
        config: BrowserPushNativeConfig,
        http: Arc<dyn NativeHttpClient>,
        registry: NativeEndpointRegistry,
    ) -> Self {
        Self {
            config,
            http,
            registry,
        }
    }

    fn vapid_token(&self, endpoint: &str) -> Result<String, ChannelError> {
        let aud = validate_public_https_url(endpoint, "browser-push").and_then(|url| {
            let origin = url.origin().ascii_serialization();
            if origin == "null" {
                Err(ChannelError::SendFailed {
                    name: "browser-push".to_string(),
                    reason: "subscription endpoint has no origin".to_string(),
                })
            } else {
                Ok(origin)
            }
        })?;
        jwt_es256(
            "browser-push",
            String::new(),
            VapidClaims {
                aud,
                exp: unix_now() + 12 * 60 * 60,
                sub: self.config.subject.clone(),
            },
            &self.config.vapid_private_key_pem,
        )
    }

    async fn send_to_endpoint(
        &self,
        endpoint: &str,
        message: &NativeOutboundMessage,
    ) -> Result<(), ChannelError> {
        let token = self.vapid_token(endpoint)?;
        let response = self
            .http
            .send(NativeHttpRequest {
                method: "POST".to_string(),
                url: endpoint.to_string(),
                headers: HashMap::from([
                    (
                        "Authorization".to_string(),
                        format!("vapid t={token}, k={}", self.config.vapid_public_key),
                    ),
                    ("TTL".to_string(), self.config.ttl_seconds.to_string()),
                    ("Content-Length".to_string(), "0".to_string()),
                ]),
                body: Vec::new(),
                destination_policy: NativeHttpDestinationPolicy::PublicHttps,
            })
            .await?;
        ensure_success("browser-push", "wake subscription", response.status)?;
        let _ = message;
        Ok(())
    }
}

#[async_trait]
impl NativeLifecycleClient for BrowserPushNativeClient {
    async fn validate(&self) -> Result<(), ChannelError> {
        if self.config.vapid_public_key.trim().is_empty()
            || self.config.vapid_public_key.len() > 4096
            || self.config.vapid_public_key.chars().any(char::is_control)
            || self.config.vapid_private_key_pem.is_empty()
            || self.config.vapid_private_key_pem.len() > 64 * 1024
            || self.config.subject.is_empty()
            || self.config.subject.len() > 4096
            || self.config.subject.chars().any(char::is_control)
            || self.config.ttl_seconds > 2_419_200
        {
            return Err(ChannelError::AuthFailed {
                name: "browser-push".to_string(),
                reason: "VAPID configuration is malformed or oversized".to_string(),
            });
        }
        if !(self.config.subject.starts_with("mailto:")
            || self.config.subject.starts_with("https://"))
        {
            return Err(ChannelError::AuthFailed {
                name: "browser-push".to_string(),
                reason: "VAPID subject must be a mailto: or HTTPS URI".to_string(),
            });
        }
        let _ = jwt_es256(
            "browser-push",
            String::new(),
            VapidClaims {
                aud: "https://example.invalid".to_string(),
                exp: unix_now() + 60,
                sub: self.config.subject.clone(),
            },
            &self.config.vapid_private_key_pem,
        )?;
        Ok(())
    }

    async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError> {
        let targets = self.registry.endpoints_for(&message.chat_id).await;
        if targets.is_empty() {
            self.send_to_endpoint(&message.chat_id, &message).await
        } else {
            for endpoint in targets {
                self.send_to_endpoint(&endpoint, &message).await?;
            }
            Ok(())
        }
    }

    async fn diagnostics(&self) -> Value {
        serde_json::json!({
            "provider": "browser-push",
            "registered_users": self.registry.user_count().await,
            "registered_endpoints": self.registry.endpoint_count().await,
        })
    }
}

#[derive(Clone, Default)]
pub struct NativeEndpointRegistry {
    inner: Arc<RwLock<HashMap<String, BTreeSet<String>>>>,
    persistence_path: Option<Arc<PathBuf>>,
    persist_lock: Arc<tokio::sync::Mutex<()>>,
}

impl std::fmt::Debug for NativeEndpointRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeEndpointRegistry")
            .field("persistence_configured", &self.persistence_path.is_some())
            .finish_non_exhaustive()
    }
}

impl NativeEndpointRegistry {
    pub async fn persistent(path: impl Into<PathBuf>) -> Result<Self, ChannelError> {
        let path = path.into();
        let registry = Self {
            inner: Arc::new(RwLock::new(load_endpoint_registry_file(&path).await?)),
            persistence_path: Some(Arc::new(path)),
            persist_lock: Arc::new(tokio::sync::Mutex::new(())),
        };
        Ok(registry)
    }

    pub async fn register(
        &self,
        user_id: impl Into<String>,
        endpoint: impl Into<String>,
    ) -> Result<(), ChannelError> {
        let user_id = user_id.into();
        let endpoint = endpoint.into();
        if !valid_native_identifier(&user_id, 4096)
            || !valid_native_identifier(&endpoint, MAX_NATIVE_HTTP_URL_BYTES)
        {
            return Err(ChannelError::Configuration(
                "native endpoint registration is malformed or oversized".to_string(),
            ));
        }
        {
            let mut guard = self.inner.write().await;
            if !guard.contains_key(&user_id) && guard.len() >= MAX_NATIVE_REGISTRY_USERS {
                return Err(ChannelError::Configuration(
                    "native endpoint registry user limit reached".to_string(),
                ));
            }
            let endpoints = guard.entry(user_id).or_default();
            if !endpoints.contains(&endpoint)
                && endpoints.len() >= MAX_NATIVE_REGISTRY_ENDPOINTS_PER_USER
            {
                return Err(ChannelError::Configuration(
                    "native endpoint limit reached for user".to_string(),
                ));
            }
            endpoints.insert(endpoint);
        }
        self.persist().await
    }

    pub async fn unregister(&self, user_id: &str, endpoint: &str) -> bool {
        let removed = {
            let mut guard = self.inner.write().await;
            let Some(endpoints) = guard.get_mut(user_id) else {
                return false;
            };
            let removed = endpoints.remove(endpoint);
            if endpoints.is_empty() {
                guard.remove(user_id);
            }
            removed
        };
        if removed && let Err(error) = self.persist().await {
            tracing::warn!(error = %error, "failed to persist native endpoint unregistration");
        }
        removed
    }

    pub async fn endpoints_for(&self, user_id: &str) -> Vec<String> {
        self.inner
            .read()
            .await
            .get(user_id)
            .map(|endpoints| endpoints.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub async fn user_count(&self) -> usize {
        self.inner.read().await.len()
    }

    pub async fn endpoint_count(&self) -> usize {
        self.inner.read().await.values().map(BTreeSet::len).sum()
    }

    pub async fn persist(&self) -> Result<(), ChannelError> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let _persist_guard = self.persist_lock.lock().await;
        let snapshot = self.snapshot().await;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                ChannelError::Configuration(format!(
                    "failed to create native endpoint registry directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let encoded = serde_json::to_vec_pretty(&NativeEndpointRegistryFile { users: snapshot })
            .map_err(|error| {
                ChannelError::Configuration(format!(
                    "failed to encode native endpoint registry {}: {error}",
                    path.display()
                ))
            })?;
        if encoded.len() > MAX_NATIVE_REGISTRY_FILE_BYTES {
            return Err(ChannelError::Configuration(
                "native endpoint registry exceeds the persistence size limit".to_string(),
            ));
        }
        thinclaw_platform::write_private_file_atomic_async(path.as_ref().clone(), encoded, true)
            .await
            .map_err(|error| {
                ChannelError::Configuration(format!(
                    "failed to persist native endpoint registry: {error}"
                ))
            })
    }

    async fn snapshot(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.inner
            .read()
            .await
            .iter()
            .map(|(user_id, endpoints)| (user_id.clone(), endpoints.clone()))
            .collect()
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
struct NativeEndpointRegistryFile {
    #[serde(default)]
    users: BTreeMap<String, BTreeSet<String>>,
}

impl std::fmt::Debug for NativeEndpointRegistryFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeEndpointRegistryFile")
            .field("user_count", &self.users.len())
            .field(
                "endpoint_count",
                &self.users.values().map(BTreeSet::len).sum::<usize>(),
            )
            .finish()
    }
}

async fn load_endpoint_registry_file(
    path: &Path,
) -> Result<HashMap<String, BTreeSet<String>>, ChannelError> {
    let bytes = match thinclaw_platform::read_regular_file_bounded_async(
        path.to_path_buf(),
        MAX_NATIVE_REGISTRY_FILE_BYTES as u64,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => {
            return Err(ChannelError::Configuration(format!(
                "native endpoint registry file is unsafe or unreadable: {error}"
            )));
        }
    };
    let decoded: NativeEndpointRegistryFile = serde_json::from_slice(&bytes).map_err(|error| {
        ChannelError::Configuration(format!(
            "failed to parse native endpoint registry {}: {error}",
            path.display()
        ))
    })?;
    if decoded.users.len() > MAX_NATIVE_REGISTRY_USERS
        || decoded.users.iter().any(|(user_id, endpoints)| {
            !valid_native_identifier(user_id, 4096)
                || endpoints.len() > MAX_NATIVE_REGISTRY_ENDPOINTS_PER_USER
                || endpoints
                    .iter()
                    .any(|endpoint| !valid_native_identifier(endpoint, MAX_NATIVE_HTTP_URL_BYTES))
        })
    {
        return Err(ChannelError::Configuration(
            "native endpoint registry contents are malformed or oversized".to_string(),
        ));
    }
    Ok(decoded.users.into_iter().collect())
}

#[derive(Debug, Serialize)]
struct ApnsClaims {
    iss: String,
    iat: u64,
}

#[derive(Debug, Serialize)]
struct VapidClaims {
    aud: String,
    exp: u64,
    sub: String,
}

fn jwt_es256<T: Serialize>(
    channel: &str,
    key_id: String,
    claims: T,
    private_key_pem: &str,
) -> Result<String, ChannelError> {
    let mut header = Header::new(Algorithm::ES256);
    if !key_id.is_empty() {
        header.kid = Some(key_id);
    }
    let key = EncodingKey::from_ec_pem(private_key_pem.as_bytes()).map_err(|error| {
        ChannelError::AuthFailed {
            name: channel.to_string(),
            reason: format!("invalid ES256 private key: {error}"),
        }
    })?;
    encode(&header, &claims, &key).map_err(|error| ChannelError::AuthFailed {
        name: channel.to_string(),
        reason: format!("failed to sign provider token: {error}"),
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn valid_native_identifier(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes && !value.chars().any(char::is_control)
}

fn validate_secret_value(value: &str, channel: &str, label: &str) -> Result<(), ChannelError> {
    if value.is_empty() || value.len() > 64 * 1024 || value.chars().any(char::is_control) {
        return Err(ChannelError::AuthFailed {
            name: channel.to_string(),
            reason: format!("{label} is missing, malformed, or oversized"),
        });
    }
    Ok(())
}

fn validate_configured_origin(value: &str, channel: &str) -> Result<Url, ChannelError> {
    let parsed = Url::parse(value).map_err(|_| ChannelError::StartupFailed {
        name: channel.to_string(),
        reason: "configured server URL is malformed".to_string(),
    })?;
    let host = parsed
        .host_str()
        .ok_or_else(|| ChannelError::StartupFailed {
            name: channel.to_string(),
            reason: "configured server URL requires a host".to_string(),
        })?;
    if value.len() > MAX_NATIVE_HTTP_URL_BYTES
        || !matches!(parsed.scheme(), "http" | "https")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
        || (parsed.scheme() == "http" && !is_local_native_host(host))
    {
        return Err(ChannelError::StartupFailed {
            name: channel.to_string(),
            reason: "configured server must be an HTTPS origin (local HTTP is allowed)".to_string(),
        });
    }
    Ok(parsed)
}

fn validate_public_https_url(value: &str, channel: &str) -> Result<Url, ChannelError> {
    let parsed = Url::parse(value).map_err(|_| ChannelError::StartupFailed {
        name: channel.to_string(),
        reason: "provider URL is malformed".to_string(),
    })?;
    if value.len() > MAX_NATIVE_HTTP_URL_BYTES
        || parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
        || parsed
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|ip| !thinclaw_tools_core::is_public_outbound_ip(ip))
    {
        return Err(ChannelError::StartupFailed {
            name: channel.to_string(),
            reason: "provider URL must be a public credential-free HTTPS URL".to_string(),
        });
    }
    Ok(parsed)
}

fn is_local_native_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.to_ascii_lowercase().ends_with(".local")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| !thinclaw_tools_core::is_public_outbound_ip(ip))
}

fn bearer_headers(token: &str) -> HashMap<String, String> {
    HashMap::from([("Authorization".to_string(), format!("Bearer {token}"))])
}

fn json_bearer_headers(token: &str) -> HashMap<String, String> {
    let mut headers = bearer_headers(token);
    headers.insert("Content-Type".to_string(), "application/json".to_string());
    headers
}

pub(crate) fn ensure_success(
    channel: &str,
    operation: &str,
    status: u16,
) -> Result<(), ChannelError> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(ChannelError::SendFailed {
            name: channel.to_string(),
            reason: format!("{operation} failed with HTTP {status}"),
        })
    }
}

/// Shared test fixtures for the native lifecycle client tests and the
/// [`crate::apns_push`] pusher tests, which reuse the same ES256 signing key.
#[cfg(test)]
pub(crate) mod test_support {
    use base64::Engine as _;
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};

    /// Generate a throwaway ES256 private key in memory so signing-path tests
    /// never require a credential-like private key fixture in the repository.
    pub(crate) fn ec_private_key() -> String {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .expect("generate test-only ES256 key");
        let encoded = base64::engine::general_purpose::STANDARD.encode(pkcs8.as_ref());
        let mut pem = String::from("-----BEGIN PRIVATE KEY-----\n");
        for line in encoded.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
            pem.push('\n');
        }
        pem.push_str("-----END PRIVATE KEY-----\n");
        pem
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::ec_private_key;
    use super::*;
    use tokio::sync::Mutex;

    #[derive(Default)]
    struct MockHttp {
        requests: Mutex<Vec<NativeHttpRequest>>,
        status: u16,
    }

    impl MockHttp {
        fn ok() -> Arc<Self> {
            Arc::new(Self {
                status: 200,
                ..Self::default()
            })
        }

        async fn take(&self) -> Vec<NativeHttpRequest> {
            std::mem::take(&mut *self.requests.lock().await)
        }
    }

    #[async_trait]
    impl NativeHttpClient for MockHttp {
        async fn send(
            &self,
            request: NativeHttpRequest,
        ) -> Result<NativeHttpResponse, ChannelError> {
            self.requests.lock().await.push(request);
            Ok(NativeHttpResponse {
                status: if self.status == 0 { 200 } else { self.status },
                body: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn matrix_client_validates_and_sends_room_message() {
        let http = MockHttp::ok();
        let client = MatrixNativeClient::new(
            MatrixNativeConfig {
                homeserver: "https://matrix.example.org".to_string(),
                access_token: "matrix-token".to_string(),
            },
            http.clone(),
        );

        client.validate().await.unwrap();
        client
            .send(NativeOutboundMessage {
                channel: "matrix".to_string(),
                chat_type: "room".to_string(),
                chat_id: "!room:example.org".to_string(),
                user_id: "@user:example.org".to_string(),
                content: "hello".to_string(),
                metadata: Value::Null,
            })
            .await
            .unwrap();

        let requests = http.take().await;
        assert_eq!(requests[0].method, "GET");
        assert_eq!(
            requests[0].url,
            "https://matrix.example.org/_matrix/client/v3/account/whoami"
        );
        assert_eq!(
            requests[0].headers.get("Authorization").map(String::as_str),
            Some("Bearer matrix-token")
        );
        assert_eq!(requests[1].method, "PUT");
        assert!(
            requests[1]
                .url
                .contains("/rooms/%21room%3Aexample.org/send/m.room.message/")
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[1].body).unwrap()["body"],
            "hello"
        );
    }

    #[test]
    fn matrix_event_parser_maps_room_message_to_lifecycle_event() {
        let event = MatrixNativeClient::event_from_room_message(&serde_json::json!({
            "room_id": "!room:example.org",
            "sender": "@user:example.org",
            "event_id": "$event",
            "origin_server_ts": 1,
            "content": { "body": "hello" }
        }))
        .unwrap();

        assert_eq!(event.chat_id, "!room:example.org");
        assert_eq!(event.chat_type.as_deref(), Some("room"));
        assert_eq!(event.user_id, "@user:example.org");
        assert_eq!(event.text, "hello");
    }

    #[tokio::test]
    async fn voice_client_posts_call_response_payload() {
        let http = MockHttp::ok();
        let client = VoiceCallNativeClient::new(
            VoiceCallNativeConfig {
                response_url: "https://voice.example.test/respond".to_string(),
                webhook_secret: Some("voice-secret".to_string()),
            },
            http.clone(),
        );

        client.validate().await.unwrap();
        client
            .send(NativeOutboundMessage {
                channel: "voice-call".to_string(),
                chat_type: "call".to_string(),
                chat_id: "call-1".to_string(),
                user_id: "caller".to_string(),
                content: "short spoken reply".to_string(),
                metadata: serde_json::json!({"state": "active"}),
            })
            .await
            .unwrap();

        let request = http.take().await.pop().unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "https://voice.example.test/respond");
        assert_eq!(
            request
                .headers
                .get("X-ThinClaw-Voice-Secret")
                .map(String::as_str),
            Some("voice-secret")
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap()["call_id"],
            "call-1"
        );
    }

    #[tokio::test]
    async fn apns_client_signs_provider_token_and_posts_alert() {
        let http = MockHttp::ok();
        let client = ApnsNativeClient::new(
            ApnsNativeConfig {
                team_id: "TEAMID1234".to_string(),
                key_id: "KEYID1234".to_string(),
                bundle_id: "com.example.thinclaw".to_string(),
                private_key_pem: ec_private_key(),
                sandbox: true,
            },
            http.clone(),
        );

        client.validate().await.unwrap();
        client
            .send(NativeOutboundMessage {
                channel: "apns".to_string(),
                chat_type: "device".to_string(),
                chat_id: "device-token".to_string(),
                user_id: "user".to_string(),
                content: "wake up".to_string(),
                metadata: Value::Null,
            })
            .await
            .unwrap();

        let request = http.take().await.pop().unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.url,
            "https://api.sandbox.push.apple.com/3/device/device-token"
        );
        assert!(request.headers["Authorization"].starts_with("bearer "));
        assert_eq!(request.headers["apns-topic"], "com.example.thinclaw");
        assert_eq!(
            serde_json::from_slice::<Value>(&request.body).unwrap()["aps"]["alert"],
            "wake up"
        );
    }

    #[tokio::test]
    async fn apns_client_broadcasts_to_registered_device_tokens() {
        let http = MockHttp::ok();
        let registry = NativeEndpointRegistry::default();
        registry.register("user-1", "device-token-a").await.unwrap();
        registry.register("user-1", "device-token-b").await.unwrap();
        let client = ApnsNativeClient::with_registry(
            ApnsNativeConfig {
                team_id: "TEAMID1234".to_string(),
                key_id: "KEYID1234".to_string(),
                bundle_id: "com.example.thinclaw".to_string(),
                private_key_pem: ec_private_key(),
                sandbox: true,
            },
            http.clone(),
            registry,
        );

        client
            .send(NativeOutboundMessage {
                channel: "apns".to_string(),
                chat_type: "device".to_string(),
                chat_id: "user-1".to_string(),
                user_id: "user-1".to_string(),
                content: "notify all devices".to_string(),
                metadata: Value::Null,
            })
            .await
            .unwrap();

        let requests = http.take().await;
        let urls = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(urls.len(), 2);
        assert!(urls.iter().any(|url| url.ends_with("/device-token-a")));
        assert!(urls.iter().any(|url| url.ends_with("/device-token-b")));
    }

    #[tokio::test]
    async fn browser_push_client_posts_vapid_wake_request() {
        let http = MockHttp::ok();
        let client = BrowserPushNativeClient::new(
            BrowserPushNativeConfig {
                vapid_public_key: "public-key".to_string(),
                vapid_private_key_pem: ec_private_key(),
                subject: "mailto:ops@example.test".to_string(),
                ttl_seconds: 60,
            },
            http.clone(),
        );

        client.validate().await.unwrap();
        client
            .send(NativeOutboundMessage {
                channel: "browser-push".to_string(),
                chat_type: "subscription".to_string(),
                chat_id: "https://push.example.test/subscription/1".to_string(),
                user_id: "user".to_string(),
                content: "wake".to_string(),
                metadata: Value::Null,
            })
            .await
            .unwrap();

        let request = http.take().await.pop().unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(request.url, "https://push.example.test/subscription/1");
        assert!(request.headers["Authorization"].starts_with("vapid t="));
        assert!(request.headers["Authorization"].contains(", k=public-key"));
        assert_eq!(request.headers["TTL"], "60");
        assert!(request.body.is_empty());
    }

    #[tokio::test]
    async fn browser_push_client_broadcasts_to_registered_subscriptions() {
        let http = MockHttp::ok();
        let registry = NativeEndpointRegistry::default();
        registry
            .register("user-1", "https://push.example.test/subscription/a")
            .await
            .unwrap();
        registry
            .register("user-1", "https://push.example.test/subscription/b")
            .await
            .unwrap();
        let client = BrowserPushNativeClient::with_registry(
            BrowserPushNativeConfig {
                vapid_public_key: "public-key".to_string(),
                vapid_private_key_pem: ec_private_key(),
                subject: "mailto:ops@example.test".to_string(),
                ttl_seconds: 60,
            },
            http.clone(),
            registry,
        );

        client
            .send(NativeOutboundMessage {
                channel: "browser-push".to_string(),
                chat_type: "subscription".to_string(),
                chat_id: "user-1".to_string(),
                user_id: "user-1".to_string(),
                content: "wake all browsers".to_string(),
                metadata: Value::Null,
            })
            .await
            .unwrap();

        let requests = http.take().await;
        let urls = requests
            .iter()
            .map(|request| request.url.as_str())
            .collect::<Vec<_>>();
        assert_eq!(urls.len(), 2);
        assert!(urls.contains(&"https://push.example.test/subscription/a"));
        assert!(urls.contains(&"https://push.example.test/subscription/b"));
    }

    #[tokio::test]
    async fn native_endpoint_registry_persists_registered_endpoints() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("native-endpoints").join("apns.json");
        let registry = NativeEndpointRegistry::persistent(path.clone())
            .await
            .unwrap();

        registry.register("user-1", "endpoint-a").await.unwrap();
        registry.register("user-1", "endpoint-b").await.unwrap();
        registry.register("user-2", "endpoint-c").await.unwrap();

        let loaded = NativeEndpointRegistry::persistent(path.clone())
            .await
            .unwrap();
        assert_eq!(
            loaded.endpoints_for("user-1").await,
            vec!["endpoint-a".to_string(), "endpoint-b".to_string()]
        );
        assert_eq!(loaded.endpoint_count().await, 3);

        assert!(loaded.unregister("user-1", "endpoint-a").await);

        let reloaded = NativeEndpointRegistry::persistent(path).await.unwrap();
        assert_eq!(
            reloaded.endpoints_for("user-1").await,
            vec!["endpoint-b".to_string()]
        );
        assert_eq!(reloaded.endpoint_count().await, 2);
    }

    #[tokio::test]
    async fn native_endpoint_registry_rejects_corrupt_persistence_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("native-endpoints.json");
        tokio::fs::write(&path, b"{not-json").await.unwrap();

        let error = NativeEndpointRegistry::persistent(path).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to parse native endpoint registry")
        );
    }
}
