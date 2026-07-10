//! First-party APNs pusher.
//!
//! [`ApnsPusher`] owns the APNs HTTP/2 request shape for content-free mobile
//! pushes: it sets the `apns-push-type`, `apns-priority`, `apns-collapse-id`,
//! `apns-expiration`, and `apns-topic` headers per Apple's contract and reads
//! the response body on `400`/`410` so callers can prune stale device tokens.
//!
//! This is a distinct surface from the `apns` chat channel in
//! [`crate::native_lifecycle`]: the channel remains a content-in-alert
//! lifecycle transport, while the pusher backs the first-party iOS device
//! registration path. [`crate::native_lifecycle_clients::ApnsNativeClient`]
//! delegates its per-device delivery to this pusher with an `Alert` spec so the
//! two share one signed-request implementation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use thinclaw_types::error::ChannelError;

use crate::native_lifecycle_clients::{
    ApnsNativeConfig, NativeHttpClient, NativeHttpRequest, apns_provider_token, ensure_success,
};

/// APNs delivery category, mapped to the `apns-push-type` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApnsPushType {
    /// User-visible alert (`apns-push-type: alert`).
    Alert,
    /// Silent content-available wake (`apns-push-type: background`).
    Background,
    /// Live Activity update (`apns-push-type: liveactivity`).
    LiveActivity,
}

impl ApnsPushType {
    /// The literal `apns-push-type` header value.
    pub fn header_value(self) -> &'static str {
        match self {
            Self::Alert => "alert",
            Self::Background => "background",
            Self::LiveActivity => "liveactivity",
        }
    }

    /// The default `apns-priority` for this push type: `10` for user-visible
    /// alerts and `5` for background wakes and throttled Live Activity updates.
    pub fn default_priority(self) -> u8 {
        match self {
            Self::Alert => 10,
            Self::Background | Self::LiveActivity => 5,
        }
    }

    /// The `apns-topic` suffix appended to the bundle id, if any. Live Activity
    /// pushes target `<bundle_id>.push-type.liveactivity`; other push types use
    /// the bare bundle id.
    pub fn topic_suffix(self) -> Option<&'static str> {
        match self {
            Self::LiveActivity => Some(".push-type.liveactivity"),
            Self::Alert | Self::Background => None,
        }
    }
}

/// A single APNs push request: the delivery headers plus the JSON payload.
#[derive(Debug, Clone, PartialEq)]
pub struct ApnsPushSpec {
    /// The delivery category (`apns-push-type`).
    pub push_type: ApnsPushType,
    /// The `apns-priority` header value (`10` or `5`).
    pub priority: u8,
    /// Optional `apns-collapse-id` used to coalesce updates.
    pub collapse_id: Option<String>,
    /// Optional `apns-expiration` (UNIX seconds); `0` means deliver-or-discard.
    pub expiration: Option<u64>,
    /// Optional `apns-topic` suffix appended to the bundle id.
    pub topic_suffix: Option<&'static str>,
    /// The APNs JSON payload (the full body, including the `aps` dictionary).
    pub payload: Value,
}

impl ApnsPushSpec {
    /// Build a spec for `push_type` carrying `payload`, applying the type's
    /// default priority and topic suffix.
    pub fn new(push_type: ApnsPushType, payload: Value) -> Self {
        Self {
            push_type,
            priority: push_type.default_priority(),
            collapse_id: None,
            expiration: None,
            topic_suffix: push_type.topic_suffix(),
            payload,
        }
    }

    /// A user-visible alert push carrying `payload`.
    pub fn alert(payload: Value) -> Self {
        Self::new(ApnsPushType::Alert, payload)
    }

    /// A silent background wake push carrying `payload`.
    pub fn background(payload: Value) -> Self {
        Self::new(ApnsPushType::Background, payload)
    }

