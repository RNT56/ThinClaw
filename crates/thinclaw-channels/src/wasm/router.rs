//! HTTP router for WASM channel webhooks.
//!
//! Routes incoming HTTP requests to the appropriate WASM channel based on
//! registered paths. Handles secret validation at the host level.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{any, get},
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::wasm::schema::WebhookSecretValidation;
use crate::wasm::wrapper::WasmChannel;

/// A registered HTTP endpoint for a WASM channel.
#[derive(Debug, Clone)]
pub struct RegisteredEndpoint {
    /// Channel name that owns this endpoint.
    pub channel_name: String,
    /// HTTP path (e.g., "/webhook/slack").
    pub path: String,
    /// Allowed HTTP methods.
    pub methods: Vec<String>,
    /// Whether secret validation is required.
    pub require_secret: bool,
}

/// Runtime webhook auth configuration for a registered channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegisteredWebhookAuth {
    /// HTTP header name that carries the secret or signature.
    pub secret_header: Option<String>,
    /// POST validation strategy.
    pub secret_validation: WebhookSecretValidation,
    /// Shared secret used for POST validation.
    pub signature_secret: Option<String>,
    /// Query parameter name used for GET/HEAD verification.
    pub verify_token_param: Option<String>,
    /// Shared secret used for GET/HEAD verification.
    pub verify_token_secret: Option<String>,
}

impl RegisteredWebhookAuth {
    fn has_any_secret(&self) -> bool {
        self.signature_secret.is_some() || self.verify_token_secret.is_some()
    }
}

/// Router for WASM channel HTTP endpoints.
pub struct WasmChannelRouter {
    /// Registered channels by name.
    channels: RwLock<HashMap<String, Arc<WasmChannel>>>,
    /// Path to channel mapping for fast lookup.
    path_to_channel: RwLock<HashMap<String, String>>,
    /// Endpoint metadata keyed by path.
    endpoints: RwLock<HashMap<String, RegisteredEndpoint>>,
    /// Webhook auth config keyed by channel name.
    webhook_auth: RwLock<HashMap<String, RegisteredWebhookAuth>>,
}

impl WasmChannelRouter {
    /// Create a new router.
    pub fn new() -> Self {
        Self {
            channels: RwLock::new(HashMap::new()),
            path_to_channel: RwLock::new(HashMap::new()),
            endpoints: RwLock::new(HashMap::new()),
            webhook_auth: RwLock::new(HashMap::new()),
        }
    }

    /// Register a channel with its endpoints.
    ///
    /// # Arguments
    /// * `channel` - The WASM channel to register
    /// * `endpoints` - HTTP endpoints to register for this channel
    /// * `auth` - Runtime webhook auth config and secrets for the channel
    pub async fn register(
        &self,
        channel: Arc<WasmChannel>,
        endpoints: Vec<RegisteredEndpoint>,
        auth: RegisteredWebhookAuth,
    ) {
        let name = channel.channel_name().to_string();

        // Store the channel
        self.channels.write().await.insert(name.clone(), channel);

        // Register path mappings
        let mut path_map = self.path_to_channel.write().await;
        let mut endpoint_map = self.endpoints.write().await;
        endpoint_map.retain(|_, endpoint| endpoint.channel_name != name);
        path_map.retain(|_, channel_name| channel_name != &name);
        for endpoint in endpoints {
            path_map.insert(endpoint.path.clone(), name.clone());
            endpoint_map.insert(endpoint.path.clone(), endpoint.clone());
            tracing::info!(
                channel = %name,
                path = %endpoint.path,
                methods = ?endpoint.methods,
                "Registered WASM channel HTTP endpoint"
            );
        }

        self.webhook_auth.write().await.insert(name, auth);
    }

    /// Get the secret header name for a channel.
    ///
    /// Returns the configured header or "X-Webhook-Secret" as default.
    pub async fn get_secret_header(&self, channel_name: &str) -> String {
        self.webhook_auth
            .read()
            .await
            .get(channel_name)
            .and_then(|auth| auth.secret_header.clone())
            .unwrap_or_else(|| "X-Webhook-Secret".to_string())
    }

