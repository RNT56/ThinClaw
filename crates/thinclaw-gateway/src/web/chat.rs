//! Root-independent web chat projection helpers.

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_history::ConversationMessage;
use uuid::Uuid;

use crate::web::ports::{message_hidden_from_main_chat, message_is_startup_hook};
use crate::web::types::{
    ActionResponse, HistoryQuery, HistoryResponse, SendMessageResponse, ThreadCommandResponse,
    ThreadExportResponse, ThreadInfo, ThreadListResponse, ToolCallInfo, TurnInfo,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayChatMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayTurnInfo {
    pub turn_number: usize,
    pub user_input: String,
    #[serde(default)]
    pub hide_user_input: bool,
    pub response: Option<String>,
    pub state: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<GatewayTurnToolCallInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayTurnToolCallInfo {
    pub name: String,
    pub has_result: bool,
    pub has_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewaySessionToolCallInfo {
    pub name: String,
    pub has_result: bool,
    pub has_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewaySessionTurnInfo {
    pub turn_number: usize,
    pub user_input: String,
    #[serde(default)]
    pub hide_user_input: bool,
    pub response: Option<String>,
    pub state: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub tool_calls: Vec<GatewaySessionToolCallInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadInfoInput {
    pub id: Uuid,
    pub state: String,
    pub turn_count: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub title: Option<String>,
    pub thread_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayThreadSummaryInput {
    pub id: Uuid,
    pub message_count: i64,
    pub started_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
    pub title: Option<String>,
    pub thread_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayThreadExportMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub actor_id: Option<String>,
    pub actor_display_name: Option<String>,
    pub raw_sender_id: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChatThreadDeleteResponse {
    pub deleted: bool,
    pub thread_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatHistoryQueryOptions {
    pub limit: usize,
    pub before_cursor: Option<DateTime<Utc>>,
}

/// Result of a framework-agnostic send-message call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SendMessageResult {
    /// Unique ID for this message, used by callers to correlate async events.
    pub message_id: Uuid,
    /// Immediate submission status; response content is delivered separately.
    pub status: String,
}

pub const CHAT_RATE_LIMIT_MESSAGE: &str = "Rate limit exceeded. Try again shortly.";
pub const INVALID_APPROVAL_REQUEST_ID_MESSAGE: &str = "Invalid request_id (expected UUID)";
pub const EXTENSION_MANAGER_UNAVAILABLE_MESSAGE: &str = "Extension manager not available";
pub const TOO_MANY_CHAT_CONNECTIONS_MESSAGE: &str = "Too many connections";
pub const SESSION_MANAGER_UNAVAILABLE_MESSAGE: &str = "Session manager not available";
pub const INVALID_BEFORE_TIMESTAMP_MESSAGE: &str = "Invalid 'before' timestamp";
pub const INVALID_THREAD_QUERY_ID_MESSAGE: &str = "Invalid thread_id";
pub const INVALID_THREAD_PATH_ID_MESSAGE: &str = "Invalid thread id";
pub const INVALID_THREAD_DELETE_ID_MESSAGE: &str = "Invalid thread ID";
pub const NO_ACTIVE_THREAD_MESSAGE: &str = "No active thread";
pub const THREAD_NOT_FOUND_MESSAGE: &str = "Thread not found";
pub const CHAT_STORE_UNAVAILABLE_MESSAGE: &str = "Store not available";
pub const CHAT_DATABASE_UNAVAILABLE_MESSAGE: &str = "Database not available";
pub const DELETE_ASSISTANT_THREAD_FORBIDDEN_MESSAGE: &str = "Cannot delete the Assistant thread";
pub const EMPTY_CHAT_MESSAGE_CONTENT_MESSAGE: &str = "Message content is empty";

pub fn chat_rate_limit_error() -> (StatusCode, String) {
    (
        StatusCode::TOO_MANY_REQUESTS,
        CHAT_RATE_LIMIT_MESSAGE.to_string(),
    )
}

pub fn empty_chat_message_content_message() -> String {
    EMPTY_CHAT_MESSAGE_CONTENT_MESSAGE.to_string()
}

pub fn chat_cancel_failed_message(error: impl std::fmt::Display) -> String {
    format!("Cancel failed: {error}")
}

pub fn unknown_approval_action_error(action: impl AsRef<str>) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        format!("Unknown action: {}", action.as_ref()),
    )
}

pub fn parse_approval_request_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            INVALID_APPROVAL_REQUEST_ID_MESSAGE.to_string(),
        )
    })
}

pub fn extension_manager_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        EXTENSION_MANAGER_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn too_many_chat_connections_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        TOO_MANY_CHAT_CONNECTIONS_MESSAGE.to_string(),
    )
}

