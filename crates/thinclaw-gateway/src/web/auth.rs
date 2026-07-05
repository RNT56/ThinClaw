//! Bearer token authentication middleware for the web gateway.
//!
//! Supports two authentication modes:
//! 1. **Bearer token** (default): `Authorization: Bearer <token>` header or `?token=<token>` query param
//! 2. **Trusted proxy** (optional): When `TRUSTED_PROXY_HEADER` env var is set, the gateway
//!    trusts that header (e.g., `X-Forwarded-User`) as the authenticated identity. This mode
//!    requires `TRUSTED_PROXY_IPS` to restrict which source IPs can use it (CIDR notation,
//!    comma-separated). If TRUSTED_PROXY_IPS is empty/unset, only loopback IPs are trusted.

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::{IpAddr, SocketAddr};

use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use thinclaw_settings::{GatewayPrincipalConfig, GatewayRole};

use crate::web::identity::{GatewayAuthSource, GatewayRequestIdentity};
use crate::web::ports::IdentityLookupPort;
use crate::web::rbac::role_allows_request;

/// Where a presented bearer token came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenSource {
    Header,
    Query,
}

impl TokenSource {
    fn auth_source(self) -> GatewayAuthSource {
        match self {
            TokenSource::Header => GatewayAuthSource::BearerHeader,
            TokenSource::Query => GatewayAuthSource::BearerQuery,
        }
    }
}

/// Shared auth state injected via axum middleware state.
#[derive(Clone)]
pub struct AuthState {
    pub token: String,
    /// Header name for trusted-proxy mode (e.g., "X-Forwarded-User").
    /// When set, requests with this header from trusted IPs are accepted without a bearer token.
    pub trusted_proxy_header: Option<String>,
    /// IP addresses allowed to use trusted-proxy auth.
    /// If empty, only loopback addresses (127.0.0.1, ::1) are trusted.
    pub trusted_proxy_ips: Vec<IpNet>,
    /// Default gateway principal when auth alone cannot identify the caller.
    pub fallback_principal_id: String,
    /// Default gateway actor when auth alone cannot identify the caller.
    pub fallback_actor_id: String,
    /// Optional store so bearer-token requests can infer the primary principal.
    pub store: Option<Arc<dyn IdentityLookupPort>>,
    /// Extra RBAC principals layered on top of the primary token. Each has its
    /// own token and role; the primary `token` above always authenticates as
    /// [`thinclaw_settings::GatewayRole::Admin`]. Empty by default, so RBAC is
    /// inactive unless the operator configures principals.
    pub principals: Vec<GatewayPrincipalConfig>,
}

/// Check if an IP is trusted for proxy auth.
///
/// Each trusted entry is a network (CIDR); a bare host is a /32 or /128. An IP
/// is trusted if it is contained in any configured network — so `10.0.0.0/8`
/// trusts the whole subnet, not just `10.0.0.0`.
fn is_trusted_ip(ip: &IpAddr, trusted_ips: &[IpNet]) -> bool {
    if trusted_ips.is_empty() {
        // Default: only trust loopback
        return ip.is_loopback();
    }
    trusted_ips.iter().any(|net| net.contains(ip))
}

/// Parse a single `TRUSTED_PROXY_IPS` entry: a CIDR network (`10.0.0.0/8`) or a
/// bare host address (`10.0.0.1`, treated as `/32` / `/128`). Returns `None` for
/// empty or malformed entries.
fn parse_trusted_proxy_entry(entry: &str) -> Option<IpNet> {
    if entry.is_empty() {
        return None;
    }
    if entry.contains('/') {
        return entry.parse::<IpNet>().ok();
    }
    match entry.parse::<IpAddr>().ok()? {
        IpAddr::V4(v4) => Ipv4Net::new(v4, 32).ok().map(IpNet::V4),
        IpAddr::V6(v6) => Ipv6Net::new(v6, 128).ok().map(IpNet::V6),
    }
}

