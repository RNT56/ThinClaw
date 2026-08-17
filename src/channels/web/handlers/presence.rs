//! Authenticated, privacy-scoped presence endpoints.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderName, StatusCode, header},
};
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::identity_helpers::{
    GatewayRequestIdentity, conversation_event_visible_to_identity,
};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{
    PresenceClearResponse, PresenceEventCause, PresencePublishRequest, PresencePublishResponse,
    PresenceScope, PresenceSnapshotResponse,
};
use thinclaw_gateway::web::presence::PresenceError;

#[derive(Debug, Deserialize)]
pub(crate) struct PresenceSnapshotQuery {
    pub thread_id: Option<Uuid>,
}

pub(crate) async fn presence_scope_authorized(
    state: &GatewayState,
    identity: &GatewayRequestIdentity,
    scope: &PresenceScope,
) -> bool {
    match scope {
        PresenceScope::Principal => true,
        PresenceScope::Thread { thread_id } => {
            conversation_event_visible_to_identity(
                state.store.as_ref(),
                state,
                identity,
                &thread_id.to_string(),
            )
            .await
        }
    }
}

fn forbidden_scope() -> (StatusCode, String) {
    // Deliberately indistinguishable from an unknown thread.
    (
        StatusCode::NOT_FOUND,
        "Presence scope not found".to_string(),
    )
}

fn presence_error(error: PresenceError) -> (StatusCode, String) {
    match error {
        PresenceError::SessionLimit
        | PresenceError::PrincipalSessionLimit
        | PresenceError::Capacity
        | PresenceError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, error.to_string()),
        PresenceError::StaleLease => (StatusCode::CONFLICT, error.to_string()),
        PresenceError::InvalidTtl | PresenceError::TypingRequiresThread => {
            (StatusCode::BAD_REQUEST, error.to_string())
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/presence",
    tag = "presence",
    params(("thread_id" = Option<Uuid>, Query, description = "Optional owned thread scope")),
    responses(
        (status = 200, description = "Current principal- or thread-scoped presence aggregates", body = PresenceSnapshotResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 404, description = "Thread scope not found or not owned by the caller"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn presence_snapshot_handler(
    State(state): State<Arc<GatewayState>>,
    identity: GatewayRequestIdentity,
    Query(query): Query<PresenceSnapshotQuery>,
) -> Result<
    (
        [(HeaderName, &'static str); 1],
        Json<PresenceSnapshotResponse>,
    ),
    (StatusCode, String),
> {
    let scope = query
        .thread_id
        .map_or(PresenceScope::Principal, |thread_id| {
            PresenceScope::Thread { thread_id }
        });
    if !presence_scope_authorized(state.as_ref(), &identity, &scope).await {
        return Err(forbidden_scope());
    }
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(PresenceSnapshotResponse {
            presences: state
                .sse
                .presence()
                .snapshot(&identity.principal_id, &scope)
                .await,
            server_time: Utc::now().to_rfc3339(),
        }),
    ))
}

#[utoipa::path(
    put,
    path = "/api/presence/{session_id}",
    tag = "presence",
    params(("session_id" = Uuid, Path, description = "Opaque client-generated session UUID")),
    request_body = PresencePublishRequest,
    responses(
        (status = 200, description = "Published or renewed presence", body = PresencePublishResponse),
        (status = 400, description = "Invalid TTL or scope/state combination"),
        (status = 401, description = "Missing or invalid gateway bearer token"),
        (status = 404, description = "Thread scope not found or not owned by the caller"),
        (status = 429, description = "Presence session, principal mutation-rate, or gateway capacity limit reached"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn presence_publish_handler(
    State(state): State<Arc<GatewayState>>,
    identity: GatewayRequestIdentity,
    Path(session_id): Path<Uuid>,
    Json(request): Json<PresencePublishRequest>,
) -> Result<Json<PresencePublishResponse>, (StatusCode, String)> {
    let scope = request.scope();
    if !presence_scope_authorized(state.as_ref(), &identity, &scope).await {
        return Err(forbidden_scope());
    }
    state
        .sse
        .presence()
        .publish(
            &identity.principal_id,
            &identity.actor_id,
            session_id,
            request,
            None,
        )
        .await
        .map(Json)
        .map_err(presence_error)
}

#[utoipa::path(
    delete,
    path = "/api/presence/{session_id}",
    tag = "presence",
    params(("session_id" = Uuid, Path, description = "Opaque client-generated session UUID")),
    responses(
        (status = 200, description = "Presence session cleared idempotently", body = PresenceClearResponse),
        (status = 401, description = "Missing or invalid gateway bearer token"),
    ),
    security(("gateway_token" = [])),
)]
pub(crate) async fn presence_clear_handler(
    State(state): State<Arc<GatewayState>>,
    identity: GatewayRequestIdentity,
    Path(session_id): Path<Uuid>,
) -> Json<PresenceClearResponse> {
    Json(
        state
            .sse
            .presence()
            .clear(
                &identity.principal_id,
                &identity.actor_id,
                session_id,
                PresenceEventCause::Clear,
            )
            .await,
    )
}
