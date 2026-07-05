//! OpenAPI assembly for the gateway's v1 mobile contract.
//!
//! Path annotations live next to their handlers in `handlers/*`; DTO schemas
//! live in `thinclaw_gateway::web::types` behind that crate's `openapi`
//! feature. This module only merges the per-group documents, describes the
//! two streaming endpoints that cannot carry `#[utoipa::path]` annotations,
//! and serves the result.
//!
//! The committed snapshot at `clients/openapi/thinclaw-gateway.openapi.json`
//! is regenerated with `cargo run --bin export-openapi -- generate` and
//! drift-checked in CI (`-- check`). See `docs/MOBILE_APP.md`.

use axum::Json;
use utoipa::openapi::content::ContentBuilder;
use utoipa::openapi::path::{OperationBuilder, PathItemBuilder};
use utoipa::openapi::request_body::RequestBodyBuilder;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::openapi::{HttpMethod, PathsBuilder, Ref, ResponseBuilder, SecurityRequirement};
use utoipa::{Modify, OpenApi};

use thinclaw_gateway::web::types::{SseEvent, WsClientMessage, WsServerMessage};

/// Contract version of the mobile API surface, independent of crate releases
/// so the committed snapshot only changes when the contract does.
const CONTRACT_VERSION: &str = "v1";

/// Adds the shared bearer-token security scheme referenced by the
/// `security(("gateway_token" = []))` clauses on the handler annotations.
struct GatewayTokenSecurity;

impl Modify for GatewayTokenSecurity {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "gateway_token",
            SecurityScheme::Http(
                HttpBuilder::new()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "Gateway bearer token (GATEWAY_AUTH_TOKEN or, once the \
                         device-identity layer ships, a per-device `tcd_` token).",
                    ))
                    .build(),
            ),
        );
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        super::handlers::chat::chat_send_handler,
        super::handlers::chat::chat_approval_handler,
        super::handlers::chat::chat_approvals_handler,
        super::handlers::chat::chat_abort_handler,
        super::handlers::chat::chat_history_handler,
        super::handlers::chat::chat_threads_handler,
        super::handlers::chat::chat_new_thread_handler,
        super::handlers::chat::chat_delete_thread_handler,
    ),
    components(schemas(
        SseEvent,
        WsClientMessage,
        WsServerMessage,
        thinclaw_gateway::web::types::PendingApprovalEntry,
        thinclaw_gateway::web::types::PendingApprovalsResponse
    ))
)]
struct ChatApiDoc;

#[derive(OpenApi)]
#[openapi(paths(
    super::handlers::jobs::jobs_list_handler,
    super::handlers::jobs::jobs_summary_handler,
    super::handlers::jobs::jobs_detail_handler,
))]
struct JobsApiDoc;

#[derive(OpenApi)]
#[openapi(
    paths(
        super::handlers::devices::devices_pair_start_handler,
        super::handlers::devices::devices_pair_pending_handler,
        super::handlers::devices::devices_pair_complete_handler,
        super::handlers::devices::devices_pair_approve_handler,
        super::handlers::devices::devices_list_handler,
        super::handlers::devices::devices_rename_handler,
        super::handlers::devices::devices_revoke_handler,
        super::handlers::devices::devices_rotate_handler,
        super::handlers::devices::devices_me_handler,
        super::handlers::devices::devices_me_push_register_handler,
        super::handlers::devices::devices_me_push_remove_handler,
        super::handlers::devices::devices_me_live_activity_register_handler,
        super::handlers::devices::devices_me_live_activity_remove_handler,
        super::handlers::devices::devices_me_live_activity_start_token_register_handler,
        super::handlers::devices::devices_me_live_activity_start_token_remove_handler,
    ),
    components(schemas(
        thinclaw_gateway::web::devices::QrPairingPayload,
        thinclaw_gateway::web::devices::PairStartResponse,
        thinclaw_gateway::web::devices::PairCompleteRequest,
        thinclaw_gateway::web::devices::PairCompleteResponse,
        thinclaw_gateway::web::devices::DeviceInfo,
        thinclaw_gateway::web::devices::DeviceListResponse,
        thinclaw_gateway::web::devices::RenameDeviceRequest,
        thinclaw_gateway::web::devices::RotateTokenResponse,
        thinclaw_gateway::web::devices::PendingPairInfo,
        thinclaw_gateway::web::devices::PendingPairListResponse,
        thinclaw_gateway::web::devices::DevicePlatform,
        thinclaw_gateway::web::devices::DeviceScope,
        thinclaw_gateway::web::devices::DeviceApnsRegistration,
        thinclaw_gateway::web::devices::DeviceLiveActivityKind,
        thinclaw_gateway::web::devices::DeviceLiveActivityToken,
        thinclaw_gateway::web::devices::RegisterPushRequest,
        thinclaw_gateway::web::devices::RegisterLiveActivityRequest,
        thinclaw_gateway::web::devices::RegisterLiveActivityStartTokenRequest,
        super::handlers::devices::PairPendingConfirmResponse,
    ))
)]
struct DevicesApiDoc;