    /// Get the full webhook auth config for a channel.
    pub async fn get_webhook_auth(&self, channel_name: &str) -> RegisteredWebhookAuth {
        self.webhook_auth
            .read()
            .await
            .get(channel_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Update webhook auth for an already-registered channel.
    pub async fn update_webhook_auth(&self, channel_name: &str, auth: RegisteredWebhookAuth) {
        self.webhook_auth
            .write()
            .await
            .insert(channel_name.to_string(), auth);
        tracing::info!(
            channel = %channel_name,
            "Updated webhook auth for channel"
        );
    }

    /// Update only the POST webhook secret for an already-registered channel.
    pub async fn update_secret(&self, channel_name: &str, secret: String) {
        let mut auth = self.get_webhook_auth(channel_name).await;
        auth.signature_secret = Some(secret.clone());
        if auth.verify_token_param.is_some() && auth.verify_token_secret.is_none() {
            auth.verify_token_secret = Some(secret);
        }
        self.update_webhook_auth(channel_name, auth).await;
    }

    /// Unregister a channel and its endpoints.
    pub async fn unregister(&self, channel_name: &str) {
        self.channels.write().await.remove(channel_name);
        self.webhook_auth.write().await.remove(channel_name);
        self.endpoints
            .write()
            .await
            .retain(|_, endpoint| endpoint.channel_name != channel_name);

        // Remove all paths for this channel
        self.path_to_channel
            .write()
            .await
            .retain(|_, name| name != channel_name);

        tracing::info!(
            channel = %channel_name,
            "Unregistered WASM channel"
        );
    }

    /// Get the channel for a given path.
    pub async fn get_channel_for_path(&self, path: &str) -> Option<Arc<WasmChannel>> {
        let path_map = self.path_to_channel.read().await;
        let channel_name = path_map.get(path)?;

        self.channels.read().await.get(channel_name).cloned()
    }

    pub async fn get_endpoint_for_path(&self, path: &str) -> Option<RegisteredEndpoint> {
        self.endpoints.read().await.get(path).cloned()
    }

    /// Validate a secret for a channel.
    pub async fn validate_secret(&self, channel_name: &str, provided: &str) -> bool {
        let auth = self.get_webhook_auth(channel_name).await;
        auth.signature_secret
            .as_ref()
            .map(|expected| expected == provided)
            .or_else(|| {
                auth.verify_token_secret
                    .as_ref()
                    .map(|expected| expected == provided)
            })
            .unwrap_or(true)
    }

    /// Check if a channel requires a secret.
    pub async fn requires_secret(&self, channel_name: &str) -> bool {
        self.webhook_auth
            .read()
            .await
            .get(channel_name)
            .map(RegisteredWebhookAuth::has_any_secret)
            .unwrap_or(false)
    }

    /// List all registered channels.
    pub async fn list_channels(&self) -> Vec<String> {
        self.channels.read().await.keys().cloned().collect()
    }

    /// List all registered paths.
    pub async fn list_paths(&self) -> Vec<String> {
        self.path_to_channel.read().await.keys().cloned().collect()
    }
}

impl Default for WasmChannelRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared state for the HTTP server.
#[allow(dead_code)]
#[derive(Clone)]
pub struct RouterState {
    router: Arc<WasmChannelRouter>,
}

impl RouterState {
    pub fn new(router: Arc<WasmChannelRouter>) -> Self {
        Self { router }
    }
}

fn verify_signature(secret: &[u8], payload: &[u8], signature: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use subtle::ConstantTimeEq;

    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length");
    mac.update(payload);
    let result = mac.finalize();
    let expected = format!("sha256={}", hex::encode(result.into_bytes()));
    expected.as_bytes().ct_eq(signature.as_bytes()).into()
}

fn json_error_response(status: StatusCode, value: serde_json::Value) -> Response {
    (status, Json(value)).into_response()
}

fn build_raw_http_response(
    status: StatusCode,
    headers: &HashMap<String, String>,
    body: Vec<u8>,
) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;

    for (name, value) in headers {
        let Ok(header_name) = HeaderName::from_bytes(name.as_bytes()) else {
            tracing::warn!(header = %name, "Skipping invalid response header name");
            continue;
        };
        let Ok(header_value) = HeaderValue::from_str(value) else {
            tracing::warn!(header = %name, "Skipping invalid response header value");
            continue;
        };
        response.headers_mut().insert(header_name, header_value);
    }

    response
}

/// Webhook request body for WASM channels.
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct WasmWebhookRequest {
    /// Optional secret for authentication.
    #[serde(default)]
    pub secret: Option<String>,
}

/// Health response.
#[allow(dead_code)]
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: String,
    channels: Vec<String>,
}

/// Handler for health check endpoint.
#[allow(dead_code)]
async fn health_handler(State(state): State<RouterState>) -> impl IntoResponse {
    let channels = state.router.list_channels().await;
    Json(HealthResponse {
        status: "healthy".to_string(),
        channels,
    })
}

