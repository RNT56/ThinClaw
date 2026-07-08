use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};

use thinclaw_identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};
use thinclaw_settings::GatewayRole;

use crate::web::devices::{DevicePlatform, DeviceScope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayAuthSource {
    BearerHeader,
    BearerQuery,
    TrustedProxy,
    DeviceToken,
}

impl GatewayAuthSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BearerHeader => "bearer_header",
            Self::BearerQuery => "bearer_query",
            Self::TrustedProxy => "trusted_proxy",
            Self::DeviceToken => "device_token",
        }
    }

    /// Whether this auth source may honor request-supplied
    /// `user_id`/`actor_id` compatibility overrides.
    ///
    /// Device principals must never be able to override their identity via
    /// request params — a paired device always acts as the operator
    /// (`fallback_principal_id`/`fallback_actor_id`), never as an arbitrary
    /// caller-chosen identity. See `docs/MOBILE_SECURITY.md` D-T4 and the
    /// gateway hardening checklist (§8).
    pub fn allows_compat_overrides(&self) -> bool {
        matches!(self, Self::BearerHeader | Self::BearerQuery)
    }
}

/// Attached to a request's extensions when it authenticated with a device
/// token (see `crate::web::auth::auth_middleware`). Scope-enforcement
/// middleware and stream handlers (SSE/WS revocation teardown) key off this
/// to know the request carries a device principal and which scopes it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceContext {
    pub device_id: String,
    pub scopes: Vec<DeviceScope>,
    /// The device's platform family, carried so handlers can apply device-
    /// class policy (milestone M4: the approve handler's watch low-risk-only
    /// rule keys off `platform == Watchos`).
    pub platform: DevicePlatform,
    /// True if this device is a companion (minted by a parent). Companions are
    /// subject to the low-risk-only approval rule (D-K4).
    pub is_companion: bool,
}

impl DeviceContext {
    /// Construct a context for a top-level (non-companion) device with an
    /// unspecified platform. Kept for tests and callers that only care about
    /// scopes; production auth uses [`DeviceContext::with_class`].
    pub fn new(device_id: impl Into<String>, scopes: Vec<DeviceScope>) -> Self {
        Self {
            device_id: device_id.into(),
            scopes,
            platform: DevicePlatform::Other(String::new()),
            is_companion: false,
        }
    }

    /// Construct a context carrying the device's class (platform + companion
    /// flag), as the auth middleware does for device-authenticated requests.
    pub fn with_class(
        device_id: impl Into<String>,
        scopes: Vec<DeviceScope>,
        platform: DevicePlatform,
        is_companion: bool,
    ) -> Self {
        Self {
            device_id: device_id.into(),
            scopes,
            platform,
            is_companion,
        }
    }

    pub fn has_scope(&self, scope: DeviceScope) -> bool {
        self.scopes.contains(&scope)
    }