#[derive(OpenApi)]
#[openapi(paths(
    super::handlers::gateway::health_handler,
    super::handlers::gateway::gateway_status_handler,
))]
struct StatusApiDoc;

/// The merged v1 mobile-contract document.
pub fn gateway_openapi() -> utoipa::openapi::OpenApi {
    let mut doc = ChatApiDoc::openapi();
    doc.merge(JobsApiDoc::openapi());
    doc.merge(StatusApiDoc::openapi());
    doc.merge(DevicesApiDoc::openapi());

    doc.info.title = "ThinClaw Gateway API".to_string();
    doc.info.version = CONTRACT_VERSION.to_string();
    doc.info.description = Some(
        "The v1 mobile contract of the ThinClaw web gateway: chat, threads, \
         approvals, read-only jobs, gateway status, and device identity \
         (pairing, device management, per-device tokens). Streaming events \
         are delivered over `/api/chat/events` (SSE) or `/api/chat/ws` \
         (WebSocket); both carry the `SseEvent` component schema."
            .to_string(),
    );

    GatewayTokenSecurity.modify(&mut doc);

    let streaming = PathsBuilder::new()
        .path("/api/chat/events", sse_path_item())
        .path("/api/chat/ws", ws_path_item())
        .build();
    doc.paths.paths.extend(streaming.paths);

    doc
}

/// `GET /api/chat/events` — cannot be expressed with `#[utoipa::path]`
/// because the handler returns an infinite `text/event-stream`.
fn sse_path_item() -> utoipa::openapi::path::PathItem {
    let operation = OperationBuilder::new()
        .tag("chat")
        .operation_id(Some("chat_events_sse"))
        .description(Some(
            "Server-Sent Events stream of agent events. Each `data:` line is a \
             JSON-encoded `SseEvent` discriminated by its `type` field. The \
             stream has no replay: after a reconnect, clients must reconcile \
             missed turns via `GET /api/chat/history`. Browser EventSource \
             clients may pass the bearer token as a `token` query parameter; \
             native clients must use the Authorization header.",
        ))
        .response(
            "200",
            ResponseBuilder::new()
                .description("Event stream (never terminates normally)")
                .content(
                    "text/event-stream",
                    ContentBuilder::new()
                        .schema(Some(Ref::from_schema_name("SseEvent")))
                        .build(),
                )
                .build(),
        )
        .response(
            "401",
            ResponseBuilder::new()
                .description("Missing or invalid gateway bearer token")
                .build(),
        )
        .security(SecurityRequirement::new(
            "gateway_token",
            Vec::<String>::new(),
        ))
        .build();

    PathItemBuilder::new()
        .operation(HttpMethod::Get, operation)
        .build()
}