/// Load trusted-proxy configuration from environment variables.
pub fn load_trusted_proxy_config() -> (Option<String>, Vec<IpNet>) {
    let header = std::env::var("TRUSTED_PROXY_HEADER")
        .ok()
        .filter(|s| !s.is_empty());
    let ips = std::env::var("TRUSTED_PROXY_IPS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| parse_trusted_proxy_entry(s.trim()))
        .collect();
    (header, ips)
}

/// Auth middleware that validates bearer token from header or query param,
/// with optional trusted-proxy auth mode.
///
/// SSE connections can't set headers from `EventSource`, so we also accept
/// `?token=xxx` as a query parameter.
pub async fn auth_middleware(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let mut request = request;
    // Check trusted-proxy mode first (if configured)
    if let Some(ref proxy_header) = auth.trusted_proxy_header {
        let source_ip = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip());

        if let Some(ip) = source_ip
            && is_trusted_ip(&ip, &auth.trusted_proxy_ips)
            && let Some(user_header) = headers.get(proxy_header.as_str())
            && let Ok(principal_id) = user_header.to_str()
        {
            let principal_id = principal_id.trim();
            if principal_id.is_empty() {
                return (StatusCode::UNAUTHORIZED, "Trusted proxy identity was empty")
                    .into_response();
            }
            let actor_id = principal_id.to_string();
            // A trusted upstream proxy has authenticated the user; grant admin,
            // preserving pre-RBAC trusted-proxy behavior.
            let identity = GatewayRequestIdentity::new(
                principal_id,
                actor_id,
                GatewayAuthSource::TrustedProxy,
                false,
            )
            .with_role(GatewayRole::Admin);
            if let Some(denied) = enforce_capability(&request, &identity) {
                return denied;
            }
            request.extensions_mut().insert(identity);
            tracing::debug!(
                proxy_header = %proxy_header,
                source_ip = %ip,
                "Trusted-proxy auth accepted"
            );
            return next.run(request).await;
        }
    }

    // Collect presented tokens in precedence order: Authorization header first,
    // then the `?token=` query fallback for SSE EventSource clients that cannot
    // set headers. Owned copies so the borrow of `headers`/`request` ends before
    // we mutate the request below.
    let mut candidates: Vec<(String, TokenSource)> = Vec::new();
    if let Some(token) = header_bearer_token(&headers) {
        candidates.push((token, TokenSource::Header));
    }
    if let Some(token) = query_token(request.uri()) {
        candidates.push((token, TokenSource::Query));
    }

    for (token, source) in &candidates {
        // Primary token → full admin rights (backward compatible).
        if ct_eq_str(token, &auth.token) {
            if *source == TokenSource::Query {
                warn_query_token_auth();
            }
            let identity = fallback_request_identity(&auth, source.auth_source()).await;
            if let Some(denied) = enforce_capability(&request, &identity) {
                return denied;
            }
            request.extensions_mut().insert(identity);
            return next.run(request).await;
        }
        // Extra RBAC principals → scoped identity + role.
        for principal in &auth.principals {
            if ct_eq_str(token, &principal.token) {
                if *source == TokenSource::Query {
                    warn_query_token_auth();
                }
                let identity = GatewayRequestIdentity::new(
                    principal.principal_id.clone(),
                    principal.effective_actor_id().to_string(),
                    source.auth_source(),
                    false,
                )
                .with_role(principal.role);
                if let Some(denied) = enforce_capability(&request, &identity) {
                    return denied;
                }
                request.extensions_mut().insert(identity);
                return next.run(request).await;
            }
        }
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

/// Extract a `Bearer <token>` value from the Authorization header, if present.
fn header_bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::to_string)
}

/// Extract a `token=<value>` query parameter, if present.
fn query_token(uri: &Uri) -> Option<String> {
    uri.query()?
        .split('&')
        .find_map(|pair| pair.strip_prefix("token=").map(str::to_string))
}

/// Constant-time string comparison (differing lengths short-circuit to false,
/// matching the prior bearer-compare behavior).
fn ct_eq_str(presented: &str, expected: &str) -> bool {
    bool::from(presented.as_bytes().ct_eq(expected.as_bytes()))
}

/// Warn once that a token was accepted via `?token=` (RFC 6750 §2.3: tokens in
/// URLs can leak via access logs, proxies, and Referer headers). This path
/// exists only for SSE EventSource clients that cannot set a header.
fn warn_query_token_auth() {
    static QUERY_AUTH_WARNED: std::sync::Once = std::sync::Once::new();
    QUERY_AUTH_WARNED.call_once(|| {
        tracing::warn!(
            "gateway accepted bearer auth via `?token=` query parameter \
             (SSE EventSource fallback); per RFC 6750 §2.3 tokens in URLs \
             can leak through logs/proxies/Referer — prefer the \
             Authorization header where the client supports it"
        );
    });
}

