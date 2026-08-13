//! Bearer token authentication middleware for the web gateway.
//!
//! Supports authentication modes that preserve explicit credentials first,
//! then opt-in identity-aware ingress:
//! 1. **Trusted proxy** (optional): When `TRUSTED_PROXY_HEADER` env var is set, the gateway
//!    trusts that header (e.g., `X-Forwarded-User`) as the authenticated identity. This mode
//!    requires `TRUSTED_PROXY_IPS` to restrict which source IPs can use it (CIDR notation,
//!    comma-separated). If TRUSTED_PROXY_IPS is empty/unset, only loopback IPs are trusted.
//! 2. **Shared bearer token** (default): `Authorization: Bearer <token>` header, byte-identical
//!    constant-time compare against the operator's shared token; falls back to `?token=<token>`
//!    query param for SSE `EventSource` clients that cannot set headers.
//! 3. **Device token** (milestone B1, optional): `Authorization: Bearer tcd_...` header only —
//!    looked up in the [`crate::web::devices::DeviceRegistry`] only when the presented token
//!    carries the `tcd_` prefix, so the shared-token fast path above is never slowed down.
//!    Device tokens are header-only: a `tcd_` token via `?token=` is rejected outright. See
//!    `docs/MOBILE_SECURITY.md` (D-T*, §8 gateway hardening) and `docs/MOBILE_APP.md`.
//! 4. **Tailscale identity** (optional): an explicitly mapped tailnet peer is
//!    resolved through the authenticated local `tailscale whois` boundary, or
//!    through Tailscale Serve's loopback-only, spoof-stripping identity header
//!    contract. Funnel/public traffic never enables the Serve path.

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};

use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::sync::Arc;
use subtle::ConstantTimeEq;
use thinclaw_settings::{GatewayPrincipalConfig, GatewayRole};

use crate::web::devices::{DEVICE_TOKEN_PREFIX, DeviceRegistry, required_scope, store::now_iso};
use crate::web::identity::{
    DeviceContext, GatewayAuthSource, GatewayRequestIdentity, valid_gateway_identity_component,
};
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

/// Generic 403 body for device-principal scope denials. Deliberately
/// identical whether the route exists with a different required scope or
/// doesn't exist at all — `docs/MOBILE_SECURITY.md` §8 gateway hardening
/// item 4 forbids route-existence leakage to a device principal.
const DEVICE_SCOPE_FORBIDDEN_BODY: &str = "Forbidden";

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
    /// Optional device-token registry (milestone B1). `None` disables the
    /// device-token auth path entirely — trusted-proxy and shared-token
    /// behavior is unaffected either way.
    pub devices: Option<Arc<DeviceRegistry>>,
    /// Extra RBAC principals layered on top of the primary token. Each has its
    /// own token and role; the primary `token` above always authenticates as
    /// [`thinclaw_settings::GatewayRole::Admin`]. Empty by default, so RBAC is
    /// inactive unless the operator configures principals.
    pub principals: Vec<GatewayPrincipalConfig>,
    /// Optional Tailscale identity adapter. It is absent unless the operator
    /// supplies an explicit identity-to-principal mapping.
    pub tailscale: Option<TailscaleAuthState>,
}

/// One explicit Tailscale identity selector mapped to a bound ThinClaw
/// principal. Exactly one selector must be set. Stable numeric user/node IDs
/// are preferred; login and tag selectors are supported for operator
/// convenience.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TailscalePrincipalConfig {
    #[serde(default)]
    pub user_login: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    pub principal_id: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub role: GatewayRole,
}

impl TailscalePrincipalConfig {
    fn effective_actor_id(&self) -> &str {
        self.actor_id.as_deref().unwrap_or(&self.principal_id)
    }

    fn same_target(&self, other: &Self) -> bool {
        self.principal_id == other.principal_id
            && self.effective_actor_id() == other.effective_actor_id()
            && self.role == other.role
    }
}

/// Identity returned by the authenticated local Tailscale daemon boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailscalePeerIdentity {
    pub node_id: String,
    pub node_name: Option<String>,
    pub user_id: Option<String>,
    pub user_login: Option<String>,
    pub tags: Vec<String>,
}

/// Adapter boundary kept in the gateway crate so authentication can be tested
/// without launching a real Tailscale process.
#[async_trait::async_trait]
pub trait TailscaleIdentityResolver: Send + Sync {
    async fn resolve(&self, source: SocketAddr) -> Result<Option<TailscalePeerIdentity>, String>;
}

/// Shared, immutable state for Tailscale passwordless authentication.
#[derive(Clone)]
pub struct TailscaleAuthState {
    resolver: Arc<dyn TailscaleIdentityResolver>,
    principals: Arc<[TailscalePrincipalConfig]>,
    allow_serve_proxy: bool,
}