    /// A Live Activity update push carrying `payload`.
    pub fn live_activity(payload: Value) -> Self {
        Self::new(ApnsPushType::LiveActivity, payload)
    }

    /// Set the `apns-collapse-id` used to coalesce updates.
    pub fn with_collapse_id(mut self, collapse_id: impl Into<String>) -> Self {
        self.collapse_id = Some(collapse_id.into());
        self
    }

    /// Set the `apns-expiration` (UNIX seconds); `0` means deliver-or-discard.
    pub fn with_expiration(mut self, expiration: u64) -> Self {
        self.expiration = Some(expiration);
        self
    }
}

/// The outcome of a single [`ApnsPusher::send`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApnsSendOutcome {
    /// APNs accepted the push (`2xx`).
    Delivered,
    /// APNs rejected the device token as unregistered or invalid
    /// (`{"reason":"Unregistered"|"BadDeviceToken"}` on `400`/`410`); callers
    /// should prune the token.
    Unregistered {
        /// The APNs rejection reason, e.g. `"Unregistered"`.
        reason: String,
    },
}

/// Apple expects an APNs provider (JWT) token to be reused for 20–60 minutes and
/// returns `403 TooManyProviderTokenUpdates` if it is regenerated too often. We
/// refresh well inside that window.
const PROVIDER_TOKEN_TTL: Duration = Duration::from_secs(45 * 60);

/// A cached, signed APNs provider token and when it was minted.
struct CachedProviderToken {
    token: String,
    issued: Instant,
}

/// First-party APNs pusher over an injectable HTTP transport.
pub struct ApnsPusher {
    config: ApnsNativeConfig,
    http: Arc<dyn NativeHttpClient>,
    provider_token: Mutex<Option<CachedProviderToken>>,
}

impl ApnsPusher {
    /// Build a pusher over `config` and `http`.
    pub fn new(config: ApnsNativeConfig, http: Arc<dyn NativeHttpClient>) -> Self {
        Self {
            config,
            http,
            provider_token: Mutex::new(None),
        }
    }

    /// Return a cached provider token, re-signing only when the cached one is
    /// older than [`PROVIDER_TOKEN_TTL`]. The lock is never held across an await.
    fn provider_token(&self) -> Result<String, ChannelError> {
        let mut guard = self
            .provider_token
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(cached) = guard.as_ref()
            && cached.issued.elapsed() < PROVIDER_TOKEN_TTL
        {
            return Ok(cached.token.clone());
        }
        let token = apns_provider_token(&self.config)?;
        *guard = Some(CachedProviderToken {
            token: token.clone(),
            issued: Instant::now(),
        });
        Ok(token)
    }

    fn host(&self) -> &'static str {
        if self.config.sandbox {
            "https://api.sandbox.push.apple.com"
        } else {
            "https://api.push.apple.com"
        }
    }

    fn topic(&self, suffix: Option<&str>) -> String {
        match suffix {
            Some(suffix) => format!("{}{suffix}", self.config.bundle_id),
            None => self.config.bundle_id.clone(),
        }
    }

    /// Send `spec` to `device_token`, returning [`ApnsSendOutcome::Unregistered`]
    /// when APNs rejects the token so the caller can prune it.
    pub async fn send(
        &self,
        device_token: &str,
        spec: ApnsPushSpec,
    ) -> Result<ApnsSendOutcome, ChannelError> {
        let token = self.provider_token()?;
        let mut headers = HashMap::from([
            ("Authorization".to_string(), format!("bearer {token}")),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("apns-topic".to_string(), self.topic(spec.topic_suffix)),
            (
                "apns-push-type".to_string(),
                spec.push_type.header_value().to_string(),
            ),
            ("apns-priority".to_string(), spec.priority.to_string()),
        ]);
        if let Some(collapse_id) = &spec.collapse_id {
            headers.insert("apns-collapse-id".to_string(), collapse_id.clone());
        }
        if let Some(expiration) = spec.expiration {
            headers.insert("apns-expiration".to_string(), expiration.to_string());
        }

        let response = self
            .http
            .send(NativeHttpRequest {
                method: "POST".to_string(),
                url: format!("{}/3/device/{device_token}", self.host()),
                headers,
                body: serde_json::to_vec(&spec.payload).unwrap_or_default(),
            })
            .await?;

        if (200..300).contains(&response.status) {
            return Ok(ApnsSendOutcome::Delivered);
        }
        if matches!(response.status, 400 | 410)
            && let Some(reason) = unregistered_reason(&response.body)
        {
            return Ok(ApnsSendOutcome::Unregistered { reason });
        }
        // Non-token errors surface as failures; ensure_success re-derives the
        // error from the status code.
        ensure_success("apns", "send notification", response.status)?;
        Ok(ApnsSendOutcome::Delivered)
    }
}