/// Enforce the request's RBAC capability against the caller's role. Returns
/// `Some(403)` when the role is insufficient, `None` when allowed. Admin roles
/// always pass, so this is a no-op for the primary token and trusted proxies.
fn enforce_capability(request: &Request, identity: &GatewayRequestIdentity) -> Option<Response> {
    if role_allows_request(identity.role, request.method(), request.uri().path()) {
        return None;
    }
    tracing::debug!(
        principal = %identity.principal_id,
        role = %identity.role.as_str(),
        method = %request.method(),
        path = %request.uri().path(),
        "RBAC denied request"
    );
    Some(
        (
            StatusCode::FORBIDDEN,
            "Insufficient role for this operation",
        )
            .into_response(),
    )
}

async fn fallback_request_identity(
    auth: &AuthState,
    auth_source: GatewayAuthSource,
) -> GatewayRequestIdentity {
    let principal_id = if !auth.fallback_principal_id.trim().is_empty()
        && auth.fallback_principal_id != "default"
    {
        auth.fallback_principal_id.clone()
    } else if let Some(store) = auth.store.as_ref() {
        match store.infer_primary_user_id_for_channel("gateway").await {
            Ok(Some(inferred)) if !inferred.trim().is_empty() => inferred,
            Ok(_) | Err(_) => auth.fallback_principal_id.clone(),
        }
    } else {
        auth.fallback_principal_id.clone()
    };
    let actor_id = default_gateway_actor_id_from_auth(auth, &principal_id);
    // The primary gateway token is the admin credential.
    GatewayRequestIdentity::new(principal_id, actor_id, auth_source, true)
        .with_role(GatewayRole::Admin)
}

