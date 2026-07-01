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
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::{IpAddr, SocketAddr};

use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::sync::Arc;
use subtle::ConstantTimeEq;

use crate::web::identity::{GatewayAuthSource, GatewayRequestIdentity};
use crate::web::ports::IdentityLookupPort;

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
                // RFC 6750 §2.3: tokens in URLs can leak via access logs, proxies,
                // and Referer headers. This path exists only for SSE EventSource
                // clients that cannot set an Authorization header; warn once so the
                // operator is aware of the log-exposure trade-off.
                static QUERY_AUTH_WARNED: std::sync::Once = std::sync::Once::new();
                QUERY_AUTH_WARNED.call_once(|| {
                    tracing::warn!(
                        "gateway accepted bearer auth via `?token=` query parameter \
                         (SSE EventSource fallback); per RFC 6750 §2.3 tokens in URLs \
                         can leak through logs/proxies/Referer — prefer the \
                         Authorization header where the client supports it"
                    );
                });
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
