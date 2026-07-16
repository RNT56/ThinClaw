//! Chat send/approval/history/thread DTOs.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::web::devices::ApprovalRisk;

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SendMessageRequest {
    #[serde(alias = "message")]
    pub content: String,
    pub thread_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
    /// Optional client-generated UUID used to make offline/retry sends
    /// idempotent across ambiguous transport failures.
    #[serde(default)]
    pub client_message_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SendMessageResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadInfo {
    pub id: Uuid,
    pub state: String,
    pub turn_count: usize,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_type: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadListResponse {
    /// The pinned assistant thread (always present after first load).
    ///
    /// Schema note: emitted as a non-nullable, optional `$ref` to `ThreadInfo`
    /// (`schema(nullable = false)` + `skip_serializing_if`), rather than the
    /// default `oneOf: [null, $ref]` that utoipa produces for `Option<$ref>`.
    /// swift-openapi-generator drops a `oneOf`-with-null property entirely, so
    /// the null-union would make `assistant_thread` invisible to the generated
    /// iOS client. The optional-ref shape keeps it as `ThreadInfo?`. On the wire
    /// the field is now absent (not `null`) when `None`; the web UI already
    /// coalesces via `data.assistant_thread || null`, so absent and `null` are
    /// equivalent there.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(nullable = false))]
    pub assistant_thread: Option<ThreadInfo>,
    /// Regular conversation threads.
    pub threads: Vec<ThreadInfo>,
    pub active_thread: Option<Uuid>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TurnInfo {
    pub turn_number: usize,
    pub user_input: String,
    #[serde(default)]
    pub hide_user_input: bool,
    pub response: Option<String>,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub tool_calls: Vec<ToolCallInfo>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolCallInfo {
    pub name: String,
    pub has_result: bool,
    pub has_error: bool,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HistoryResponse {
    pub thread_id: Uuid,
    pub turns: Vec<TurnInfo>,
    /// Whether there are older messages available.
    #[serde(default)]
    pub has_more: bool,
    /// Cursor for the next page (ISO8601 timestamp of the oldest message returned).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_timestamp: Option<String>,
}

// --- Approval ---

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalRequest {
    pub request_id: String,
    /// "approve", "always", or "deny"
    pub action: String,
    /// Thread that owns the pending approval (so the agent loop finds the right session).
    pub thread_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
}

/// One pending tool-approval request, as surfaced by `GET /api/chat/approvals`
/// (milestone B1 approvals-pull endpoint for mobile clients that are not
/// holding an open SSE/WS stream when the approval is raised).
///
/// Populated from the `StatusUpdate::ApprovalNeeded` -> `SseEvent::ApprovalNeeded`
/// projection at broadcast time (see `src/channels/web/mod.rs::send_status`),
/// and removed once the underlying decision is accepted. The gateway persists
/// this registry so reconnecting mobile clients receive an authoritative
/// snapshot across process restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PendingApprovalEntry {
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    /// Pretty-printed JSON, matching `SseEvent::ApprovalNeeded.parameters`.
    pub parameters: String,
    /// Gateway-computed risk tier (D-K3), matching
    /// `SseEvent::ApprovalNeeded.risk`. Always present.
    pub risk: ApprovalRisk,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// RFC 3339 timestamp; entries are returned sorted oldest-first.
    pub created_at: String,
}

/// `GET /api/chat/approvals` response.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PendingApprovalsResponse {
    pub approvals: Vec<PendingApprovalEntry>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadCommandRequest {
    pub thread_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadCommandResponse {
    pub message_id: Uuid,
    pub status: &'static str,
}

#[derive(Debug, Deserialize)]
pub struct ThreadExportQuery {
    pub format: Option<String>,
    pub user_id: Option<String>,
    pub actor_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThreadExportResponse {
    pub thread_id: Uuid,
    pub format: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct HistoryQuery {
    pub thread_id: Option<String>,
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub user_id: Option<String>,
    pub actor_id: Option<String>,
}
