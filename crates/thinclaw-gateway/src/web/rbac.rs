//! Request-level RBAC: mapping an HTTP request to the capability it needs, and
//! deciding whether a [`GatewayRole`] grants that capability.
//!
//! The role data type and principal configuration live in
//! `thinclaw_settings::gateway_rbac`; this module owns the HTTP-facing policy:
//! which route requires which capability, and the role→capability grant table.
//!
//! Enforcement is applied in [`crate::web::auth::auth_middleware`] after the
//! caller's identity+role is resolved. Because the primary gateway token (and
//! trusted-proxy identities) resolve to [`GatewayRole::Admin`], and Admin grants
//! every capability, a deployment with no extra principals is never restricted.

use axum::http::Method;

pub use thinclaw_settings::GatewayRole;

/// A coarse capability a request requires. Kept intentionally small: the tiers
/// map cleanly onto the three roles and avoid a brittle per-route matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayCapability {
    /// Read-only observation of state (safe HTTP methods on non-admin paths).
    ReadState,
    /// Drive the agent: chat, sessions, jobs, memory, skills — any state change
    /// that is not a configuration/security change.
    Chat,
    /// Change gateway configuration, security, or code-execution surfaces
    /// (settings, provider vault, extensions, MCP servers, hooks).
    ManageConfig,
}

/// Path prefixes whose routes administer configuration, secrets, or
/// code-execution surfaces. Any request to these — read or write — requires
/// [`GatewayCapability::ManageConfig`] (Admin only), because even reads can
/// expose configuration and the write side installs/executes code.
const ADMIN_SURFACE_PREFIXES: &[&str] = &[
    "/api/settings",
    "/api/providers",
    "/api/tool-policies",
    "/api/security",
    "/api/extensions",
    "/api/mcp",
    "/api/hooks",
    "/api/principals",
];

fn is_admin_surface(path: &str) -> bool {
    ADMIN_SURFACE_PREFIXES
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
}

fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// The capability a request requires, from its method and path.
///
/// - Any request to an admin surface → [`GatewayCapability::ManageConfig`].
/// - Otherwise a safe (read) method → [`GatewayCapability::ReadState`].
/// - Otherwise (a state-changing method) → [`GatewayCapability::Chat`].
pub fn capability_for_request(method: &Method, path: &str) -> GatewayCapability {
    if is_admin_surface(path) {
        GatewayCapability::ManageConfig
    } else if is_safe_method(method) {
        GatewayCapability::ReadState
    } else {
        GatewayCapability::Chat
    }
}

/// Whether `role` is permitted `capability`.
pub fn role_grants(role: GatewayRole, capability: GatewayCapability) -> bool {
    use GatewayCapability::*;
    match role {
        GatewayRole::Admin => true,
        GatewayRole::Operator => matches!(capability, ReadState | Chat),
        GatewayRole::ReadOnly => matches!(capability, ReadState),
    }
}

/// Whether `role` may perform the given request. Convenience over
/// [`capability_for_request`] + [`role_grants`].
pub fn role_allows_request(role: GatewayRole, method: &Method, path: &str) -> bool {
    role_grants(role, capability_for_request(method, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_surfaces_require_manage_config() {
        for path in [
            "/api/settings",
            "/api/settings/save",
            "/api/providers/openai/key",
            "/api/extensions/install",
            "/api/mcp/servers",
            "/api/hooks",
            "/api/tool-policies",
            "/api/security/policy",
        ] {
            assert_eq!(
                capability_for_request(&Method::GET, path),
                GatewayCapability::ManageConfig,
                "GET {path} should require ManageConfig"
            );
            assert_eq!(
                capability_for_request(&Method::POST, path),
                GatewayCapability::ManageConfig,
                "POST {path} should require ManageConfig"
            );
        }
    }

    #[test]
    fn prefix_match_is_boundary_safe() {
        // A path that merely shares a textual prefix but is a different segment
        // must NOT be treated as an admin surface.
        assert_eq!(
            capability_for_request(&Method::GET, "/api/settings-preview"),
            GatewayCapability::ReadState
        );
        assert_eq!(
            capability_for_request(&Method::POST, "/api/mcp-lite/run"),
            GatewayCapability::Chat
        );
    }

    #[test]
    fn non_admin_reads_and_writes() {
        assert_eq!(
            capability_for_request(&Method::GET, "/api/chat/history"),
            GatewayCapability::ReadState
        );
        assert_eq!(
            capability_for_request(&Method::POST, "/api/chat/send"),
            GatewayCapability::Chat
        );
        assert_eq!(
            capability_for_request(&Method::DELETE, "/api/chat/thread/abc"),
            GatewayCapability::Chat
        );
        assert_eq!(
            capability_for_request(&Method::HEAD, "/api/status"),
            GatewayCapability::ReadState
        );
    }

    #[test]
    fn role_grant_table() {
        use GatewayCapability::*;
        // Admin: everything.
        for cap in [ReadState, Chat, ManageConfig] {
            assert!(role_grants(GatewayRole::Admin, cap));
        }
        // Operator: read + chat, not config.
        assert!(role_grants(GatewayRole::Operator, ReadState));
        assert!(role_grants(GatewayRole::Operator, Chat));
        assert!(!role_grants(GatewayRole::Operator, ManageConfig));
        // ReadOnly: read only.
        assert!(role_grants(GatewayRole::ReadOnly, ReadState));
        assert!(!role_grants(GatewayRole::ReadOnly, Chat));
        assert!(!role_grants(GatewayRole::ReadOnly, ManageConfig));
    }

    #[test]
    fn role_allows_request_end_to_end() {
        // ReadOnly can read chat history but not send.
        assert!(role_allows_request(
            GatewayRole::ReadOnly,
            &Method::GET,
            "/api/chat/history"
        ));
        assert!(!role_allows_request(
            GatewayRole::ReadOnly,
            &Method::POST,
            "/api/chat/send"
        ));
        // Operator can send chat but not touch settings.
        assert!(role_allows_request(
            GatewayRole::Operator,
            &Method::POST,
            "/api/chat/send"
        ));
        assert!(!role_allows_request(
            GatewayRole::Operator,
            &Method::GET,
            "/api/settings"
        ));
        // Admin can do anything.
        assert!(role_allows_request(
            GatewayRole::Admin,
            &Method::POST,
            "/api/settings/save"
        ));
    }
}
