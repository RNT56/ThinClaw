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
use std::path::PathBuf;
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

use crate::native_lifecycle::{NativeLifecycleClient, NativeLifecycleEvent, NativeOutboundMessage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeHttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait NativeHttpClient: Send + Sync {
    async fn send(&self, request: NativeHttpRequest) -> Result<NativeHttpResponse, ChannelError>;
}

#[derive(Debug, Default)]
pub struct ReqwestNativeHttpClient {
    client: reqwest::Client,
}

impl ReqwestNativeHttpClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl NativeHttpClient for ReqwestNativeHttpClient {
    async fn send(&self, request: NativeHttpRequest) -> Result<NativeHttpResponse, ChannelError> {
        let method = request.method.parse::<reqwest::Method>().map_err(|error| {
            ChannelError::SendFailed {
                name: "native-http".to_string(),
                reason: format!("invalid HTTP method: {error}"),
            }
        })?;
        let mut builder = self.client.request(method, &request.url);
        for (name, value) in request.headers {
            builder = builder.header(name, value);
        }
        let response =
            builder
                .body(request.body)
                .send()
                .await
                .map_err(|error| ChannelError::SendFailed {
                    name: "native-http".to_string(),
                    reason: error.to_string(),
                })?;
        let status = response.status().as_u16();
        let body = response
            .bytes()
            .await
            .map_err(|error| ChannelError::SendFailed {
                name: "native-http".to_string(),
                reason: error.to_string(),
            })?
            .to_vec();
        Ok(NativeHttpResponse { status, body })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixNativeConfig {
    pub homeserver: String,
    pub access_token: String,
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
        let base =
            Url::parse(&self.config.homeserver).map_err(|error| ChannelError::StartupFailed {
                name: "matrix".to_string(),
                reason: format!("invalid MATRIX_HOMESERVER: {error}"),
            })?;
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
        if text.is_empty() {
            return None;
        }
        Some(NativeLifecycleEvent {
            chat_id: event.get("room_id")?.as_str()?.to_string(),
            chat_type: Some("room".to_string()),
            user_id: event.get("sender")?.as_str()?.to_string(),
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
        let response = self
            .http
            .send(NativeHttpRequest {
                method: "GET".to_string(),
                url: self.endpoint("/_matrix/client/v3/account/whoami")?,
                headers: bearer_headers(&self.config.access_token),
                body: Vec::new(),
            })
            .await?;
        ensure_success("matrix", "whoami", response.status)
    }

    async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceCallNativeConfig {
    pub response_url: String,
    pub webhook_secret: Option<String>,
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
        Url::parse(&self.config.response_url).map_err(|error| ChannelError::StartupFailed {
            name: "voice-call".to_string(),
            reason: format!("invalid voice response URL: {error}"),
        })?;
        Ok(())
    }

    async fn send(&self, message: NativeOutboundMessage) -> Result<(), ChannelError> {
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
            })
            .await?;
        ensure_success("voice-call", "send response", response.status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnsNativeConfig {
    pub team_id: String,
    pub key_id: String,
    pub bundle_id: String,
    pub private_key_pem: String,
    pub sandbox: bool,
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
        jwt_es256(
            "apns",
            self.config.key_id.clone(),
            ApnsClaims {
                iss: self.config.team_id.clone(),
                iat: unix_now(),
            },
            &self.config.private_key_pem,
        )
    }

    async fn send_to_device_token(
        &self,
        device_token: &str,
        message: &NativeOutboundMessage,
    ) -> Result<(), ChannelError> {
        let host = if self.config.sandbox {
            "https://api.sandbox.push.apple.com"
        } else {
            "https://api.push.apple.com"
        };
        let token = self.provider_token()?;
        let body = serde_json::json!({
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
        let response = self
            .http
            .send(NativeHttpRequest {
                method: "POST".to_string(),
                url: format!("{host}/3/device/{device_token}"),
                headers: HashMap::from([
                    ("Authorization".to_string(), format!("bearer {token}")),
                    ("Content-Type".to_string(), "application/json".to_string()),
                    ("apns-topic".to_string(), self.config.bundle_id.clone()),
                    ("apns-push-type".to_string(), "alert".to_string()),
                ]),
                body: serde_json::to_vec(&body).unwrap_or_default(),
            })
            .await?;
        ensure_success("apns", "send notification", response.status)
    }
}

#[async_trait]
impl NativeLifecycleClient for ApnsNativeClient {
    async fn validate(&self) -> Result<(), ChannelError> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserPushNativeConfig {
    pub vapid_public_key: String,
    pub vapid_private_key_pem: String,
    pub subject: String,
    pub ttl_seconds: u32,
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
        let aud = Url::parse(endpoint)
            .map_err(|error| ChannelError::SendFailed {
                name: "browser-push".to_string(),
                reason: format!("invalid subscription endpoint: {error}"),
            })
            .and_then(|url| {
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
        if self.config.vapid_public_key.trim().is_empty() {
            return Err(ChannelError::AuthFailed {
                name: "browser-push".to_string(),
                reason: "VAPID public key is required".to_string(),
            });
        }
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

#[derive(Debug, Clone, Default)]
pub struct NativeEndpointRegistry {
    inner: Arc<RwLock<HashMap<String, BTreeSet<String>>>>,
    persistence_path: Option<Arc<PathBuf>>,
}

impl NativeEndpointRegistry {
    pub async fn persistent(path: impl Into<PathBuf>) -> Result<Self, ChannelError> {
        let path = path.into();
        let registry = Self {
            inner: Arc::new(RwLock::new(load_endpoint_registry_file(&path).await?)),
            persistence_path: Some(Arc::new(path)),
        };
        Ok(registry)
    }

    pub async fn register(&self, user_id: impl Into<String>, endpoint: impl Into<String>) {
        let user_id = user_id.into();
        let endpoint = endpoint.into();
        if user_id.trim().is_empty() || endpoint.trim().is_empty() {
            return;
        }
        self.inner
            .write()
            .await
            .entry(user_id)
            .or_default()
            .insert(endpoint);
        if let Err(error) = self.persist().await {
            tracing::warn!(error = %error, "failed to persist native endpoint registration");
        }
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
        tokio::fs::write(path.as_ref(), encoded)
            .await
            .map_err(|error| {
                ChannelError::Configuration(format!(
                    "failed to write native endpoint registry {}: {error}",
                    path.display()
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct NativeEndpointRegistryFile {
    #[serde(default)]
    users: BTreeMap<String, BTreeSet<String>>,
}

async fn load_endpoint_registry_file(
    path: &PathBuf,
) -> Result<HashMap<String, BTreeSet<String>>, ChannelError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => {
            return Err(ChannelError::Configuration(format!(
                "failed to read native endpoint registry {}: {error}",
                path.display()
            )));
        }
    };
    let decoded: NativeEndpointRegistryFile = serde_json::from_slice(&bytes).map_err(|error| {
        ChannelError::Configuration(format!(
            "failed to parse native endpoint registry {}: {error}",
            path.display()
        ))
    })?;
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

fn bearer_headers(token: &str) -> HashMap<String, String> {
    HashMap::from([("Authorization".to_string(), format!("Bearer {token}"))])
}

fn json_bearer_headers(token: &str) -> HashMap<String, String> {
    let mut headers = bearer_headers(token);
    headers.insert("Content-Type".to_string(), "application/json".to_string());
    headers
}

fn ensure_success(channel: &str, operation: &str, status: u16) -> Result<(), ChannelError> {
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(ChannelError::SendFailed {
            name: channel.to_string(),
            reason: format!("{operation} failed with HTTP {status}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Mutex;

    const EC_PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgJucIoF39xIAoPvuA\nkM4ZnoH7Epi+1r1Navl4D+QjrHahRANCAAQDw2h+idVQiHyp4aRqib1xUtSm0Xry\ndVN+sF496LBtVCQQ/vf0xtLTAxgXy3ViSOFKgac0apHRwNA8boZDN7Yy\n-----END PRIVATE KEY-----\n";

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
                private_key_pem: EC_PRIVATE_KEY.to_string(),
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
        registry.register("user-1", "device-token-a").await;
        registry.register("user-1", "device-token-b").await;
        let client = ApnsNativeClient::with_registry(
            ApnsNativeConfig {
                team_id: "TEAMID1234".to_string(),
                key_id: "KEYID1234".to_string(),
                bundle_id: "com.example.thinclaw".to_string(),
                private_key_pem: EC_PRIVATE_KEY.to_string(),
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
                vapid_private_key_pem: EC_PRIVATE_KEY.to_string(),
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
            .await;
        registry
            .register("user-1", "https://push.example.test/subscription/b")
            .await;
        let client = BrowserPushNativeClient::with_registry(
            BrowserPushNativeConfig {
                vapid_public_key: "public-key".to_string(),
                vapid_private_key_pem: EC_PRIVATE_KEY.to_string(),
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

        registry.register("user-1", "endpoint-a").await;
        registry.register("user-1", "endpoint-b").await;
        registry.register("user-2", "endpoint-c").await;

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