impl TailscaleAuthState {
    pub fn new(
        resolver: Arc<dyn TailscaleIdentityResolver>,
        principals: Vec<TailscalePrincipalConfig>,
        allow_serve_proxy: bool,
    ) -> Self {
        Self {
            resolver,
            principals: principals.into(),
            allow_serve_proxy,
        }
    }

    pub fn allow_serve_proxy(&self) -> bool {
        self.allow_serve_proxy
    }
}

/// Parse the opt-in `GATEWAY_TAILSCALE_PRINCIPALS` JSON mapping. Invalid,
/// ambiguous, duplicate, or oversized configurations fail closed rather than
/// silently weakening passwordless identity binding.
pub fn parse_tailscale_principals(raw: &str) -> Result<Vec<TailscalePrincipalConfig>, String> {
    const MAX_CONFIG_BYTES: usize = 1024 * 1024;
    const MAX_PRINCIPALS: usize = 1024;
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    if raw.len() > MAX_CONFIG_BYTES {
        return Err("Tailscale principal mapping exceeds the 1 MiB limit".to_string());
    }
    let mut mappings: Vec<TailscalePrincipalConfig> = serde_json::from_str(raw)
        .map_err(|error| format!("invalid Tailscale principal mapping JSON: {error}"))?;
    if mappings.len() > MAX_PRINCIPALS {
        return Err("Tailscale principal mapping contains more than 1024 entries".to_string());
    }

    let mut selectors = HashSet::new();
    for mapping in &mut mappings {
        mapping.user_login = normalize_selector(mapping.user_login.take(), true);
        mapping.user_id = normalize_selector(mapping.user_id.take(), false);
        mapping.node_id = normalize_selector(mapping.node_id.take(), false);
        mapping.tag = normalize_selector(mapping.tag.take(), false);
        mapping.principal_id = mapping.principal_id.trim().to_string();
        mapping.actor_id = normalize_selector(mapping.actor_id.take(), false);

        if !valid_gateway_identity_component(&mapping.principal_id)
            || mapping
                .actor_id
                .as_deref()
                .is_some_and(|actor| !valid_gateway_identity_component(actor))
        {
            return Err(
                "Tailscale mappings require valid principal_id/actor_id values".to_string(),
            );
        }

        let configured = [
            ("user_login", mapping.user_login.as_deref()),
            ("user_id", mapping.user_id.as_deref()),
            ("node_id", mapping.node_id.as_deref()),
            ("tag", mapping.tag.as_deref()),
        ]
        .into_iter()
        .filter_map(|(kind, value)| value.map(|value| (kind, value)))
        .collect::<Vec<_>>();
        if configured.len() != 1 {
            return Err(
                "each Tailscale mapping must configure exactly one of user_login, user_id, node_id, or tag"
                    .to_string(),
            );
        }
        let (kind, value) = configured[0];
        if !valid_gateway_identity_component(value) {
            return Err(format!("Tailscale {kind} selector is invalid"));
        }
        if kind == "tag" && !value.starts_with("tag:") {
            return Err("Tailscale tag selectors must start with `tag:`".to_string());
        }
        if !selectors.insert((kind.to_string(), value.to_string())) {
            return Err(format!("duplicate Tailscale {kind} selector `{value}`"));
        }
    }
    Ok(mappings)
}

fn normalize_selector(value: Option<String>, lowercase: bool) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim();
        if value.is_empty() {
            None
        } else if lowercase {
            Some(value.to_ascii_lowercase())
        } else {
            Some(value.to_string())
        }
    })
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

/// Tailscale allocates IPv4 peers from 100.64.0.0/10 and IPv6 peers from
/// fd7a:115c:a1e0::/48. This range check is only a cheap precondition: the
/// authenticated local daemon still has to resolve the exact source address.
pub fn is_tailscale_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            octets[0] == 100 && (octets[1] & 0b1100_0000) == 64
        }
        IpAddr::V6(ip) => ip.octets()[..6] == [0xfd, 0x7a, 0x11, 0x5c, 0xa1, 0xe0],
    }
}

fn mapping_matches_peer(mapping: &TailscalePrincipalConfig, peer: &TailscalePeerIdentity) -> bool {
    (mapping.user_login.as_deref().is_some_and(|expected| {
        peer.user_login
            .as_deref()
            .is_some_and(|actual| expected.eq_ignore_ascii_case(actual))
    }) && peer.user_id.is_some())
        || mapping
            .user_id
            .as_deref()
            .is_some_and(|expected| peer.user_id.as_deref() == Some(expected))
        || mapping
            .node_id
            .as_deref()
            .is_some_and(|expected| peer.node_id == expected)
        || mapping
            .tag
            .as_deref()
            .is_some_and(|expected| peer.tags.iter().any(|tag| tag == expected))
}

