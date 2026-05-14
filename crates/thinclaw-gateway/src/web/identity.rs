use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};

use thinclaw_identity::{ConversationKind, ResolvedIdentity, scope_id_from_key};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayAuthSource {
    BearerHeader,
    BearerQuery,
    TrustedProxy,
}

impl GatewayAuthSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BearerHeader => "bearer_header",
            Self::BearerQuery => "bearer_query",
            Self::TrustedProxy => "trusted_proxy",
        }
    }

    pub fn allows_compat_overrides(&self) -> bool {
        matches!(self, Self::BearerHeader | Self::BearerQuery)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRequestIdentity {
    pub principal_id: String,
    pub actor_id: String,
    pub auth_source: GatewayAuthSource,
    pub compatibility_fallback: bool,
}

impl GatewayRequestIdentity {
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
        }
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
