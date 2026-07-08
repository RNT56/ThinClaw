//! Gateway role-based access control (RBAC) configuration.
//!
//! The gateway historically authenticates with a single bearer token that
//! grants full control. RBAC layers *additional* principals on top of that
//! primary token: each extra principal has its own token and a [`GatewayRole`]
//! that bounds what it may do.
//!
//! This is **opt-in and additive** — the primary gateway token always maps to
//! [`GatewayRole::Admin`], so a deployment that configures no extra principals
//! behaves exactly as before. The capability model and per-request enforcement
//! live in `thinclaw_gateway::web::rbac`; this module only owns the serializable
//! configuration and the role data type.

use serde::{Deserialize, Serialize};

/// A gateway principal's privilege tier.
///
/// Ordering is by privilege (`ReadOnly < Operator < Admin`) so a role can be
/// compared against a required minimum.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRole {
    /// May only read state (safe `GET`-style requests). The safe default for a
    /// deserialized principal that omits its role.
    #[default]
    ReadOnly,
    /// May read and drive the agent (chat, sessions, jobs) but not change
    /// gateway configuration or security settings.
    Operator,
    /// Full control, equivalent to the primary gateway token.
    Admin,
}

impl GatewayRole {
    /// Stable lowercase identifier, for logs and API responses.
    pub fn as_str(self) -> &'static str {
        match self {
            GatewayRole::ReadOnly => "read_only",
            GatewayRole::Operator => "operator",
            GatewayRole::Admin => "admin",
        }
    }
}

/// One configured extra principal: a bearer token bound to an identity and a
/// role. Stored alongside the primary token in gateway settings and protected
/// by the same file permissions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayPrincipalConfig {
    /// Bearer token this principal presents. Compared in constant time by the
    /// gateway; an empty/whitespace-only token is ignored at load time so it can
    /// never authenticate an empty `Authorization: Bearer`.
    pub token: String,
    /// Stable principal id this token authenticates as.
    pub principal_id: String,
    /// Optional actor id; defaults to `principal_id` when omitted.
    #[serde(default)]
    pub actor_id: Option<String>,
    /// Privilege tier; defaults to the least-privileged [`GatewayRole::ReadOnly`].
    #[serde(default)]
    pub role: GatewayRole,
}

impl GatewayPrincipalConfig {
    /// Whether this entry is usable: a non-empty token and principal id. Blank
    /// tokens are dropped so they cannot authenticate.
    pub fn is_valid(&self) -> bool {
        !self.token.trim().is_empty() && !self.principal_id.trim().is_empty()
    }

    /// The effective actor id (`actor_id` if set and non-empty, else the
    /// principal id).
    pub fn effective_actor_id(&self) -> &str {
        match self.actor_id.as_deref() {
            Some(actor) if !actor.trim().is_empty() => actor,
            _ => &self.principal_id,
        }
    }
}

/// Parse a JSON array of principals from an env-var / config string, keeping
/// only valid entries. Returns an empty vec for blank input, and an error
/// string when the JSON is malformed so the operator learns their config was
/// ignored rather than silently dropping every principal.
pub fn parse_gateway_principals(raw: &str) -> Result<Vec<GatewayPrincipalConfig>, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Vec<GatewayPrincipalConfig> = serde_json::from_str(trimmed)
        .map_err(|error| format!("invalid gateway principals JSON: {error}"))?;
    Ok(parsed.into_iter().filter(|p| p.is_valid()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_default_is_least_privileged() {
        assert_eq!(GatewayRole::default(), GatewayRole::ReadOnly);
        assert!(GatewayRole::ReadOnly < GatewayRole::Operator);
        assert!(GatewayRole::Operator < GatewayRole::Admin);
    }

    #[test]
    fn role_serde_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&GatewayRole::ReadOnly).unwrap(),
            "\"read_only\""
        );
        assert_eq!(
            serde_json::from_str::<GatewayRole>("\"operator\"").unwrap(),
            GatewayRole::Operator
        );
    }

    #[test]
    fn principal_effective_actor_defaults_to_principal() {
        let p = GatewayPrincipalConfig {
            token: "t".into(),
            principal_id: "alice".into(),
            actor_id: None,
            role: GatewayRole::Operator,
        };
        assert_eq!(p.effective_actor_id(), "alice");

        let with_actor = GatewayPrincipalConfig {
            actor_id: Some("alice-laptop".into()),
            ..p.clone()
        };
        assert_eq!(with_actor.effective_actor_id(), "alice-laptop");

        // Blank actor falls back to principal.
        let blank_actor = GatewayPrincipalConfig {
            actor_id: Some("   ".into()),
            ..p
        };
        assert_eq!(blank_actor.effective_actor_id(), "alice");
    }

    #[test]
    fn parse_empty_is_empty() {
        assert!(parse_gateway_principals("").unwrap().is_empty());
        assert!(parse_gateway_principals("   ").unwrap().is_empty());
    }

    #[test]
    fn parse_drops_invalid_entries() {
        let raw = r#"[
            {"token":"tok-a","principal_id":"alice","role":"operator"},
            {"token":"  ","principal_id":"blank-token"},
            {"token":"tok-c","principal_id":"","role":"admin"},
            {"token":"tok-d","principal_id":"dave"}
        ]"#;
        let parsed = parse_gateway_principals(raw).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].principal_id, "alice");
        assert_eq!(parsed[0].role, GatewayRole::Operator);
        // Omitted role defaults to ReadOnly.
        assert_eq!(parsed[1].principal_id, "dave");
        assert_eq!(parsed[1].role, GatewayRole::ReadOnly);
    }

    #[test]
    fn parse_malformed_json_is_error() {
        assert!(parse_gateway_principals("{not json").is_err());
    }
}