/// Resolve all selectors that match a peer. Multiple selectors may point to
/// the same target (for example a stable user id plus a node-specific rule),
/// but conflicting targets fail closed instead of depending on JSON order.
fn mapped_tailscale_principal(
    tailscale: &TailscaleAuthState,
    peer: &TailscalePeerIdentity,
) -> Result<Option<TailscalePrincipalConfig>, ()> {
    let mut matches = tailscale
        .principals
        .iter()
        .filter(|mapping| mapping_matches_peer(mapping, peer));
    let Some(first) = matches.next() else {
        return Ok(None);
    };
    if matches.any(|mapping| !mapping.same_target(first)) {
        return Err(());
    }
    Ok(Some(first.clone()))
}

async fn tailscale_request_identity(
    tailscale: &TailscaleAuthState,
    headers: &HeaderMap,
    source: SocketAddr,
) -> Option<(GatewayRequestIdentity, TailscalePeerIdentity)> {
    // Tailscale Serve strips caller-supplied identity headers and adds its own
    // only for tailnet traffic. The backend contract is accepted solely from
    // loopback and only when the runtime confirmed managed Serve (never
    // Funnel). Explicit mappings remain mandatory.
    if tailscale.allow_serve_proxy && source.ip().is_loopback() {
        let Some(login) = headers
            .get("tailscale-user-login")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| valid_gateway_identity_component(value))
        else {
            return None;
        };
        let peer = TailscalePeerIdentity {
            node_id: String::new(),
            node_name: None,
            user_id: None,
            user_login: Some(login.to_ascii_lowercase()),
            tags: Vec::new(),
        };
        // Serve's documented identity header authenticates a user login but
        // does not expose the stable user id. Match a login selector directly
        // under this narrower contract; tag/node/user-id selectors cannot
        // match Serve traffic.
        let mut matches = tailscale.principals.iter().filter(|mapping| {
            mapping
                .user_login
                .as_deref()
                .is_some_and(|expected| expected.eq_ignore_ascii_case(login))
        });
        let first = matches.next()?;
        if matches.any(|mapping| !mapping.same_target(first)) {
            tracing::error!(
                tailscale_user_login = %login,
                "Conflicting Tailscale Serve identity mappings matched one user; passwordless authentication denied"
            );
            return None;
        }
        let identity = GatewayRequestIdentity::new(
            first.principal_id.clone(),
            first.effective_actor_id().to_string(),
            GatewayAuthSource::TailscaleServe,
            false,
        )
        .with_role(first.role);
        return Some((identity, peer));
    }

    // Public/LAN/loopback sources can never enter the daemon-whois path. A
    // CGNAT-looking address alone is not trusted: the local daemon must return
    // a current peer record for the exact socket address.
    if !is_tailscale_ip(&source.ip()) {
        return None;
    }
    let peer = match tailscale.resolver.resolve(source).await {
        Ok(Some(peer)) if valid_gateway_identity_component(&peer.node_id) => peer,
        Ok(Some(_)) | Ok(None) => return None,
        Err(error) => {
            tracing::warn!(
                source_ip = %source.ip(),
                error = %error,
                "Tailscale whois identity resolution failed; bearer authentication remains available"
            );
            return None;
        }
    };
    identity_for_tailscale_peer(tailscale, peer, GatewayAuthSource::TailscaleWhois)
}