/// Parse an APNs error body, returning the rejection reason when it indicates
/// the device token is no longer valid (`Unregistered` or `BadDeviceToken`).
fn unregistered_reason(body: &[u8]) -> Option<String> {
    let reason = serde_json::from_slice::<Value>(body)
        .ok()?
        .get("reason")?
        .as_str()?
        .to_string();
    matches!(reason.as_str(), "Unregistered" | "BadDeviceToken").then_some(reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_lifecycle_clients::{NativeHttpResponse, test_support::EC_PRIVATE_KEY};
    use async_trait::async_trait;
    use tokio::sync::Mutex;

    struct MockHttp {
        requests: Mutex<Vec<NativeHttpRequest>>,
        status: u16,
        body: Vec<u8>,
    }

    impl MockHttp {
        fn with_status(status: u16, body: Vec<u8>) -> Arc<Self> {
            Arc::new(Self {
                requests: Mutex::new(Vec::new()),
                status,
                body,
            })
        }

        fn ok() -> Arc<Self> {
            Self::with_status(200, Vec::new())
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
                status: self.status,
                body: self.body.clone(),
            })
        }
    }

    fn config() -> ApnsNativeConfig {
        ApnsNativeConfig {
            team_id: "TEAMID1234".to_string(),
            key_id: "KEYID1234".to_string(),
            bundle_id: "com.example.thinclaw".to_string(),
            private_key_pem: EC_PRIVATE_KEY.to_string(),
            sandbox: true,
        }
    }

    async fn send_spec(http: Arc<MockHttp>, spec: ApnsPushSpec) -> ApnsSendOutcome {
        ApnsPusher::new(config(), http)
            .send("device-token", spec)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn alert_push_sets_alert_headers_and_bare_topic() {
        let http = MockHttp::ok();
        let outcome = send_spec(
            http.clone(),
            ApnsPushSpec::alert(serde_json::json!({"aps": {"alert": "hi"}})),
        )
        .await;
        assert_eq!(outcome, ApnsSendOutcome::Delivered);

        let request = http.take().await.pop().unwrap();
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.url,
            "https://api.sandbox.push.apple.com/3/device/device-token"
        );
        assert!(request.headers["Authorization"].starts_with("bearer "));
        assert_eq!(request.headers["apns-push-type"], "alert");
        assert_eq!(request.headers["apns-priority"], "10");
        assert_eq!(request.headers["apns-topic"], "com.example.thinclaw");
        assert!(!request.headers.contains_key("apns-collapse-id"));
        assert!(!request.headers.contains_key("apns-expiration"));
    }

    #[tokio::test]
    async fn background_push_defaults_to_priority_five() {
        let http = MockHttp::ok();
        send_spec(
            http.clone(),
            ApnsPushSpec::background(serde_json::json!({"aps": {"content-available": 1}})),
        )
        .await;

        let request = http.take().await.pop().unwrap();
        assert_eq!(request.headers["apns-push-type"], "background");
        assert_eq!(request.headers["apns-priority"], "5");
        assert_eq!(request.headers["apns-topic"], "com.example.thinclaw");
    }

    #[tokio::test]
    async fn live_activity_push_uses_topic_suffix_and_priority_five() {
        let http = MockHttp::ok();
        send_spec(
            http.clone(),
            ApnsPushSpec::live_activity(serde_json::json!({"aps": {"event": "update"}})),
        )
        .await;

        let request = http.take().await.pop().unwrap();
        assert_eq!(request.headers["apns-push-type"], "liveactivity");
        assert_eq!(request.headers["apns-priority"], "5");
        assert_eq!(
            request.headers["apns-topic"],
            "com.example.thinclaw.push-type.liveactivity"
        );
    }

    #[tokio::test]
    async fn collapse_id_and_expiration_headers_are_forwarded() {
        let http = MockHttp::ok();
        send_spec(
            http.clone(),
            ApnsPushSpec::alert(serde_json::json!({"aps": {}}))
                .with_collapse_id("run-42")
                .with_expiration(0),
        )
        .await;

        let request = http.take().await.pop().unwrap();
        assert_eq!(request.headers["apns-collapse-id"], "run-42");
        assert_eq!(request.headers["apns-expiration"], "0");
    }

    #[tokio::test]
    async fn payload_is_passed_through_verbatim() {
        let http = MockHttp::ok();
        let payload = serde_json::json!({
            "aps": {"alert": "wake up", "sound": "default"},
            "thinclaw": {"category": "response", "id": "abc"}
        });
        send_spec(http.clone(), ApnsPushSpec::alert(payload.clone())).await;

        let request = http.take().await.pop().unwrap();
        let sent: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(sent, payload);
    }

    #[tokio::test]
    async fn production_host_used_when_not_sandbox() {
        let http = MockHttp::ok();
        let mut config = config();
        config.sandbox = false;
        ApnsPusher::new(config, http.clone())
            .send("device-token", ApnsPushSpec::alert(serde_json::json!({})))
            .await
            .unwrap();

        let request = http.take().await.pop().unwrap();
        assert_eq!(
            request.url,
            "https://api.push.apple.com/3/device/device-token"
        );
    }

    #[tokio::test]
    async fn unregistered_reason_detected_on_410() {
        let http = MockHttp::with_status(410, br#"{"reason":"Unregistered"}"#.to_vec());
        let outcome = send_spec(http, ApnsPushSpec::alert(serde_json::json!({}))).await;
        assert_eq!(
            outcome,
            ApnsSendOutcome::Unregistered {
                reason: "Unregistered".to_string()
            }
        );
    }

    #[tokio::test]
    async fn bad_device_token_reason_detected_on_400() {
        let http = MockHttp::with_status(400, br#"{"reason":"BadDeviceToken"}"#.to_vec());
        let outcome = send_spec(http, ApnsPushSpec::alert(serde_json::json!({}))).await;
        assert_eq!(
            outcome,
            ApnsSendOutcome::Unregistered {
                reason: "BadDeviceToken".to_string()
            }
        );
    }

    #[tokio::test]
    async fn other_400_reasons_surface_as_send_failures() {
        let http = MockHttp::with_status(400, br#"{"reason":"PayloadTooLarge"}"#.to_vec());
        let error = ApnsPusher::new(config(), http)
            .send("device-token", ApnsPushSpec::alert(serde_json::json!({})))
            .await
            .unwrap_err();
        assert!(matches!(error, ChannelError::SendFailed { .. }));
    }

    #[tokio::test]
    async fn server_errors_without_token_reason_surface_as_failures() {
        let http = MockHttp::with_status(503, Vec::new());
        let error = ApnsPusher::new(config(), http)
            .send("device-token", ApnsPushSpec::alert(serde_json::json!({})))
            .await
            .unwrap_err();
        assert!(matches!(error, ChannelError::SendFailed { .. }));
    }
}