fn default_gateway_actor_id_from_auth(auth: &AuthState, principal_id: &str) -> String {
    if auth.fallback_actor_id.trim().is_empty()
        || auth.fallback_actor_id == auth.fallback_principal_id
    {
        principal_id.to_string()
    } else {
        auth.fallback_actor_id.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_clone() {
        let state = AuthState {
            token: "test-token".to_string(),
            trusted_proxy_header: None,
            trusted_proxy_ips: vec![],
            fallback_principal_id: "test-user".to_string(),
            fallback_actor_id: "test-actor".to_string(),
            store: None,
            principals: vec![],
        };
        let cloned = state.clone();
        assert_eq!(cloned.token, "test-token");
    }

    #[test]
    fn test_is_trusted_ip_loopback_default() {
        // When no trusted IPs configured, loopback is trusted
        assert!(is_trusted_ip(&"127.0.0.1".parse().unwrap(), &[]));
        assert!(is_trusted_ip(&"::1".parse().unwrap(), &[]));
        assert!(!is_trusted_ip(&"192.168.1.1".parse().unwrap(), &[]));
    }

    #[test]
    fn test_is_trusted_ip_explicit_list() {
        let trusted = vec![
            parse_trusted_proxy_entry("10.0.0.1").unwrap(),
            parse_trusted_proxy_entry("10.0.0.2").unwrap(),
        ];
        assert!(is_trusted_ip(&"10.0.0.1".parse().unwrap(), &trusted));
        assert!(is_trusted_ip(&"10.0.0.2".parse().unwrap(), &trusted));
        assert!(!is_trusted_ip(&"10.0.0.3".parse().unwrap(), &trusted));
        // Loopback not in explicit list
        assert!(!is_trusted_ip(&"127.0.0.1".parse().unwrap(), &trusted));
    }

    #[test]
    fn test_is_trusted_ip_cidr_subnet() {
        // A CIDR entry must trust the whole subnet, not just the base address —
        // this is the regression the single-IP CIDR-stripping bug introduced.
        let trusted = vec![parse_trusted_proxy_entry("10.0.0.0/8").unwrap()];
        assert!(is_trusted_ip(&"10.0.0.0".parse().unwrap(), &trusted));
        assert!(is_trusted_ip(&"10.1.2.3".parse().unwrap(), &trusted));
        assert!(is_trusted_ip(&"10.255.255.255".parse().unwrap(), &trusted));
        assert!(!is_trusted_ip(&"11.0.0.1".parse().unwrap(), &trusted));
        assert!(!is_trusted_ip(&"192.168.0.1".parse().unwrap(), &trusted));
    }

    #[test]
    fn test_parse_trusted_proxy_entry() {
        assert!(parse_trusted_proxy_entry("").is_none());
        assert!(parse_trusted_proxy_entry("not-an-ip").is_none());
        assert_eq!(
            parse_trusted_proxy_entry("10.0.0.1").unwrap(),
            "10.0.0.1/32".parse::<IpNet>().unwrap()
        );
        assert_eq!(
            parse_trusted_proxy_entry("10.0.0.0/8").unwrap(),
            "10.0.0.0/8".parse::<IpNet>().unwrap()
        );
    }

    #[test]
    fn test_auth_state_with_proxy() {
        let state = AuthState {
            token: "my-token".to_string(),
            trusted_proxy_header: Some("X-Forwarded-User".to_string()),
            trusted_proxy_ips: vec![parse_trusted_proxy_entry("10.0.0.1").unwrap()],
            fallback_principal_id: "user-1".to_string(),
            fallback_actor_id: "actor-1".to_string(),
            store: None,
            principals: vec![],
        };
        assert_eq!(
            state.trusted_proxy_header.as_deref(),
            Some("X-Forwarded-User")
        );
        assert_eq!(state.trusted_proxy_ips.len(), 1);
    }

    #[tokio::test]
    async fn fallback_identity_marks_compatibility_mode() {
        let state = AuthState {
            token: "my-token".to_string(),
            trusted_proxy_header: None,
            trusted_proxy_ips: vec![],
            fallback_principal_id: "user-1".to_string(),
            fallback_actor_id: "actor-1".to_string(),
            store: None,
            principals: vec![],
        };

        let identity = fallback_request_identity(&state, GatewayAuthSource::BearerHeader).await;
        assert_eq!(identity.principal_id, "user-1");
        assert_eq!(identity.actor_id, "actor-1");
        assert!(identity.compatibility_fallback);
        assert_eq!(identity.auth_source.as_str(), "bearer_header");
        // The primary token is the admin credential.
        assert_eq!(identity.role, GatewayRole::Admin);
    }

    // --- End-to-end middleware tests (token → identity+role → capability gate) ---

    mod middleware {
        use super::*;
        use axum::{
            Router,
            body::Body,
            http::Method,
            middleware::from_fn_with_state,
            routing::{get, post},
        };
        use thinclaw_settings::{GatewayPrincipalConfig, GatewayRole};
        use tower::ServiceExt;

        async fn ok_handler() -> StatusCode {
            StatusCode::OK
        }

        fn base_auth(principals: Vec<GatewayPrincipalConfig>) -> AuthState {
            AuthState {
                token: "admin-token".to_string(),
                trusted_proxy_header: None,
                trusted_proxy_ips: vec![],
                fallback_principal_id: "root".to_string(),
                fallback_actor_id: "root".to_string(),
                store: None,
                principals,
            }
        }

        fn principal(token: &str, id: &str, role: GatewayRole) -> GatewayPrincipalConfig {
            GatewayPrincipalConfig {
                token: token.to_string(),
                principal_id: id.to_string(),
                actor_id: None,
                role,
            }
        }

        /// Drive one request through the auth middleware; returns the status.
        /// `bearer` sets an `Authorization: Bearer` header; `query` appends
        /// `?token=` to exercise the SSE fallback path.
        async fn status(
            auth: &AuthState,
            method: Method,
            path: &str,
            bearer: Option<&str>,
            query_token: Option<&str>,
        ) -> StatusCode {
            let app = Router::new()
                .route("/api/chat/history", get(ok_handler))
                .route("/api/chat/send", post(ok_handler))
                .route("/api/settings/save", post(ok_handler))
                .route_layer(from_fn_with_state(auth.clone(), auth_middleware));

            let uri = match query_token {
                Some(tok) => format!("{path}?token={tok}"),
                None => path.to_string(),
            };
            let mut builder = Request::builder().method(method).uri(uri);
            if let Some(tok) = bearer {
                builder = builder.header("authorization", format!("Bearer {tok}"));
            }
            let request = builder.body(Body::empty()).unwrap();
            app.oneshot(request).await.unwrap().status()
        }

        #[tokio::test]
        async fn primary_token_is_admin_everywhere() {
            let auth = base_auth(vec![]);
            assert_eq!(
                status(
                    &auth,
                    Method::GET,
                    "/api/chat/history",
                    Some("admin-token"),
                    None
                )
                .await,
                StatusCode::OK
            );
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/chat/send",
                    Some("admin-token"),
                    None
                )
                .await,
                StatusCode::OK
            );
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/settings/save",
                    Some("admin-token"),
                    None
                )
                .await,
                StatusCode::OK
            );
        }

        #[tokio::test]
        async fn readonly_principal_reads_only() {
            let auth = base_auth(vec![principal("ro-token", "alice", GatewayRole::ReadOnly)]);
            // Read allowed.
            assert_eq!(
                status(
                    &auth,
                    Method::GET,
                    "/api/chat/history",
                    Some("ro-token"),
                    None
                )
                .await,
                StatusCode::OK
            );
            // Non-admin write forbidden.
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/chat/send",
                    Some("ro-token"),
                    None
                )
                .await,
                StatusCode::FORBIDDEN
            );
            // Admin surface forbidden.
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/settings/save",
                    Some("ro-token"),
                    None
                )
                .await,
                StatusCode::FORBIDDEN
            );
        }

        #[tokio::test]
        async fn operator_principal_chats_but_not_config() {
            let auth = base_auth(vec![principal("op-token", "bob", GatewayRole::Operator)]);
            assert_eq!(
                status(
                    &auth,
                    Method::GET,
                    "/api/chat/history",
                    Some("op-token"),
                    None
                )
                .await,
                StatusCode::OK
            );
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/chat/send",
                    Some("op-token"),
                    None
                )
                .await,
                StatusCode::OK
            );
            // Config surface still Admin-only.
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/settings/save",
                    Some("op-token"),
                    None
                )
                .await,
                StatusCode::FORBIDDEN
            );
        }

        #[tokio::test]
        async fn explicit_admin_principal_has_full_access() {
            let auth = base_auth(vec![principal("admin2-token", "carol", GatewayRole::Admin)]);
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/settings/save",
                    Some("admin2-token"),
                    None
                )
                .await,
                StatusCode::OK
            );
        }

        #[tokio::test]
        async fn unknown_and_missing_tokens_are_unauthorized() {
            let auth = base_auth(vec![principal("ro-token", "alice", GatewayRole::ReadOnly)]);
            assert_eq!(
                status(&auth, Method::GET, "/api/chat/history", Some("nope"), None).await,
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                status(&auth, Method::GET, "/api/chat/history", None, None).await,
                StatusCode::UNAUTHORIZED
            );
        }

        #[tokio::test]
        async fn rbac_applies_over_query_token_fallback() {
            let auth = base_auth(vec![principal("ro-token", "alice", GatewayRole::ReadOnly)]);
            // ReadOnly via ?token= can read...
            assert_eq!(
                status(
                    &auth,
                    Method::GET,
                    "/api/chat/history",
                    None,
                    Some("ro-token")
                )
                .await,
                StatusCode::OK
            );
            // ...but the role still blocks a write over the same fallback.
            assert_eq!(
                status(
                    &auth,
                    Method::POST,
                    "/api/chat/send",
                    None,
                    Some("ro-token")
                )
                .await,
                StatusCode::FORBIDDEN
            );
        }

        #[tokio::test]
        async fn wrong_header_falls_back_to_valid_query_token() {
            // Backward-compat: a present-but-wrong header must not prevent the
            // valid `?token=` fallback from authenticating.
            let auth = base_auth(vec![]);
            assert_eq!(
                status(
                    &auth,
                    Method::GET,
                    "/api/chat/history",
                    Some("wrong"),
                    Some("admin-token")
                )
                .await,
                StatusCode::OK
            );
        }

        #[tokio::test]
        async fn trusted_proxy_identity_is_admin() {
            use axum::extract::ConnectInfo;
            use std::net::SocketAddr;

            let mut auth = base_auth(vec![]);
            auth.trusted_proxy_header = Some("x-forwarded-user".to_string());
            // Empty trusted_proxy_ips ⇒ only loopback is trusted.

            let app = Router::new()
                .route("/api/settings/save", post(ok_handler))
                .route_layer(from_fn_with_state(auth.clone(), auth_middleware));

            let mut request = Request::builder()
                .method(Method::POST)
                .uri("/api/settings/save")
                .header("x-forwarded-user", "proxied-alice")
                .body(Body::empty())
                .unwrap();
            request
                .extensions_mut()
                .insert(ConnectInfo("127.0.0.1:5555".parse::<SocketAddr>().unwrap()));

            // Trusted proxy from loopback → admin → may reach an admin surface.
            let response = app.oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    }
}