fn identity_for_tailscale_peer(
    tailscale: &TailscaleAuthState,
    peer: TailscalePeerIdentity,
    source: GatewayAuthSource,
) -> Option<(GatewayRequestIdentity, TailscalePeerIdentity)> {
    let mapping = match mapped_tailscale_principal(tailscale, &peer) {
        Ok(Some(mapping)) => mapping,
        Ok(None) => return None,
        Err(()) => {
            tracing::error!(
                tailscale_node_id = %peer.node_id,
                "Conflicting Tailscale identity mappings matched one peer; passwordless authentication denied"
            );
            return None;
        }
    };
    let actor_id = mapping.effective_actor_id().to_string();
    let role = mapping.role;
    let identity =
        GatewayRequestIdentity::new(mapping.principal_id, actor_id, source, false).with_role(role);
    Some((identity, peer))
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
            if !valid_gateway_identity_component(principal_id) {
                return (
                    StatusCode::UNAUTHORIZED,
                    "Trusted proxy identity was malformed",
                )
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
            let identity = fallback_request_identity(&auth, source.auth_source())
                .await
                .with_legacy_primary_binding();
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
        if token.starts_with(DEVICE_TOKEN_PREFIX) {
            if *source == TokenSource::Query {
                return (
                    StatusCode::UNAUTHORIZED,
                    "Device tokens are not accepted via query parameters",
                )
                    .into_response();
            }
            if let Some(devices) = auth.devices.as_ref() {
                return authenticate_device_request(&auth, devices, token, request, next).await;
            }
        }
    }

    // Tailscale is a passwordless fallback, never an override for an explicit
    // valid bearer/device credential. Every accepted identity is bound to an
    // operator-supplied mapping and receives the same RBAC gate as tokens.
    if let Some(tailscale) = auth.tailscale.as_ref()
        && let Some(source) = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0)
        && let Some((identity, peer)) =
            tailscale_request_identity(tailscale, &headers, source).await
    {
        if let Some(denied) = enforce_capability(&request, &identity) {
            return denied;
        }
        tracing::info!(
            audit_event = "gateway.auth.accepted",
            auth_mechanism = identity.auth_source.as_str(),
            principal_id = %identity.principal_id,
            actor_id = %identity.actor_id,
            role = identity.role.as_str(),
            source_ip = %source.ip(),
            tailscale_node_id = %peer.node_id,
            tailscale_user_id = peer.user_id.as_deref().unwrap_or(""),
            tailscale_user_login = peer.user_login.as_deref().unwrap_or(""),
            "Tailscale identity authentication accepted"
        );
        request.extensions_mut().insert(identity);
        return next.run(request).await;
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

/// Authenticate a `tcd_`-prefixed bearer token against the device registry,
/// enforce the route's required scope, and (on success) run the request
/// with a `GatewayRequestIdentity` + `DeviceContext` attached.
///
/// Devices act as the operator for thread-visibility purposes: the resolved
/// identity uses the same fallback principal/actor as shared-token requests,
/// so thread visibility stays identical across surfaces. Device attribution
/// (which physical device made the call) is carried separately via
/// `DeviceContext`, not by diverging the principal/actor id.
async fn authenticate_device_request(
    auth: &AuthState,
    devices: &Arc<DeviceRegistry>,
    token: &str,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(device_auth) = devices.authenticate(token).await else {
        return (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response();
    };

    let method = request.method().as_str().to_string();
    let path = request.uri().path().to_string();
    match required_scope(&method, &path) {
        Some(scope) if device_auth.scopes.contains(&scope) => {}
        _ => {
            // Identical body/status whether the scope is simply missing or
            // the route isn't part of the device-token surface at all — no
            // route-existence leakage to a device principal (§8 item 4).
            return (StatusCode::FORBIDDEN, DEVICE_SCOPE_FORBIDDEN_BODY).into_response();
        }
    }

    devices.touch(&device_auth.device_id, &now_iso()).await.ok();

    let identity = fallback_request_identity(auth, GatewayAuthSource::DeviceToken).await;
    request.extensions_mut().insert(identity);
    request.extensions_mut().insert(DeviceContext::with_class(
        device_auth.device_id,
        device_auth.scopes,
        device_auth.platform,
        device_auth.is_companion,
    ));
    next.run(request).await
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
    let principal_id = if valid_gateway_identity_component(&auth.fallback_principal_id)
        && auth.fallback_principal_id != "default"
    {
        auth.fallback_principal_id.clone()
    } else if let Some(store) = auth.store.as_ref() {
        match store.infer_primary_user_id_for_channel("gateway").await {
            Ok(Some(inferred)) if valid_gateway_identity_component(&inferred) => inferred,
            Ok(_) | Err(_) => auth.fallback_principal_id.clone(),
        }
    } else {
        auth.fallback_principal_id.clone()
    };
    let principal_id = if valid_gateway_identity_component(&principal_id) {
        principal_id
    } else {
        "default".to_string()
    };
    let actor_id = default_gateway_actor_id_from_auth(auth, &principal_id);
    // The primary gateway token is the admin credential.
    GatewayRequestIdentity::new(principal_id, actor_id, auth_source, true)
        .with_role(GatewayRole::Admin)
}

fn default_gateway_actor_id_from_auth(auth: &AuthState, principal_id: &str) -> String {
    if !valid_gateway_identity_component(&auth.fallback_actor_id)
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

    fn base_auth_state() -> AuthState {
        AuthState {
            token: "test-token".to_string(),
            trusted_proxy_header: None,
            trusted_proxy_ips: vec![],
            fallback_principal_id: "test-user".to_string(),
            fallback_actor_id: "test-actor".to_string(),
            store: None,
            devices: None,
            principals: vec![],
            tailscale: None,
        }
    }

    #[test]
    fn test_auth_state_clone() {
        let state = base_auth_state();
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
    fn tailscale_address_ranges_are_exact() {
        assert!(is_tailscale_ip(&"100.64.0.1".parse().unwrap()));
        assert!(is_tailscale_ip(&"100.127.255.254".parse().unwrap()));
        assert!(!is_tailscale_ip(&"100.63.255.255".parse().unwrap()));
        assert!(!is_tailscale_ip(&"100.128.0.1".parse().unwrap()));
        assert!(is_tailscale_ip(&"fd7a:115c:a1e0::1".parse().unwrap()));
        assert!(!is_tailscale_ip(&"fd7a:115c:a1e1::1".parse().unwrap()));
        assert!(!is_tailscale_ip(&"127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn tailscale_mapping_parser_normalizes_and_validates_selectors() {
        let mappings = parse_tailscale_principals(
            r#"[
                {"user_login":" Alice@Example.COM ","principal_id":" alice ","role":"operator"},
                {"node_id":"1234","principal_id":"builder","actor_id":"node-a"},
                {"tag":"tag:ci","principal_id":"ci"}
            ]"#,
        )
        .unwrap();
        assert_eq!(mappings.len(), 3);
        assert_eq!(mappings[0].user_login.as_deref(), Some("alice@example.com"));
        assert_eq!(mappings[0].principal_id, "alice");
        assert_eq!(mappings[0].role, GatewayRole::Operator);
        assert_eq!(mappings[1].effective_actor_id(), "node-a");
        assert_eq!(mappings[2].role, GatewayRole::ReadOnly);
    }

    #[test]
    fn tailscale_mapping_parser_fails_closed_on_ambiguity() {
        assert!(
            parse_tailscale_principals(
                r#"[{"user_login":"a@example.com","node_id":"1","principal_id":"a"}]"#
            )
            .unwrap_err()
            .contains("exactly one")
        );
        assert!(
            parse_tailscale_principals(
                r#"[
                    {"node_id":"1","principal_id":"a"},
                    {"node_id":"1","principal_id":"b"}
                ]"#
            )
            .unwrap_err()
            .contains("duplicate")
        );
        assert!(
            parse_tailscale_principals(r#"[{"tag":"ci","principal_id":"a"}]"#)
                .unwrap_err()
                .contains("tag:")
        );
        assert!(
            parse_tailscale_principals(r#"[{"node_id":"1","principal_id":"a","unexpected":true}]"#)
                .is_err()
        );
    }

    #[test]
    fn test_auth_state_with_proxy() {
        let state = AuthState {
            trusted_proxy_header: Some("X-Forwarded-User".to_string()),
            trusted_proxy_ips: vec![parse_trusted_proxy_entry("10.0.0.1").unwrap()],
            fallback_principal_id: "user-1".to_string(),
            fallback_actor_id: "actor-1".to_string(),
            ..base_auth_state()
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
            fallback_principal_id: "user-1".to_string(),
            fallback_actor_id: "actor-1".to_string(),
            ..base_auth_state()
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
                devices: None,
                principals,
                tailscale: None,
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

        struct ToggleTailscaleResolver {
            active: std::sync::atomic::AtomicBool,
            peer: TailscalePeerIdentity,
        }

        #[async_trait::async_trait]
        impl TailscaleIdentityResolver for ToggleTailscaleResolver {
            async fn resolve(
                &self,
                _source: SocketAddr,
            ) -> Result<Option<TailscalePeerIdentity>, String> {
                Ok(self
                    .active
                    .load(std::sync::atomic::Ordering::SeqCst)
                    .then(|| self.peer.clone()))
            }
        }

        fn test_peer() -> TailscalePeerIdentity {
            TailscalePeerIdentity {
                node_id: "node-123".to_string(),
                node_name: Some("alice-phone".to_string()),
                user_id: Some("user-456".to_string()),
                user_login: Some("alice@example.com".to_string()),
                tags: vec!["tag:staff".to_string()],
            }
        }

        fn tailscale_mapping(
            selector: (&str, &str),
            principal_id: &str,
            role: GatewayRole,
        ) -> TailscalePrincipalConfig {
            let mut mapping = TailscalePrincipalConfig {
                user_login: None,
                user_id: None,
                node_id: None,
                tag: None,
                principal_id: principal_id.to_string(),
                actor_id: None,
                role,
            };
            match selector.0 {
                "user_login" => mapping.user_login = Some(selector.1.to_string()),
                "user_id" => mapping.user_id = Some(selector.1.to_string()),
                "node_id" => mapping.node_id = Some(selector.1.to_string()),
                "tag" => mapping.tag = Some(selector.1.to_string()),
                _ => panic!("unknown test selector"),
            }
            mapping
        }

        async fn echo_tailscale_identity(identity: GatewayRequestIdentity) -> String {
            format!(
                "{}:{}:{}:{}",
                identity.principal_id,
                identity.actor_id,
                identity.auth_source.as_str(),
                identity.role.as_str()
            )
        }

        async fn tailscale_request(
            auth: AuthState,
            method: Method,
            path: &str,
            source: &str,
            tailscale_login_header: Option<&str>,
            bearer: Option<&str>,
        ) -> Response {
            use axum::extract::ConnectInfo;

            let app = Router::new()
                .route("/api/chat/history", get(echo_tailscale_identity))
                .route("/api/chat/send", post(echo_tailscale_identity))
                .route("/api/settings/save", post(echo_tailscale_identity))
                .route_layer(from_fn_with_state(auth, auth_middleware));
            let mut builder = Request::builder().method(method).uri(path);
            if let Some(login) = tailscale_login_header {
                builder = builder.header("tailscale-user-login", login);
            }
            if let Some(token) = bearer {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            let mut request = builder.body(Body::empty()).unwrap();
            request.extensions_mut().insert(ConnectInfo(
                source.parse::<SocketAddr>().expect("valid test source"),
            ));
            app.oneshot(request).await.unwrap()
        }

        async fn response_body(response: Response) -> String {
            String::from_utf8(
                axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap()
        }

        #[tokio::test]
        async fn direct_whois_maps_bound_identity_and_enforces_role() {
            let resolver = Arc::new(ToggleTailscaleResolver {
                active: std::sync::atomic::AtomicBool::new(true),
                peer: test_peer(),
            });
            let mut auth = base_auth(vec![]);
            auth.tailscale = Some(TailscaleAuthState::new(
                resolver,
                vec![tailscale_mapping(
                    ("node_id", "node-123"),
                    "alice",
                    GatewayRole::Operator,
                )],
                false,
            ));

            let response = tailscale_request(
                auth.clone(),
                Method::POST,
                "/api/chat/send",
                "100.64.10.20:50100",
                None,
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response_body(response).await,
                "alice:alice:tailscale_whois:operator"
            );

            // The mapped Operator remains bound by normal RBAC.
            let response = tailscale_request(
                auth,
                Method::POST,
                "/api/settings/save",
                "100.64.10.20:50101",
                None,
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }

        #[tokio::test]
        async fn serve_headers_are_loopback_only_and_never_enabled_for_funnel() {
            let inactive = Arc::new(ToggleTailscaleResolver {
                active: std::sync::atomic::AtomicBool::new(false),
                peer: test_peer(),
            });
            let mapping = tailscale_mapping(
                ("user_login", "alice@example.com"),
                "alice",
                GatewayRole::ReadOnly,
            );
            let mut serve_auth = base_auth(vec![]);
            serve_auth.tailscale = Some(TailscaleAuthState::new(
                inactive.clone(),
                vec![mapping.clone()],
                true,
            ));

            let accepted = tailscale_request(
                serve_auth.clone(),
                Method::GET,
                "/api/chat/history",
                "127.0.0.1:50100",
                Some("Alice@Example.COM"),
                None,
            )
            .await;
            assert_eq!(accepted.status(), StatusCode::OK);
            assert_eq!(
                response_body(accepted).await,
                "alice:alice:tailscale_serve:read_only"
            );

            // The same header sent directly from a LAN peer is spoofable and
            // therefore ignored.
            let spoofed = tailscale_request(
                serve_auth,
                Method::GET,
                "/api/chat/history",
                "192.168.1.50:50100",
                Some("alice@example.com"),
                None,
            )
            .await;
            assert_eq!(spoofed.status(), StatusCode::UNAUTHORIZED);

            // Funnel/public mode never enables the Serve contract, even when
            // a public caller supplies the exact identity header.
            let mut funnel_auth = base_auth(vec![]);
            funnel_auth.tailscale = Some(TailscaleAuthState::new(inactive, vec![mapping], false));
            let funnel = tailscale_request(
                funnel_auth,
                Method::GET,
                "/api/chat/history",
                "127.0.0.1:50101",
                Some("alice@example.com"),
                None,
            )
            .await;
            assert_eq!(funnel.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn whois_revocation_is_immediate_and_bearer_fallback_survives() {
            let resolver = Arc::new(ToggleTailscaleResolver {
                active: std::sync::atomic::AtomicBool::new(true),
                peer: test_peer(),
            });
            let mut auth = base_auth(vec![]);
            auth.tailscale = Some(TailscaleAuthState::new(
                resolver.clone(),
                vec![tailscale_mapping(
                    ("user_id", "user-456"),
                    "alice",
                    GatewayRole::ReadOnly,
                )],
                false,
            ));

            let before = tailscale_request(
                auth.clone(),
                Method::GET,
                "/api/chat/history",
                "100.64.10.20:50100",
                None,
                None,
            )
            .await;
            assert_eq!(before.status(), StatusCode::OK);

            resolver
                .active
                .store(false, std::sync::atomic::Ordering::SeqCst);
            let revoked = tailscale_request(
                auth.clone(),
                Method::GET,
                "/api/chat/history",
                "100.64.10.20:50101",
                None,
                None,
            )
            .await;
            assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);

            let bearer = tailscale_request(
                auth,
                Method::POST,
                "/api/settings/save",
                "100.64.10.20:50102",
                None,
                Some("admin-token"),
            )
            .await;
            assert_eq!(bearer.status(), StatusCode::OK);
            assert_eq!(response_body(bearer).await, "root:root:bearer_header:admin");
        }

        #[tokio::test]
        async fn conflicting_identity_selectors_fail_closed() {
            let resolver = Arc::new(ToggleTailscaleResolver {
                active: std::sync::atomic::AtomicBool::new(true),
                peer: test_peer(),
            });
            let mut auth = base_auth(vec![]);
            auth.tailscale = Some(TailscaleAuthState::new(
                resolver,
                vec![
                    tailscale_mapping(("node_id", "node-123"), "alice", GatewayRole::Admin),
                    tailscale_mapping(
                        ("user_login", "alice@example.com"),
                        "mallory",
                        GatewayRole::Admin,
                    ),
                ],
                false,
            ));
            let response = tailscale_request(
                auth,
                Method::GET,
                "/api/chat/history",
                "100.64.10.20:50100",
                None,
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    // --- Device-token auth-path integration tests ---
    //
    // These drive `auth_middleware` through a real `axum::Router` (rather
    // than calling it directly) so header/query parsing, extension
    // insertion, and status/body codes are all exercised the way a real
    // request would hit them.

    mod device_auth {
        use super::*;
        use crate::web::devices::registry::DeviceRegistry;
        use crate::web::devices::store::DeviceStore;
        use crate::web::devices::types::{DevicePlatform, DeviceScope};
        use crate::web::identity::DeviceContext;

        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::routing::get;
        use axum::{Extension, Router};
        use tempfile::TempDir;
        use tower::ServiceExt;

        async fn test_registry() -> (DeviceRegistry, DeviceStore, TempDir) {
            let dir = TempDir::new().unwrap();
            let store = DeviceStore::with_base_dir(dir.path().to_path_buf());
            let registry = DeviceRegistry::load(store.clone()).await.unwrap();
            (registry, store, dir)
        }

        fn router(auth: AuthState) -> Router {
            Router::new()
                .route("/api/chat/send", get(echo_identity).post(echo_identity))
                .route("/api/devices/me", get(echo_identity))
                .route("/api/totally/unknown/route", get(echo_identity))
                .route_layer(axum::middleware::from_fn_with_state(auth, auth_middleware))
        }

        async fn echo_identity(
            identity: GatewayRequestIdentity,
            device_ctx: Option<Extension<DeviceContext>>,
        ) -> String {
            format!(
                "{}:{}:{}:{}",
                identity.principal_id,
                identity.actor_id,
                identity.auth_source.as_str(),
                device_ctx
                    .map(|Extension(ctx)| ctx.device_id)
                    .unwrap_or_default()
            )
        }

        fn request(uri: &str, token: Option<&str>) -> Request<Body> {
            let mut builder = Request::builder().uri(uri);
            if let Some(token) = token {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            builder.body(Body::empty()).unwrap()
        }

        fn post_request(uri: &str, token: Option<&str>) -> Request<Body> {
            let mut builder = Request::builder().method(axum::http::Method::POST).uri(uri);
            if let Some(token) = token {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            builder.body(Body::empty()).unwrap()
        }

        #[tokio::test]
        async fn device_token_accepted_uses_shared_fallback_identity() {
            let (registry, store, _dir) = test_registry().await;
            let (record, token) = store
                .insert(
                    "Phone".to_string(),
                    DevicePlatform::Ios,
                    vec![DeviceScope::Chat],
                    None,
                )
                .unwrap();
            registry.refresh(&record.device_id).await.unwrap();

            let auth = AuthState {
                fallback_principal_id: "operator".to_string(),
                fallback_actor_id: "operator".to_string(),
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            let response = router(auth)
                .oneshot(post_request("/api/chat/send", Some(&token)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);

            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            // Same fallback principal/actor as shared-token requests, plus
            // the device_token auth source and the authenticating device id.
            assert_eq!(
                body,
                format!("operator:operator:device_token:{}", record.device_id)
            );
        }

        #[tokio::test]
        async fn revoked_device_token_is_rejected() {
            let (registry, store, _dir) = test_registry().await;
            let (record, token) = store
                .insert(
                    "Phone".to_string(),
                    DevicePlatform::Ios,
                    vec![DeviceScope::Chat],
                    None,
                )
                .unwrap();
            registry.refresh(&record.device_id).await.unwrap();
            registry.revoke(&record.device_id).await.unwrap();

            let auth = AuthState {
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            let response = router(auth)
                .oneshot(post_request("/api/chat/send", Some(&token)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn expired_device_token_is_rejected() {
            let (registry, store, dir) = test_registry().await;
            let (record, token) = store
                .insert(
                    "Phone".to_string(),
                    DevicePlatform::Ios,
                    vec![DeviceScope::Chat],
                    None,
                )
                .unwrap();

            let raw_path = dir.path().join("devices.json");
            let raw = std::fs::read_to_string(&raw_path).unwrap();
            let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
            value["devices"][0]["expires_at"] = serde_json::json!("2000-01-01T00:00:00+00:00");
            std::fs::write(&raw_path, serde_json::to_string_pretty(&value).unwrap()).unwrap();
            registry.refresh(&record.device_id).await.unwrap();

            let auth = AuthState {
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            let response = router(auth)
                .oneshot(post_request("/api/chat/send", Some(&token)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn device_token_via_query_param_is_rejected() {
            let (registry, store, _dir) = test_registry().await;
            let (record, token) = store
                .insert(
                    "Phone".to_string(),
                    DevicePlatform::Ios,
                    vec![DeviceScope::Chat],
                    None,
                )
                .unwrap();
            registry.refresh(&record.device_id).await.unwrap();

            let auth = AuthState {
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            let response = router(auth)
                .oneshot(request(&format!("/api/chat/send?token={token}"), None))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn missing_scope_returns_generic_403() {
            let (registry, store, _dir) = test_registry().await;
            // Chat-only scope; /api/devices/me requires DevicesSelf.
            let (record, token) = store
                .insert(
                    "Phone".to_string(),
                    DevicePlatform::Ios,
                    vec![DeviceScope::Chat],
                    None,
                )
                .unwrap();
            registry.refresh(&record.device_id).await.unwrap();

            let auth = AuthState {
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            let response = router(auth)
                .oneshot(request("/api/devices/me", Some(&token)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(body.as_ref(), DEVICE_SCOPE_FORBIDDEN_BODY.as_bytes());
        }

        #[tokio::test]
        async fn unknown_route_403_body_matches_missing_scope_403_body() {
            let (registry, store, _dir) = test_registry().await;
            let (record, token) = store
                .insert(
                    "Phone".to_string(),
                    DevicePlatform::Ios,
                    vec![DeviceScope::Chat],
                    None,
                )
                .unwrap();
            registry.refresh(&record.device_id).await.unwrap();

            let auth = AuthState {
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            let response = router(auth)
                .oneshot(request("/api/totally/unknown/route", Some(&token)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            // Identical status + body as the "known route, wrong scope" case
            // above — no route-existence leakage to a device principal.
            assert_eq!(body.as_ref(), DEVICE_SCOPE_FORBIDDEN_BODY.as_bytes());
        }

        #[tokio::test]
        async fn shared_token_behavior_unchanged_when_devices_configured() {
            let (registry, _store, _dir) = test_registry().await;
            let auth = AuthState {
                token: "shared-secret".to_string(),
                fallback_principal_id: "operator".to_string(),
                fallback_actor_id: "operator".to_string(),
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            let response = router(auth)
                .oneshot(request("/api/devices/me", Some("shared-secret")))
                .await
                .unwrap();
            // Shared-token requests are unaffected by scope enforcement —
            // even a route with a device scope requirement succeeds.
            assert_eq!(response.status(), StatusCode::OK);
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert_eq!(body, "operator:operator:bearer_header:");
        }

        #[tokio::test]
        async fn compat_overrides_are_ignored_for_device_principals() {
            // A device-authenticated request must not be able to override
            let (registry, store, _dir) = test_registry().await;
            let (record, token) = store
                .insert(
                    "Phone".to_string(),
                    DevicePlatform::Ios,
                    vec![DeviceScope::Chat],
                    None,
                )
                .unwrap();
            registry.refresh(&record.device_id).await.unwrap();

            let auth = AuthState {
                fallback_principal_id: "operator".to_string(),
                fallback_actor_id: "operator".to_string(),
                devices: Some(Arc::new(registry)),
                ..base_auth_state()
            };

            // Compatibility is bound to the primary credential, not to the
            // bearer transport. A device fallback identity stays bound.
            let identity = fallback_request_identity(&auth, GatewayAuthSource::DeviceToken).await;
            assert_eq!(identity.principal_id, "operator");
            assert_eq!(identity.actor_id, "operator");
            assert!(!identity.allows_compat_overrides());
            assert_eq!(
                identity
                    .with_compat_overrides(Some("attacker"), Some("attacker"))
                    .principal_id,
                "operator"
            );

            let response = router(auth)
                .oneshot(post_request("/api/chat/send", Some(&token)))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    }
}
