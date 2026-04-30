//! Bearer token authentication middleware for the web gateway.
//!
//! Supports two authentication modes:
//! 1. **Bearer token** (default): `Authorization: Bearer <token>` header or `?token=<token>` query param
//! 2. **Trusted proxy** (optional): When `TRUSTED_PROXY_HEADER` env var is set, the gateway
//!    trusts that header (e.g., `X-Forwarded-User`) as the authenticated identity. This mode
//!    requires `TRUSTED_PROXY_IPS` to restrict which source IPs can use it (CIDR notation,
//!    comma-separated). If TRUSTED_PROXY_IPS is empty/unset, only loopback IPs are trusted.

use async_trait::async_trait;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use subtle::ConstantTimeEq;

use crate::web::identity::{GatewayAuthSource, GatewayRequestIdentity};

#[async_trait]
pub trait GatewayIdentityStore: Send + Sync {
    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, String>;
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
    pub trusted_proxy_ips: Vec<IpAddr>,
    /// Default gateway principal when auth alone cannot identify the caller.
    pub fallback_principal_id: String,
    /// Default gateway actor when auth alone cannot identify the caller.
    pub fallback_actor_id: String,
    /// Optional store so bearer-token requests can infer the primary principal.
    pub store: Option<Arc<dyn GatewayIdentityStore>>,
}

/// Check if an IP is trusted for proxy auth.
fn is_trusted_ip(ip: &IpAddr, trusted_ips: &[IpAddr]) -> bool {
    if trusted_ips.is_empty() {
        // Default: only trust loopback
        return ip.is_loopback();
    }
    trusted_ips.contains(ip)
}

/// Load trusted-proxy configuration from environment variables.
pub fn load_trusted_proxy_config() -> (Option<String>, Vec<IpAddr>) {
    let header = std::env::var("TRUSTED_PROXY_HEADER")
        .ok()
        .filter(|s| !s.is_empty());
    let ips = std::env::var("TRUSTED_PROXY_IPS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return None;
            }
            // Strip CIDR notation for simple IP comparison
            let ip_str = trimmed.split('/').next().unwrap_or(trimmed);
            ip_str.parse::<IpAddr>().ok()
        })
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
            request.extensions_mut().insert(GatewayRequestIdentity::new(
                principal_id,
                actor_id,
                GatewayAuthSource::TrustedProxy,
                false,
            ));
            tracing::debug!(
                proxy_header = %proxy_header,
                source_ip = %ip,
                "Trusted-proxy auth accepted"
            );
            return next.run(request).await;
        }
    }

    // Try Authorization header (constant-time comparison)
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
        && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
    {
        let identity = fallback_request_identity(&auth, GatewayAuthSource::BearerHeader).await;
        request.extensions_mut().insert(identity);
        return next.run(request).await;
    }

    // Fall back to query parameter for SSE EventSource (constant-time comparison)
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(token) = pair.strip_prefix("token=")
                && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
            {
                let identity =
                    fallback_request_identity(&auth, GatewayAuthSource::BearerQuery).await;
                request.extensions_mut().insert(identity);
                return next.run(request).await;
            }
        }
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
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
    GatewayRequestIdentity::new(principal_id, actor_id, auth_source, true)
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
        let trusted = vec!["10.0.0.1".parse().unwrap(), "10.0.0.2".parse().unwrap()];
        assert!(is_trusted_ip(&"10.0.0.1".parse().unwrap(), &trusted));
        assert!(is_trusted_ip(&"10.0.0.2".parse().unwrap(), &trusted));
        assert!(!is_trusted_ip(&"10.0.0.3".parse().unwrap(), &trusted));
        // Loopback not in explicit list
        assert!(!is_trusted_ip(&"127.0.0.1".parse().unwrap(), &trusted));
    }

    #[test]
    fn test_auth_state_with_proxy() {
        let state = AuthState {
            token: "my-token".to_string(),
            trusted_proxy_header: Some("X-Forwarded-User".to_string()),
            trusted_proxy_ips: vec!["10.0.0.1".parse().unwrap()],
            fallback_principal_id: "user-1".to_string(),
            fallback_actor_id: "actor-1".to_string(),
            store: None,
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
        };

        let identity = fallback_request_identity(&state, GatewayAuthSource::BearerHeader).await;
        assert_eq!(identity.principal_id, "user-1");
        assert_eq!(identity.actor_id, "actor-1");
        assert!(identity.compatibility_fallback);
        assert_eq!(identity.auth_source.as_str(), "bearer_header");
    }
}