pub fn session_manager_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        SESSION_MANAGER_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn invalid_before_timestamp_error() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        INVALID_BEFORE_TIMESTAMP_MESSAGE.to_string(),
    )
}

pub fn invalid_before_timestamp_message() -> String {
    INVALID_BEFORE_TIMESTAMP_MESSAGE.to_string()
}

pub fn normalize_chat_history_query(
    query: &HistoryQuery,
) -> Result<ChatHistoryQueryOptions, (StatusCode, String)> {
    let before_cursor = query
        .before
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| invalid_before_timestamp_error())
        })
        .transpose()?;

    Ok(ChatHistoryQueryOptions {
        limit: query.limit.unwrap_or(50),
        before_cursor,
    })
}

pub fn parse_chat_thread_uuid(id: &str) -> Result<Uuid, uuid::Error> {
    Uuid::parse_str(id)
}

pub fn parse_chat_thread_query_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            INVALID_THREAD_QUERY_ID_MESSAGE.to_string(),
        )
    })
}

pub fn parse_chat_thread_path_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            INVALID_THREAD_PATH_ID_MESSAGE.to_string(),
        )
    })
}

pub fn parse_chat_thread_delete_id(id: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            INVALID_THREAD_DELETE_ID_MESSAGE.to_string(),
        )
    })
}

pub fn no_active_thread_error() -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, NO_ACTIVE_THREAD_MESSAGE.to_string())
}

pub fn no_active_thread_message() -> String {
    NO_ACTIVE_THREAD_MESSAGE.to_string()
}

pub fn resolve_chat_history_thread_id(
    query_thread_id: Option<&str>,
    active_thread: Option<Uuid>,
) -> Result<Uuid, (StatusCode, String)> {
    if let Some(thread_id) = query_thread_id {
        parse_chat_thread_query_id(thread_id)
    } else {
        active_thread.ok_or_else(no_active_thread_error)
    }
}

pub fn thread_not_found_error() -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, THREAD_NOT_FOUND_MESSAGE.to_string())
}

pub fn thread_not_found_message() -> String {
    THREAD_NOT_FOUND_MESSAGE.to_string()
}

pub fn chat_store_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        CHAT_STORE_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn chat_database_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        CHAT_DATABASE_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn delete_assistant_thread_forbidden_error() -> (StatusCode, String) {
    (
        StatusCode::FORBIDDEN,
        DELETE_ASSISTANT_THREAD_FORBIDDEN_MESSAGE.to_string(),
    )
}

pub fn chat_thread_delete_response(deleted: bool, thread_id: Uuid) -> ChatThreadDeleteResponse {
    ChatThreadDeleteResponse {
        deleted,
        thread_id: thread_id.to_string(),
    }
}

pub fn send_message_response(message_id: Uuid) -> SendMessageResponse {
    SendMessageResponse {
        message_id,
        status: "accepted",
    }
}

pub fn thread_command_response(message_id: Uuid) -> ThreadCommandResponse {
    ThreadCommandResponse {
        message_id,
        status: "accepted",
    }
}