    /// True if this device is a companion running on watchOS — the class that
    /// is restricted to low-risk approvals server-side (milestone M4, D-K3/
    /// D-K4). The watch UI must not offer high-risk approvals anyway; this is
    /// the server-side backstop.
    pub fn is_watch_companion(&self) -> bool {
        self.is_companion && matches!(self.platform, DevicePlatform::Watchos)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRequestIdentity {
    pub principal_id: String,
    pub actor_id: String,
    pub auth_source: GatewayAuthSource,
    pub compatibility_fallback: bool,
    /// RBAC privilege tier for this request. See `crate::web::rbac`.
    pub role: GatewayRole,
}

impl GatewayRequestIdentity {
    /// Construct an identity. The role defaults to the least-privileged
    /// [`GatewayRole::ReadOnly`] (fail-closed): callers that authenticate a
    /// full-privilege context (primary bearer token, trusted proxy) opt into
    /// admin explicitly via [`Self::with_role`], and any future construction
    /// that forgets to set a role can only under-privilege, never escalate.
    pub fn new(
        principal_id: impl Into<String>,
        actor_id: impl Into<String>,
        auth_source: GatewayAuthSource,
        compatibility_fallback: bool,
    ) -> Self {
        Self {
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            auth_source,
            compatibility_fallback,
            role: GatewayRole::ReadOnly,
        }
    }

    /// Set the RBAC role, returning the updated identity.
    pub fn with_role(mut self, role: GatewayRole) -> Self {
        self.role = role;
        self
    }

    pub fn resolved_identity(&self, thread_id: Option<&str>) -> ResolvedIdentity {
        gateway_identity(&self.principal_id, &self.actor_id, thread_id)
    }

    pub fn with_compat_overrides(
        &self,
        requested_principal_id: Option<&str>,
        requested_actor_id: Option<&str>,
    ) -> Self {
        let principal_id = requested_identity_override(requested_principal_id)
            .unwrap_or_else(|| self.principal_id.clone());
        let actor_id = requested_identity_override(requested_actor_id).unwrap_or_else(|| {
            if self.actor_id.trim().is_empty() {
                principal_id.clone()
            } else {
                self.actor_id.clone()
            }
        });
        let compatibility_fallback = self.compatibility_fallback
            || requested_identity_override(requested_principal_id).is_some()
            || requested_identity_override(requested_actor_id).is_some();

        Self {
            principal_id,
            actor_id,
            auth_source: self.auth_source.clone(),
            compatibility_fallback,
            role: self.role,
        }
    }
}

impl<S> FromRequestParts<S> for GatewayRequestIdentity
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, String);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<GatewayRequestIdentity>()
            .cloned()
            .ok_or((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Gateway request identity missing from request context".to_string(),
            ))
    }
}

pub fn requested_identity_override(requested: Option<&str>) -> Option<String> {
    requested
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

pub fn gateway_identity(
    principal_id: &str,
    actor_id: &str,
    thread_id: Option<&str>,
) -> ResolvedIdentity {
    let stable_external_conversation_key = match thread_id {
        Some(thread_id) => {
            format!("gateway://direct/{principal_id}/actor/{actor_id}/thread/{thread_id}")
        }
        None => format!("gateway://direct/{principal_id}/actor/{actor_id}"),
    };

    ResolvedIdentity {
        principal_id: principal_id.to_string(),
        actor_id: actor_id.to_string(),
        conversation_scope_id: scope_id_from_key(&format!("principal:{principal_id}")),
        conversation_kind: ConversationKind::Direct,
        raw_sender_id: actor_id.to_string(),
        stable_external_conversation_key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_context_new_is_non_companion_and_no_watch_class() {
        let ctx = DeviceContext::new("d1", vec![DeviceScope::Chat]);
        assert!(!ctx.is_companion);
        assert!(!ctx.is_watch_companion());
    }

    #[test]
    fn watch_companion_requires_both_companion_and_watchos() {
        // Companion on watchOS => watch companion (the gated class).
        let watch = DeviceContext::with_class(
            "watch",
            vec![DeviceScope::Chat, DeviceScope::Approvals],
            DevicePlatform::Watchos,
            true,
        );
        assert!(watch.is_watch_companion());

        // Companion on a non-watch platform (e.g. a future iPad companion) is
        // NOT gated by the watch rule.
        let ipad_companion = DeviceContext::with_class(
            "ipad",
            vec![DeviceScope::Chat, DeviceScope::Approvals],
            DevicePlatform::Ipados,
            true,
        );
        assert!(!ipad_companion.is_watch_companion());

        // A top-level watchOS device (not a companion) is not gated either —
        // in practice the watch is always minted as a companion, but the guard
        // is precise about requiring the companion flag.
        let top_level_watch = DeviceContext::with_class(
            "watch2",
            vec![DeviceScope::Chat],
            DevicePlatform::Watchos,
            false,
        );
        assert!(!top_level_watch.is_watch_companion());
    }
}