/// Generic webhook handler that routes to the appropriate WASM channel.
async fn webhook_handler(
    State(state): State<RouterState>,
    method: Method,
    Path(path): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let full_path = format!("/webhook/{}", path);

    tracing::info!(
        method = %method,
        path = %full_path,
        body_len = body.len(),
        "Webhook request received"
    );

    let endpoint = match state.router.get_endpoint_for_path(&full_path).await {
        Some(endpoint) => endpoint,
        None => {
            tracing::warn!(path = %full_path, "No endpoint registered for webhook path");
            return json_error_response(
                StatusCode::NOT_FOUND,
                serde_json::json!({
                    "error": "Channel not found for path",
                    "path": full_path
                }),
            );
        }
    };

    let allowed_methods: Vec<String> = endpoint
        .methods
        .iter()
        .map(|value| value.to_ascii_uppercase())
        .collect();
    if !allowed_methods.is_empty()
        && !allowed_methods
            .iter()
            .any(|allowed| allowed == method.as_str())
    {
        return json_error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            serde_json::json!({
                "error": "HTTP method not allowed",
                "allowed_methods": allowed_methods,
            }),
        );
    }

    // Find the channel for this path
    let channel = match state.router.get_channel_for_path(&full_path).await {
        Some(c) => c,
        None => {
            tracing::warn!(
                path = %full_path,
                "No channel registered for webhook path"
            );
            return json_error_response(
                StatusCode::NOT_FOUND,
                serde_json::json!({
                    "error": "Channel not found for path",
                    "path": full_path
                }),
            );
        }
    };

    tracing::info!(
        channel = %channel.channel_name(),
        "Found channel for webhook"
    );

    let channel_name = channel.channel_name();

    let secret_validated = if endpoint.require_secret {
        let auth = state.router.get_webhook_auth(channel_name).await;
        let is_verify_request = matches!(method, Method::GET | Method::HEAD);

        tracing::debug!(
            channel = %channel_name,
            method = %method,
            secret_validation = ?auth.secret_validation,
            has_signature_secret = auth.signature_secret.is_some(),
            has_verify_token_secret = auth.verify_token_secret.is_some(),
            verify_token_param = ?auth.verify_token_param,
            "Checking webhook secret"
        );

        if is_verify_request {
            let verify_param = auth.verify_token_param.as_deref().unwrap_or("secret");
            let provided = query.get(verify_param).cloned().or_else(|| {
                if verify_param != "secret" {
                    query.get("secret").cloned()
                } else {
                    None
                }
            });

            let Some(provided) = provided else {
                tracing::warn!(
                    channel = %channel_name,
                    param = %verify_param,
                    "Webhook verify token required but not provided"
                );
                return json_error_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({
                        "error": "Webhook verify token required",
                        "param": verify_param,
                    }),
                );
            };

            let expected = auth
                .verify_token_secret
                .as_deref()
                .or(auth.signature_secret.as_deref());

            let Some(expected) = expected else {
                tracing::warn!(
                    channel = %channel_name,
                    "Webhook verify token requested but no secret is configured"
                );
                return json_error_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({
                        "error": "Webhook verify token is not configured"
                    }),
                );
            };

            if expected != provided {
                tracing::warn!(
                    channel = %channel_name,
                    "Webhook verify token validation failed"
                );
                return json_error_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({
                        "error": "Invalid webhook verify token"
                    }),
                );
            }

            tracing::debug!(channel = %channel_name, "Webhook verify token validated");
            true
        } else {
            let secret_header_name = auth.secret_header.as_deref().unwrap_or("X-Webhook-Secret");
            let provided = headers
                .get(secret_header_name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
                .or_else(|| {
                    if secret_header_name != "X-Webhook-Secret" {
                        headers
                            .get("X-Webhook-Secret")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_owned)
                    } else {
                        None
                    }
                })
                .or_else(|| query.get("secret").cloned());

            let Some(provided) = provided else {
                tracing::warn!(
                    channel = %channel_name,
                    header = %secret_header_name,
                    "Webhook secret required but not provided"
                );
                return json_error_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({
                        "error": "Webhook secret required",
                        "header": secret_header_name,
                    }),
                );
            };

            let expected = auth
                .signature_secret
                .as_deref()
                .or(auth.verify_token_secret.as_deref());
            let Some(expected) = expected else {
                tracing::warn!(
                    channel = %channel_name,
                    "Webhook secret requested but no secret is configured"
                );
                return json_error_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({
                        "error": "Webhook secret is not configured"
                    }),
                );
            };

            let valid = match auth.secret_validation {
                WebhookSecretValidation::Equals => expected == provided,
                WebhookSecretValidation::HmacSha256Body => {
                    verify_signature(expected.as_bytes(), &body, &provided)
                }
            };

            if !valid {
                tracing::warn!(
                    channel = %channel_name,
                    validation = ?auth.secret_validation,
                    "Webhook secret validation failed"
                );
                return json_error_response(
                    StatusCode::UNAUTHORIZED,
                    serde_json::json!({
                        "error": match auth.secret_validation {
                            WebhookSecretValidation::Equals => "Invalid webhook secret",
                            WebhookSecretValidation::HmacSha256Body => "Invalid webhook signature",
                        }
                    }),
                );
            }

            tracing::debug!(channel = %channel_name, "Webhook secret validated");
            true
        }
    } else {
        false
    };

    // Convert headers to HashMap
    let headers_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    tracing::info!(
        channel = %channel_name,
        secret_validated = secret_validated,
        "Calling WASM channel on_http_request"
    );

    match channel
        .call_on_http_request(
            method.as_str(),
            &full_path,
            &headers_map,
            &query,
            &body,
            secret_validated,
        )
        .await
    {
        Ok(response) => {
            let status =
                StatusCode::from_u16(response.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

            tracing::info!(
                channel = %channel_name,
                status = %status,
                body_len = response.body.len(),
                "WASM channel on_http_request completed successfully"
            );

            build_raw_http_response(status, &response.headers, response.body)
        }
        Err(e) => {
            tracing::error!(
                channel = %channel_name,
                error = %e,
                "WASM channel callback failed"
            );
            json_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({
                    "error": "Channel callback failed",
                    "details": e.to_string()
                }),
            )
        }
    }
}