pub fn chat_auth_success_response(message: impl Into<String>) -> ActionResponse {
    ActionResponse::ok(message)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatAuthRequiredResponseInput {
    pub auth_url: Option<String>,
    pub setup_url: Option<String>,
    pub auth_mode: String,
    pub auth_status: String,
    pub awaiting_token: bool,
    pub instructions: Option<String>,
    pub shared_auth_provider: Option<String>,
    pub missing_scopes: Vec<String>,
}

pub fn chat_auth_required_response(input: ChatAuthRequiredResponseInput) -> ActionResponse {
    let mut response = ActionResponse::fail(
        input
            .instructions
            .clone()
            .unwrap_or_else(|| "Invalid token".to_string()),
    );
    response.auth_url = input.auth_url;
    response.setup_url = input.setup_url;
    response.auth_mode = Some(input.auth_mode);
    response.auth_status = Some(input.auth_status);
    response.awaiting_token = Some(input.awaiting_token);
    response.instructions = input.instructions;
    response.shared_auth_provider = input.shared_auth_provider;
    response.missing_scopes = input.missing_scopes;
    response
}

pub fn chat_auth_cancel_response() -> ActionResponse {
    ActionResponse::ok("Auth cancelled")
}

pub fn turn_info_from_gateway_turn(turn: GatewayTurnInfo) -> TurnInfo {
    TurnInfo {
        turn_number: turn.turn_number,
        user_input: turn.user_input,
        hide_user_input: turn.hide_user_input,
        response: turn.response,
        state: turn.state,
        started_at: turn.started_at,
        completed_at: turn.completed_at,
        tool_calls: turn
            .tool_calls
            .into_iter()
            .map(|tool_call| ToolCallInfo {
                name: tool_call.name,
                has_result: tool_call.has_result,
                has_error: tool_call.has_error,
            })
            .collect(),
    }
}

pub fn turn_info_from_session_turn(turn: GatewaySessionTurnInfo) -> TurnInfo {
    TurnInfo {
        turn_number: turn.turn_number,
        user_input: if turn.hide_user_input {
            String::new()
        } else {
            turn.user_input
        },
        hide_user_input: turn.hide_user_input,
        response: turn.response,
        state: turn.state,
        started_at: turn.started_at.to_rfc3339(),
        completed_at: turn.completed_at.map(|timestamp| timestamp.to_rfc3339()),
        tool_calls: turn
            .tool_calls
            .into_iter()
            .map(|tool_call| ToolCallInfo {
                name: tool_call.name,
                has_result: tool_call.has_result,
                has_error: tool_call.has_error,
            })
            .collect(),
    }
}

pub fn history_response(
    thread_id: Uuid,
    turns: Vec<TurnInfo>,
    has_more: bool,
    oldest_timestamp: Option<DateTime<Utc>>,
) -> HistoryResponse {
    HistoryResponse {
        thread_id,
        turns,
        has_more,
        oldest_timestamp: oldest_timestamp.map(|timestamp| timestamp.to_rfc3339()),
    }
}

pub fn thread_info(input: ThreadInfoInput) -> ThreadInfo {
    ThreadInfo {
        id: input.id,
        state: input.state,
        turn_count: input.turn_count,
        created_at: input.created_at.to_rfc3339(),
        updated_at: input.updated_at.to_rfc3339(),
        title: input.title,
        thread_type: input.thread_type,
    }
}

pub fn thread_list_response(
    assistant_thread: Option<ThreadInfo>,
    threads: Vec<ThreadInfo>,
    active_thread: Option<Uuid>,
) -> ThreadListResponse {
    ThreadListResponse {
        assistant_thread,
        threads,
        active_thread,
    }
}

pub fn thread_list_response_from_summaries(
    assistant_id: Uuid,
    summaries: impl IntoIterator<Item = GatewayThreadSummaryInput>,
    active_thread: Option<Uuid>,
    synthesized_assistant_created_at: DateTime<Utc>,
    synthesized_assistant_updated_at: DateTime<Utc>,
) -> ThreadListResponse {
    let mut assistant_thread = None;
    let mut threads = Vec::new();

    for summary in summaries {
        let info = thread_info(ThreadInfoInput {
            id: summary.id,
            state: "Idle".to_string(),
            turn_count: (summary.message_count / 2).max(0) as usize,
            created_at: summary.started_at,
            updated_at: summary.last_activity,
            title: summary.title,
            thread_type: summary.thread_type,
        });

        if summary.id == assistant_id {
            assistant_thread = Some(info);
        } else {
            threads.push(info);
        }
    }

    if assistant_thread.is_none() {
        assistant_thread = Some(thread_info(ThreadInfoInput {
            id: assistant_id,
            state: "Idle".to_string(),
            turn_count: 0,
            created_at: synthesized_assistant_created_at,
            updated_at: synthesized_assistant_updated_at,
            title: None,
            thread_type: Some("assistant".to_string()),
        }));
    }

    thread_list_response(assistant_thread, threads, active_thread)
}

pub fn thread_export_content(
    format: &str,
    messages: &[GatewayThreadExportMessage],
) -> Result<String, serde_json::Error> {
    if format == "json" {
        return serde_json::to_string_pretty(messages);
    }

    Ok(messages
        .iter()
        .map(|message| format!("## {}\n\n{}", message.role, message.content))
        .collect::<Vec<_>>()
        .join("\n\n"))
}

pub fn thread_export_response(
    thread_id: Uuid,
    format: impl Into<String>,
    content: impl Into<String>,
) -> ThreadExportResponse {
    ThreadExportResponse {
        thread_id,
        format: format.into(),
        content: content.into(),
    }
}

pub fn build_turns_from_messages(messages: &[GatewayChatMessage]) -> Vec<GatewayTurnInfo> {
    let mut turns = Vec::new();
    let mut turn_number = 0;
    let mut iter = messages.iter().peekable();

    while let Some(msg) = iter.next() {
        if msg.role == "user" {
            let hide_user_input = message_hidden_from_main_chat(&msg.metadata);

            let mut turn = GatewayTurnInfo {
                turn_number,
                user_input: if hide_user_input {
                    String::new()
                } else {
                    msg.content.clone()
                },
                hide_user_input,
                response: None,
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: None,
                tool_calls: Vec::new(),
            };

            if let Some(next) = iter.peek()
                && next.role == "assistant"
            {
                let assistant_msg = iter.next().expect("peeked");
                turn.response = Some(assistant_msg.content.clone());
                turn.completed_at = Some(assistant_msg.created_at.to_rfc3339());
            }

            if turn.response.is_none() {
                turn.state = "Failed".to_string();
            }

            if turn.hide_user_input && turn.response.is_none() {
                continue;
            }

            turns.push(turn);
            turn_number += 1;
        } else if msg.role == "assistant" && message_is_startup_hook(&msg.metadata) {
            turns.push(GatewayTurnInfo {
                turn_number,
                user_input: String::new(),
                hide_user_input: true,
                response: Some(msg.content.clone()),
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: Some(msg.created_at.to_rfc3339()),
                tool_calls: Vec::new(),
            });
            turn_number += 1;
        }
    }

    turns
}

pub fn gateway_chat_message_from_history(message: &ConversationMessage) -> GatewayChatMessage {
    GatewayChatMessage {
        role: message.role.clone(),
        content: message.content.clone(),
        metadata: message.metadata.clone(),
        created_at: message.created_at,
    }
}

pub fn turns_from_history_messages(messages: &[ConversationMessage]) -> Vec<TurnInfo> {
    let gateway_messages = messages
        .iter()
        .map(gateway_chat_message_from_history)
        .collect::<Vec<_>>();

    build_turns_from_messages(&gateway_messages)
        .into_iter()
        .map(turn_info_from_gateway_turn)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(
        role: &str,
        content: &str,
        created_at: DateTime<Utc>,
        metadata: serde_json::Value,
    ) -> GatewayChatMessage {
        GatewayChatMessage {
            role: role.to_string(),
            content: content.to_string(),
            metadata,
            created_at,
        }
    }

    #[test]
    fn builds_complete_turns_from_user_assistant_pairs() {
        let now = Utc::now();
        let messages = vec![
            message("user", "Hello", now, serde_json::json!({})),
            message(
                "assistant",
                "Hi there!",
                now + chrono::TimeDelta::seconds(1),
                serde_json::json!({}),
            ),
            message(
                "user",
                "How are you?",
                now + chrono::TimeDelta::seconds(2),
                serde_json::json!({}),
            ),
            message(
                "assistant",
                "Doing well!",
                now + chrono::TimeDelta::seconds(3),
                serde_json::json!({}),
            ),
        ];

        let turns = build_turns_from_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "Hello");
        assert_eq!(turns[0].response.as_deref(), Some("Hi there!"));
        assert_eq!(turns[0].state, "Completed");
        assert_eq!(turns[1].user_input, "How are you?");
        assert_eq!(turns[1].response.as_deref(), Some("Doing well!"));
    }

    #[test]
    fn history_messages_project_to_web_turns() {
        let now = Utc::now();
        let messages = vec![
            ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now,
            },
            ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
                actor_id: None,
                actor_display_name: None,
                raw_sender_id: None,
                metadata: serde_json::json!({}),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
        ];

        let turns = turns_from_history_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_input, "Hello");
        assert_eq!(turns[0].response.as_deref(), Some("Hi there!"));
    }

    #[test]
    fn marks_incomplete_last_user_turn_failed() {
        let now = Utc::now();
        let messages = vec![
            message("user", "Hello", now, serde_json::json!({})),
            message(
                "assistant",
                "Hi!",
                now + chrono::TimeDelta::seconds(1),
                serde_json::json!({}),
            ),
            message(
                "user",
                "Lost message",
                now + chrono::TimeDelta::seconds(2),
                serde_json::json!({}),
            ),
        ];

        let turns = build_turns_from_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].user_input, "Lost message");
        assert!(turns[1].response.is_none());
        assert_eq!(turns[1].state, "Failed");
    }

    #[test]
    fn hides_startup_user_prompt_but_keeps_reply() {
        let now = Utc::now();
        let messages = vec![
            message(
                "user",
                "boot prompt",
                now,
                serde_json::json!({"hide_from_webui_chat": true}),
            ),
            message(
                "assistant",
                "boot reply",
                now + chrono::TimeDelta::seconds(1),
                serde_json::json!({"synthetic_origin": "startup_hook"}),
            ),
            message(
                "user",
                "real question",
                now + chrono::TimeDelta::seconds(2),
                serde_json::json!({}),
            ),
            message(
                "assistant",
                "real answer",
                now + chrono::TimeDelta::seconds(3),
                serde_json::json!({}),
            ),
        ];

        let turns = build_turns_from_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert!(turns[0].hide_user_input);
        assert_eq!(turns[0].user_input, "");
        assert_eq!(turns[0].response.as_deref(), Some("boot reply"));
        assert_eq!(turns[1].user_input, "real question");
        assert_eq!(turns[1].response.as_deref(), Some("real answer"));
    }

    #[test]
    fn preserves_legacy_assistant_only_startup_reply() {
        let now = Utc::now();
        let messages = vec![message(
            "assistant",
            "boot reply",
            now,
            serde_json::json!({"synthetic_origin": "startup_hook"}),
        )];

        let turns = build_turns_from_messages(&messages);
        assert_eq!(turns.len(), 1);
        assert!(turns[0].hide_user_input);
        assert_eq!(turns[0].user_input, "");
        assert_eq!(turns[0].response.as_deref(), Some("boot reply"));
    }

    #[test]
    fn chat_thread_delete_response_preserves_existing_json_shape() {
        let thread_id = Uuid::nil();
        let response = chat_thread_delete_response(true, thread_id);
        let value = serde_json::to_value(response).expect("serialize response");

        assert_eq!(
            value,
            serde_json::json!({
                "deleted": true,
                "thread_id": thread_id.to_string(),
            })
        );
    }

    #[test]
    fn accepted_chat_responses_preserve_existing_shapes() {
        let message_id = Uuid::from_u128(1);
        assert_eq!(
            serde_json::to_value(send_message_response(message_id)).unwrap(),
            serde_json::json!({
                "message_id": message_id,
                "status": "accepted",
            })
        );
        assert_eq!(
            serde_json::to_value(thread_command_response(message_id)).unwrap(),
            serde_json::json!({
                "message_id": message_id,
                "status": "accepted",
            })
        );
    }

    #[test]
    fn chat_auth_responses_preserve_existing_shapes() {
        assert_eq!(
            serde_json::to_value(chat_auth_success_response("notion authenticated")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "notion authenticated",
            })
        );
        assert_eq!(
            serde_json::to_value(chat_auth_cancel_response()).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Auth cancelled",
            })
        );

        let required = chat_auth_required_response(ChatAuthRequiredResponseInput {
            auth_url: Some("https://auth.example".to_string()),
            setup_url: Some("https://setup.example".to_string()),
            auth_mode: "manual_token".to_string(),
            auth_status: "awaiting_token".to_string(),
            awaiting_token: true,
            instructions: Some("Paste a token".to_string()),
            shared_auth_provider: Some("github".to_string()),
            missing_scopes: vec!["repo".to_string()],
        });
        assert_eq!(
            serde_json::to_value(required).unwrap(),
            serde_json::json!({
                "success": false,
                "message": "Paste a token",
                "auth_url": "https://auth.example",
                "setup_url": "https://setup.example",
                "auth_mode": "manual_token",
                "auth_status": "awaiting_token",
                "awaiting_token": true,
                "instructions": "Paste a token",
                "shared_auth_provider": "github",
                "missing_scopes": ["repo"],
            })
        );
        assert_eq!(
            serde_json::to_value(chat_auth_required_response(ChatAuthRequiredResponseInput {
                auth_url: None,
                setup_url: None,
                auth_mode: "manual_token".to_string(),
                auth_status: "invalid".to_string(),
                awaiting_token: true,
                instructions: None,
                shared_auth_provider: None,
                missing_scopes: Vec::new(),
            }))
            .unwrap(),
            serde_json::json!({
                "success": false,
                "message": "Invalid token",
                "auth_mode": "manual_token",
                "auth_status": "invalid",
                "awaiting_token": true,
            })
        );
    }

    #[test]
    fn session_turn_projection_hides_user_input_and_preserves_tool_flags() {
        let turn = turn_info_from_session_turn(GatewaySessionTurnInfo {
            turn_number: 2,
            user_input: "hidden".to_string(),
            hide_user_input: true,
            response: Some("done".to_string()),
            state: "Completed".to_string(),
            started_at: "2026-06-02T10:00:00Z".parse::<DateTime<Utc>>().unwrap(),
            completed_at: None,
            tool_calls: vec![GatewaySessionToolCallInfo {
                name: "memory.search".to_string(),
                has_result: true,
                has_error: false,
            }],
        });

        assert_eq!(turn.user_input, "");
        assert!(turn.hide_user_input);
        assert_eq!(turn.tool_calls.len(), 1);
        assert!(turn.tool_calls[0].has_result);
    }

    #[test]
    fn history_and_thread_response_builders_preserve_timestamps() {
        let thread_id = Uuid::from_u128(1);
        let ts = "2026-06-02T10:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let history = history_response(thread_id, Vec::new(), true, Some(ts));
        assert_eq!(
            history.oldest_timestamp.as_deref(),
            Some("2026-06-02T10:00:00+00:00")
        );

        let info = thread_info(ThreadInfoInput {
            id: thread_id,
            state: "Idle".to_string(),
            turn_count: 0,
            created_at: ts,
            updated_at: ts,
            title: Some("Chat".to_string()),
            thread_type: Some("assistant".to_string()),
        });
        let response = thread_list_response(Some(info), Vec::new(), Some(thread_id));
        assert_eq!(response.active_thread, Some(thread_id));
        assert_eq!(
            response.assistant_thread.as_ref().unwrap().created_at,
            "2026-06-02T10:00:00+00:00"
        );
    }

    #[test]
    fn thread_summary_projection_splits_assistant_and_synthesizes_missing_assistant() {
        let assistant_id = Uuid::from_u128(1);
        let side_thread_id = Uuid::from_u128(2);
        let active_thread = Some(side_thread_id);
        let ts = "2026-06-02T10:00:00Z".parse::<DateTime<Utc>>().unwrap();

        let response = thread_list_response_from_summaries(
            assistant_id,
            vec![
                GatewayThreadSummaryInput {
                    id: assistant_id,
                    message_count: 5,
                    started_at: ts,
                    last_activity: ts + chrono::TimeDelta::seconds(1),
                    title: Some("Assistant".to_string()),
                    thread_type: Some("assistant".to_string()),
                },
                GatewayThreadSummaryInput {
                    id: side_thread_id,
                    message_count: -2,
                    started_at: ts + chrono::TimeDelta::seconds(2),
                    last_activity: ts + chrono::TimeDelta::seconds(3),
                    title: Some("Side".to_string()),
                    thread_type: Some("thread".to_string()),
                },
            ],
            active_thread,
            ts + chrono::TimeDelta::seconds(4),
            ts + chrono::TimeDelta::seconds(5),
        );

        assert_eq!(response.active_thread, active_thread);
        assert_eq!(response.assistant_thread.as_ref().unwrap().id, assistant_id);
        assert_eq!(response.assistant_thread.as_ref().unwrap().turn_count, 2);
        assert_eq!(response.threads.len(), 1);
        assert_eq!(response.threads[0].id, side_thread_id);
        assert_eq!(response.threads[0].turn_count, 0);

        let synthesized = thread_list_response_from_summaries(
            assistant_id,
            vec![GatewayThreadSummaryInput {
                id: side_thread_id,
                message_count: 2,
                started_at: ts,
                last_activity: ts,
                title: None,
                thread_type: Some("thread".to_string()),
            }],
            None,
            ts + chrono::TimeDelta::seconds(4),
            ts + chrono::TimeDelta::seconds(5),
        );

        let assistant = synthesized.assistant_thread.as_ref().unwrap();
        assert_eq!(assistant.id, assistant_id);
        assert_eq!(assistant.turn_count, 0);
        assert_eq!(assistant.created_at, "2026-06-02T10:00:04+00:00");
        assert_eq!(assistant.updated_at, "2026-06-02T10:00:05+00:00");
        assert_eq!(assistant.thread_type.as_deref(), Some("assistant"));
    }

    #[test]
    fn thread_export_content_supports_markdown_and_json() {
        let message = GatewayThreadExportMessage {
            id: Uuid::from_u128(1),
            role: "user".to_string(),
            content: "hello".to_string(),
            actor_id: Some("actor".to_string()),
            actor_display_name: None,
            raw_sender_id: None,
            metadata: serde_json::json!({"source": "test"}),
            created_at: "2026-06-02T10:00:00Z".parse::<DateTime<Utc>>().unwrap(),
        };

        assert_eq!(
            thread_export_content("markdown", std::slice::from_ref(&message)).unwrap(),
            "## user\n\nhello"
        );

        let json = thread_export_content("json", &[message]).unwrap();
        assert!(json.contains("\"role\": \"user\""));
        assert!(json.contains("\"created_at\""));
    }

    #[test]
    fn chat_boundary_errors_preserve_existing_statuses_and_messages() {
        assert_eq!(
            chat_rate_limit_error(),
            (
                StatusCode::TOO_MANY_REQUESTS,
                CHAT_RATE_LIMIT_MESSAGE.to_string()
            )
        );
        assert_eq!(
            empty_chat_message_content_message(),
            EMPTY_CHAT_MESSAGE_CONTENT_MESSAGE
        );
        assert_eq!(
            chat_cancel_failed_message("offline"),
            "Cancel failed: offline"
        );
        assert_eq!(
            unknown_approval_action_error("maybe"),
            (StatusCode::BAD_REQUEST, "Unknown action: maybe".to_string())
        );
        assert_eq!(
            parse_approval_request_id("bad"),
            Err((
                StatusCode::BAD_REQUEST,
                INVALID_APPROVAL_REQUEST_ID_MESSAGE.to_string()
            ))
        );
        let valid_thread_id = Uuid::new_v4().to_string();
        assert_eq!(
            parse_chat_thread_uuid(&valid_thread_id)
                .unwrap()
                .to_string(),
            valid_thread_id
        );
        assert!(parse_chat_thread_uuid("bad").is_err());
        assert_eq!(
            parse_chat_thread_query_id("bad"),
            Err((
                StatusCode::BAD_REQUEST,
                INVALID_THREAD_QUERY_ID_MESSAGE.to_string()
            ))
        );
        assert_eq!(
            parse_chat_thread_path_id("bad"),
            Err((
                StatusCode::BAD_REQUEST,
                INVALID_THREAD_PATH_ID_MESSAGE.to_string()
            ))
        );
        assert_eq!(
            parse_chat_thread_delete_id("bad"),
            Err((
                StatusCode::BAD_REQUEST,
                INVALID_THREAD_DELETE_ID_MESSAGE.to_string()
            ))
        );
        assert_eq!(
            invalid_before_timestamp_message(),
            INVALID_BEFORE_TIMESTAMP_MESSAGE
        );
        assert_eq!(no_active_thread_message(), NO_ACTIVE_THREAD_MESSAGE);
        assert_eq!(thread_not_found_message(), THREAD_NOT_FOUND_MESSAGE);

        for (actual, expected) in [
            extension_manager_unavailable_error(),
            too_many_chat_connections_error(),
            session_manager_unavailable_error(),
            invalid_before_timestamp_error(),
            no_active_thread_error(),
            thread_not_found_error(),
            chat_store_unavailable_error(),
            chat_database_unavailable_error(),
            delete_assistant_thread_forbidden_error(),
        ]
        .into_iter()
        .zip([
            (
                StatusCode::SERVICE_UNAVAILABLE,
                EXTENSION_MANAGER_UNAVAILABLE_MESSAGE,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                TOO_MANY_CHAT_CONNECTIONS_MESSAGE,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                SESSION_MANAGER_UNAVAILABLE_MESSAGE,
            ),
            (StatusCode::BAD_REQUEST, INVALID_BEFORE_TIMESTAMP_MESSAGE),
            (StatusCode::NOT_FOUND, NO_ACTIVE_THREAD_MESSAGE),
            (StatusCode::NOT_FOUND, THREAD_NOT_FOUND_MESSAGE),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                CHAT_STORE_UNAVAILABLE_MESSAGE,
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                CHAT_DATABASE_UNAVAILABLE_MESSAGE,
            ),
            (
                StatusCode::FORBIDDEN,
                DELETE_ASSISTANT_THREAD_FORBIDDEN_MESSAGE,
            ),
        ]) {
            assert_eq!(actual, (expected.0, expected.1.to_string()));
        }
    }

    #[test]
    fn history_query_normalization_defaults_limit_and_parses_before_cursor() {
        let query = HistoryQuery {
            thread_id: None,
            limit: None,
            before: Some("2026-06-02T10:00:00Z".to_string()),
            user_id: None,
            actor_id: None,
        };

        let options = normalize_chat_history_query(&query).unwrap();

        assert_eq!(options.limit, 50);
        assert_eq!(
            options.before_cursor.unwrap().to_rfc3339(),
            "2026-06-02T10:00:00+00:00"
        );
    }

    #[test]
    fn history_query_normalization_preserves_limit_and_rejects_bad_cursor() {
        let query = HistoryQuery {
            thread_id: None,
            limit: Some(25),
            before: None,
            user_id: None,
            actor_id: None,
        };
        let options = normalize_chat_history_query(&query).unwrap();
        assert_eq!(options.limit, 25);
        assert!(options.before_cursor.is_none());

        let bad_query = HistoryQuery {
            before: Some("not-a-timestamp".to_string()),
            ..query
        };
        assert_eq!(
            normalize_chat_history_query(&bad_query),
            Err(invalid_before_timestamp_error())
        );
    }

    #[test]
    fn history_thread_resolution_prefers_query_thread_and_falls_back_to_active_thread() {
        let query_thread = Uuid::from_u128(1);
        let active_thread = Uuid::from_u128(2);

        assert_eq!(
            resolve_chat_history_thread_id(Some(&query_thread.to_string()), Some(active_thread)),
            Ok(query_thread)
        );
        assert_eq!(
            resolve_chat_history_thread_id(None, Some(active_thread)),
            Ok(active_thread)
        );
        assert_eq!(
            resolve_chat_history_thread_id(None, None),
            Err(no_active_thread_error())
        );
        assert_eq!(
            resolve_chat_history_thread_id(Some("bad"), Some(active_thread)),
            Err((
                StatusCode::BAD_REQUEST,
                INVALID_THREAD_QUERY_ID_MESSAGE.to_string()
            ))
        );
    }
}