/// `GET /api/chat/ws` — WebSocket upgrade; frames are documented via the
/// `WsClientMessage` / `WsServerMessage` component schemas.
fn ws_path_item() -> utoipa::openapi::path::PathItem {
    let operation = OperationBuilder::new()
        .tag("chat")
        .operation_id(Some("chat_ws"))
        .description(Some(
            "WebSocket upgrade for bidirectional chat. Client frames are \
             JSON-encoded `WsClientMessage`; server frames are JSON-encoded \
             `WsServerMessage` (SSE events in an envelope plus `pong`).",
        ))
        .request_body(Some(
            RequestBodyBuilder::new()
                .description(Some(
                    "Not a conventional request body: after the 101 upgrade, \
                     the client sends `WsClientMessage` frames.",
                ))
                .content(
                    "application/json",
                    ContentBuilder::new()
                        .schema(Some(Ref::from_schema_name("WsClientMessage")))
                        .build(),
                )
                .build(),
        ))
        .response(
            "101",
            ResponseBuilder::new()
                .description("Switching protocols; server frames are `WsServerMessage`")
                .content(
                    "application/json",
                    ContentBuilder::new()
                        .schema(Some(Ref::from_schema_name("WsServerMessage")))
                        .build(),
                )
                .build(),
        )
        .response(
            "401",
            ResponseBuilder::new()
                .description("Missing or invalid gateway bearer token")
                .build(),
        )
        .security(SecurityRequirement::new(
            "gateway_token",
            Vec::<String>::new(),
        ))
        .build();

    PathItemBuilder::new()
        .operation(HttpMethod::Get, operation)
        .build()
}

/// `GET /api/openapi.json` (protected router).
pub(crate) async fn openapi_json_handler() -> Json<utoipa::openapi::OpenApi> {
    Json(gateway_openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maintained list of every path the v1 contract promises. Update this
    /// alongside `#[utoipa::path]` annotations and `server.rs` routes; the
    /// committed snapshot check will also fail on unintended drift.
    const EXPECTED_PATHS: &[&str] = &[
        "/api/chat/send",
        "/api/chat/approval",
        "/api/chat/approvals",
        "/api/chat/abort",
        "/api/chat/history",
        "/api/chat/threads",
        "/api/chat/thread/new",
        "/api/chat/thread/{id}",
        "/api/chat/events",
        "/api/chat/ws",
        "/api/jobs",
        "/api/jobs/summary",
        "/api/jobs/{id}",
        "/api/health",
        "/api/gateway/status",
        "/api/devices/pair/start",
        "/api/devices/pair/pending",
        "/api/devices/pair/complete",
        "/api/devices/pair/{id}/approve",
        "/api/devices",
        "/api/devices/{id}/rename",
        "/api/devices/{id}/revoke",
        "/api/devices/{id}/rotate",
        "/api/devices/me",
        "/api/devices/me/push",
        "/api/devices/me/live-activity/{activity_id}",
        "/api/devices/me/live-activity-start-token",
    ];

    #[test]
    fn contract_paths_match_expected_list() {
        let doc = gateway_openapi();
        let mut actual: Vec<&str> = doc.paths.paths.keys().map(String::as_str).collect();
        let mut expected = EXPECTED_PATHS.to_vec();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(
            actual, expected,
            "OpenAPI paths drifted from the maintained contract list"
        );
    }

    #[test]
    fn streaming_component_schemas_are_registered() {
        let doc = gateway_openapi();
        let components = doc.components.expect("components present");
        for name in ["SseEvent", "WsClientMessage", "WsServerMessage"] {
            assert!(
                components.schemas.contains_key(name),
                "missing component schema {name}"
            );
        }
        assert!(components.security_schemes.contains_key("gateway_token"));
    }

    #[test]
    fn device_and_approvals_component_schemas_are_registered() {
        let doc = gateway_openapi();
        let components = doc.components.expect("components present");
        for name in ["PendingApprovalEntry", "PendingApprovalsResponse"] {
            assert!(
                components.schemas.contains_key(name),
                "missing component schema {name}"
            );
        }
    }

    #[test]
    fn skip_serializing_fields_stay_out_of_wire_format() {
        // `ConversationDeleted.principal_id`/`actor_id` are `skip_serializing`
        // + `schema(ignore)`; the wire format and the schema must agree.
        let event = SseEvent::ConversationDeleted {
            thread_id: "t-1".to_string(),
            principal_id: "p-1".to_string(),
            actor_id: "a-1".to_string(),
        };
        let json = serde_json::to_value(&event).expect("serializes");
        assert_eq!(json["type"], "conversation_deleted");
        assert_eq!(json["thread_id"], "t-1");
        assert!(json.get("principal_id").is_none());
        assert!(json.get("actor_id").is_none());
    }
}
