//! Chat send/approval/history/thread DTOs.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
pub struct HistoryQuery {
    pub thread_id: Option<String>,
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub user_id: Option<String>,
    pub actor_id: Option<String>,
}
