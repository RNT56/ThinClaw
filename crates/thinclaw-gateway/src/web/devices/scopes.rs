//! Route -> required-scope mapping for device-authenticated requests.
//!
//! Per `docs/MOBILE_SECURITY.md` D-T4, only four scopes are ever grantable
//! to a device token: `chat`, `approvals`, `jobs:read`, `devices:self`.
//! Everything else — settings, secrets/providers, extensions/skills, memory
//! write, logs, restart, pairing admin — is **never** grantable, which this
//! map enforces by simply returning `None` (meaning: forbidden for device
//! principals, regardless of route existence) for every prefix it doesn't
//! recognize.
//!
//! Precedence is most-specific-prefix-first: `/api/chat/approval(s)` must be
//! checked *before* the general `/api/chat/` prefix, since both share the
//! `/api/chat` root but map to different scopes.

use super::types::DeviceScope;

/// Resolve the scope required to access `path` with the given HTTP method.
///
/// Returns `None` if the route is not part of the device-token surface at
/// all (settings, secrets, extensions, memory, logs, admin, pairing-admin,
/// etc.) — callers must treat `None` as FORBIDDEN for a device principal,
/// not as "no scope needed".
pub fn required_scope(method: &str, path: &str) -> Option<DeviceScope> {
    let method = method.to_uppercase();

    // Most specific first: approvals is a sub-surface of /api/chat but
    // requires a distinct scope so watch/widget tokens can stay
    // least-privilege (approvals without full chat send/read).
    if path.starts_with("/api/chat/approvals") || path.starts_with("/api/chat/approval") {
        return Some(DeviceScope::Approvals);
    }

    // Extension-auth endpoints live under /api/chat/ for historical reasons
    // but are an extensions surface (they submit OAuth credentials and
    // activate extensions) — never grantable to device tokens per D-T4.
    if path == "/api/chat/auth-token" || path == "/api/chat/auth-cancel" {
        return None;
    }

    if path.starts_with("/api/chat/") || path == "/api/chat" {
        return Some(DeviceScope::Chat);
    }

    if path == "/api/devices/me" || path.starts_with("/api/devices/me/") {
        return Some(DeviceScope::DevicesSelf);
    }

    // Jobs are read-only for device tokens in v1 — mutating job endpoints
    // (cancel, retry, etc.) are intentionally excluded.
    if (path.starts_with("/api/jobs/") || path == "/api/jobs") && method == "GET" {
        return Some(DeviceScope::JobsRead);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approvals_takes_precedence_over_chat_prefix() {
        assert_eq!(
            required_scope("POST", "/api/chat/approval"),
            Some(DeviceScope::Approvals)
        );
        assert_eq!(
            required_scope("GET", "/api/chat/approvals"),
            Some(DeviceScope::Approvals)
        );
        assert_eq!(
            required_scope("POST", "/api/chat/approvals/123"),
            Some(DeviceScope::Approvals)
        );
    }

    #[test]
    fn chat_prefix_covers_send_history_events_ws() {
        for path in [
            "/api/chat/send",
            "/api/chat/history",
            "/api/chat/threads",
            "/api/chat/events",
            "/api/chat/ws",
        ] {
            assert_eq!(
                required_scope("GET", path),
                Some(DeviceScope::Chat),
                "path: {path}"
            );
            assert_eq!(
                required_scope("POST", path),
                Some(DeviceScope::Chat),
                "path: {path}"
            );
        }
    }

    #[test]
    fn jobs_read_only_get_allowed() {
        assert_eq!(
            required_scope("GET", "/api/jobs"),
            Some(DeviceScope::JobsRead)
        );
        assert_eq!(
            required_scope("GET", "/api/jobs/abc123"),
            Some(DeviceScope::JobsRead)
        );
    }

    #[test]
    fn jobs_mutation_methods_are_forbidden() {
        for method in ["POST", "PUT", "PATCH", "DELETE"] {
            assert_eq!(required_scope(method, "/api/jobs/abc123"), None);
            assert_eq!(required_scope(method, "/api/jobs"), None);
        }
    }

    #[test]
    fn devices_self_scope() {
        assert_eq!(
            required_scope("GET", "/api/devices/me"),
            Some(DeviceScope::DevicesSelf)
        );
        assert_eq!(
            required_scope("PUT", "/api/devices/me/push"),
            Some(DeviceScope::DevicesSelf)
        );
    }

    #[test]
    fn devices_admin_routes_are_never_grantable() {
        // /api/devices (list) and /api/devices/{id}/rotate etc. are admin
        // surfaces, not the device's own `devices:self` scope.
        assert_eq!(required_scope("GET", "/api/devices"), None);
        assert_eq!(required_scope("POST", "/api/devices/pair/start"), None);
        assert_eq!(required_scope("POST", "/api/devices/abc/revoke"), None);
    }

    #[test]
    fn never_grantable_surfaces_return_none() {
        for path in [
            "/api/settings",
            "/api/secrets",
            "/api/providers",
            "/api/extensions",
            "/api/skills",
            "/api/memory",
            "/api/logs",
            "/api/restart",
        ] {
            assert_eq!(required_scope("GET", path), None, "path: {path}");
            assert_eq!(required_scope("POST", path), None, "path: {path}");
        }
    }

    #[test]
    fn unknown_route_is_forbidden_not_no_scope() {
        assert_eq!(required_scope("GET", "/api/totally/unknown/route"), None);
    }

    #[test]
    fn extension_auth_routes_under_chat_are_never_grantable() {
        // These live under /api/chat/ but submit extension OAuth credentials
        // and trigger extension activation — an extensions surface (D-T4).
        assert_eq!(required_scope("POST", "/api/chat/auth-token"), None);
        assert_eq!(required_scope("POST", "/api/chat/auth-cancel"), None);
    }

    #[test]
    fn method_is_case_insensitive() {
        assert_eq!(
            required_scope("get", "/api/jobs"),
            Some(DeviceScope::JobsRead)
        );
    }
}