/// Create an Axum router for WASM channel webhooks.
///
/// This router can be merged with the existing HTTP channel router.
pub fn create_wasm_channel_router(router: Arc<WasmChannelRouter>) -> Router {
    let state = RouterState::new(router);
    Router::new()
        .route("/wasm-channels/health", get(health_handler))
        // Catch-all for webhook paths
        .route("/webhook/{*path}", any(webhook_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::{Body, Bytes, to_bytes};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use super::{build_raw_http_response, create_wasm_channel_router};
    use crate::pairing::PairingStore;
    use crate::wasm::capabilities::ChannelCapabilities;
    use crate::wasm::router::{RegisteredEndpoint, RegisteredWebhookAuth, WasmChannelRouter};
    use crate::wasm::runtime::{
        PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig,
    };
    use crate::wasm::schema::WebhookSecretValidation;
    use crate::wasm::wrapper::WasmChannel;

    fn sign_payload(secret: &[u8], payload: &[u8]) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        type HmacSha256 = Hmac<Sha256>;

        let mut mac =
            HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length");
        mac.update(payload);
        let result = mac.finalize();
        format!("sha256={}", hex::encode(result.into_bytes()))
    }

    fn create_test_channel(name: &str) -> Arc<WasmChannel> {
        let config = WasmChannelRuntimeConfig::for_testing();
        let runtime = Arc::new(WasmChannelRuntime::new(config).unwrap());

        let prepared = Arc::new(PreparedChannelModule::for_testing(
            name,
            format!("Test channel: {}", name),
        ));

        let capabilities =
            ChannelCapabilities::for_channel(name).with_path(format!("/webhook/{}", name));

        Arc::new(WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
            None,
            Arc::new(PairingStore::new()),
        ))
    }

    #[tokio::test]
    async fn test_router_register_and_lookup() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("slack");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "slack".to_string(),
            path: "/webhook/slack".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: true,
        }];

        router
            .register(
                channel,
                endpoints,
                RegisteredWebhookAuth {
                    signature_secret: Some("secret123".to_string()),
                    ..Default::default()
                },
            )
            .await;

        // Should find channel by path
        let found = router.get_channel_for_path("/webhook/slack").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().channel_name(), "slack");

        // Should not find non-existent path
        let not_found = router.get_channel_for_path("/webhook/telegram").await;
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_router_secret_validation() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("slack");

        router
            .register(
                channel,
                vec![],
                RegisteredWebhookAuth {
                    signature_secret: Some("secret123".to_string()),
                    ..Default::default()
                },
            )
            .await;

        // Correct secret
        assert!(router.validate_secret("slack", "secret123").await);

        // Wrong secret
        assert!(!router.validate_secret("slack", "wrong").await);

        // Channel without secret always validates
        let channel2 = create_test_channel("telegram");
        router
            .register(channel2, vec![], RegisteredWebhookAuth::default())
            .await;
        assert!(router.validate_secret("telegram", "anything").await);
    }

    #[tokio::test]
    async fn test_router_unregister() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("slack");

        let endpoints = vec![RegisteredEndpoint {
            channel_name: "slack".to_string(),
            path: "/webhook/slack".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: false,
        }];

        router
            .register(channel, endpoints, RegisteredWebhookAuth::default())
            .await;

        // Should exist
        assert!(
            router
                .get_channel_for_path("/webhook/slack")
                .await
                .is_some()
        );

        // Unregister
        router.unregister("slack").await;

        // Should no longer exist
        assert!(
            router
                .get_channel_for_path("/webhook/slack")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_router_list_channels() {
        let router = WasmChannelRouter::new();

        let channel1 = create_test_channel("slack");
        let channel2 = create_test_channel("telegram");

        router
            .register(channel1, vec![], RegisteredWebhookAuth::default())
            .await;
        router
            .register(channel2, vec![], RegisteredWebhookAuth::default())
            .await;

        let channels = router.list_channels().await;
        assert_eq!(channels.len(), 2);
        assert!(channels.contains(&"slack".to_string()));
        assert!(channels.contains(&"telegram".to_string()));
    }

    #[tokio::test]
    async fn test_router_secret_header() {
        let router = WasmChannelRouter::new();
        let channel = create_test_channel("telegram");

        // Register with custom secret header
        router
            .register(
                channel,
                vec![],
                RegisteredWebhookAuth {
                    secret_header: Some("X-Telegram-Bot-Api-Secret-Token".to_string()),
                    signature_secret: Some("secret123".to_string()),
                    secret_validation: WebhookSecretValidation::Equals,
                    ..Default::default()
                },
            )
            .await;

        // Should return the custom header
        assert_eq!(
            router.get_secret_header("telegram").await,
            "X-Telegram-Bot-Api-Secret-Token"
        );

        // Channel without custom header should use default
        let channel2 = create_test_channel("slack");
        router
            .register(
                channel2,
                vec![],
                RegisteredWebhookAuth {
                    signature_secret: Some("secret456".to_string()),
                    ..Default::default()
                },
            )
            .await;
        assert_eq!(router.get_secret_header("slack").await, "X-Webhook-Secret");
    }

    #[tokio::test]
    async fn test_get_verify_token_validation() {
        let router = Arc::new(WasmChannelRouter::new());
        let channel = create_test_channel("whatsapp");
        let endpoints = vec![RegisteredEndpoint {
            channel_name: "whatsapp".to_string(),
            path: "/webhook/whatsapp".to_string(),
            methods: vec!["GET".to_string(), "POST".to_string()],
            require_secret: true,
        }];

        router
            .register(
                channel,
                endpoints,
                RegisteredWebhookAuth {
                    verify_token_param: Some("hub.verify_token".to_string()),
                    verify_token_secret: Some("verify-token".to_string()),
                    secret_header: Some("X-Hub-Signature-256".to_string()),
                    secret_validation: WebhookSecretValidation::HmacSha256Body,
                    signature_secret: Some("app-secret".to_string()),
                },
            )
            .await;

        let app = create_wasm_channel_router(Arc::clone(&router));
        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/webhook/whatsapp?hub.verify_token=verify-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);

        let invalid = app
            .oneshot(
                Request::builder()
                    .uri("/webhook/whatsapp?hub.verify_token=wrong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_post_hmac_validation() {
        let router = Arc::new(WasmChannelRouter::new());
        let channel = create_test_channel("whatsapp");
        let endpoints = vec![RegisteredEndpoint {
            channel_name: "whatsapp".to_string(),
            path: "/webhook/whatsapp".to_string(),
            methods: vec!["POST".to_string()],
            require_secret: true,
        }];

        router
            .register(
                channel,
                endpoints,
                RegisteredWebhookAuth {
                    secret_header: Some("X-Hub-Signature-256".to_string()),
                    secret_validation: WebhookSecretValidation::HmacSha256Body,
                    signature_secret: Some("app-secret".to_string()),
                    ..Default::default()
                },
            )
            .await;

        let app = create_wasm_channel_router(Arc::clone(&router));
        let body = Bytes::from_static(br#"{"entry":[]}"#);
        let signature = sign_payload(b"app-secret", &body);

        let ok = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/whatsapp")
                    .header("X-Hub-Signature-256", signature)
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);

        let invalid = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/webhook/whatsapp")
                    .header("X-Hub-Signature-256", "sha256=deadbeef")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_raw_response_helper_preserves_headers_and_body() {
        let response = build_raw_http_response(
            StatusCode::OK,
            &HashMap::from([("Content-Type".to_string(), "text/plain".to_string())]),
            b"challenge".to_vec(),
        );

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "text/plain");
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], b"challenge");
    }
}
