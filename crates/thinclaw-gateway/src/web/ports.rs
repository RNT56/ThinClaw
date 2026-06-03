//! Narrow app-facing ports for gateway shell code.

use async_trait::async_trait;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thinclaw_channels_core::IncomingMessage;
use thinclaw_llm_core::ChatMessage;
use thinclaw_types::media::MediaContent;
use uuid::Uuid;

use crate::web::identity::GatewayRequestIdentity;

#[async_trait]
pub trait AgentSubmissionPort: Send + Sync {
    async fn submit_agent_message(&self, message: IncomingMessage) -> Result<(), String>;
}

#[async_trait]
pub trait AuthSessionPort: Send + Sync {
    async fn current_identity(
        &self,
        token: Option<&str>,
    ) -> Result<Option<GatewayRequestIdentity>, String>;
}

#[async_trait]
pub trait IdentityLookupPort: Send + Sync {
    async fn infer_primary_user_id_for_channel(
        &self,
        channel: &str,
    ) -> Result<Option<String>, String>;
}

#[async_trait]
pub trait MediaPort: Send + Sync {
    async fn attach_media(&self, message_id: &str, media: Vec<MediaContent>) -> Result<(), String>;
}

#[async_trait]
pub trait RouteStatePort: Send + Sync {
    async fn mark_conversation_updated(
        &self,
        thread_id: &str,
        reason: &str,
        channel: Option<&str>,
    ) -> Result<(), String>;

    async fn mark_conversation_deleted(
        &self,
        identity: &GatewayRequestIdentity,
        thread_id: &str,
    ) -> Result<(), String>;
}

/// Portable gateway adapter error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GatewayPortError {
    #[error("not found: {resource}")]
    NotFound { resource: String },
    #[error("forbidden: {reason}")]
    Forbidden { reason: String },
    #[error("invalid request: {reason}")]
    InvalidRequest { reason: String },
    #[error("unavailable: {service}")]
    Unavailable { service: String },
    #[error("operation failed: {reason}")]
    OperationFailed { reason: String },
}

pub fn gateway_port_error(service: &str, error: impl std::fmt::Display) -> GatewayPortError {
    GatewayPortError::OperationFailed {
        reason: format!("{service}: {error}"),
    }
}

pub fn gateway_unavailable(service: &str) -> GatewayPortError {
    GatewayPortError::Unavailable {
        service: service.to_string(),
    }
}

/// Root-independent conversation reference used by gateway handlers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConversationRef {
    pub principal_id: String,
    pub actor_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

impl GatewayConversationRef {
    pub fn new(principal_id: impl Into<String>, actor_id: impl Into<String>) -> Self {
        Self {
            principal_id: principal_id.into(),
            actor_id: actor_id.into(),
            thread_id: None,
            external_thread_id: None,
            channel: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayConversationSummary {
    pub id: Uuid,
    pub title: Option<String>,
    pub channel: String,
    pub thread_id: Option<String>,
    pub preview: Option<String>,
    pub turn_count: usize,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayConversationMessage {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConversationQuery {
    pub identity: GatewayConversationRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<DateTime<Utc>>,
    #[serde(default = "default_history_limit")]
    pub limit: i64,
}

#[async_trait]
pub trait ConversationPort: Send + Sync {
    async fn get_or_create_conversation(
        &self,
        identity: GatewayConversationRef,
    ) -> Result<GatewayConversationSummary, GatewayPortError>;

    async fn conversation_belongs_to_actor(
        &self,
        conversation_id: Uuid,
        principal_id: &str,
        actor_id: &str,
    ) -> Result<bool, GatewayPortError>;

    async fn list_conversations(
        &self,
        identity: GatewayConversationRef,
        include_group_history: bool,
        limit: i64,
    ) -> Result<Vec<GatewayConversationSummary>, GatewayPortError>;

    async fn list_messages(
        &self,
        query: GatewayConversationQuery,
    ) -> Result<(Vec<GatewayConversationMessage>, bool), GatewayPortError>;

    async fn delete_conversation(
        &self,
        identity: GatewayConversationRef,
        conversation_id: Uuid,
    ) -> Result<(), GatewayPortError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewaySettingsSnapshot {
    pub user_id: String,
    pub values: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewaySettingsPatch {
    pub user_id: String,
    pub values: serde_json::Value,
    #[serde(default)]
    pub changed_keys: Vec<String>,
}

#[async_trait]
pub trait SettingsPort: Send + Sync {
    async fn load_settings(
        &self,
        user_id: &str,
    ) -> Result<GatewaySettingsSnapshot, GatewayPortError>;

    async fn save_settings(
        &self,
        patch: GatewaySettingsPatch,
    ) -> Result<GatewaySettingsSnapshot, GatewayPortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GatewayJobStatus {
    #[default]
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Stuck,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayJobSummary {
    pub id: Uuid,
    pub title: String,
    pub status: GatewayJobStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait JobPort: Send + Sync {
    async fn list_jobs(
        &self,
        identity: GatewayConversationRef,
        limit: i64,
    ) -> Result<Vec<GatewayJobSummary>, GatewayPortError>;

    async fn load_job(
        &self,
        identity: GatewayConversationRef,
        job_id: Uuid,
    ) -> Result<Option<GatewayJobSummary>, GatewayPortError>;

    async fn cancel_job(
        &self,
        identity: GatewayConversationRef,
        job_id: Uuid,
    ) -> Result<GatewayJobSummary, GatewayPortError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayExtensionAuthStatus {
    pub extension_name: String,
    pub auth_status: String,
    pub auth_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_url: Option<String>,
    #[serde(default)]
    pub missing_scopes: Vec<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

pub fn with_activation_metadata(
    mut status: GatewayExtensionAuthStatus,
    activated: bool,
    message: impl Into<String>,
    tools_loaded: Vec<String>,
) -> GatewayExtensionAuthStatus {
    let mut metadata = status.metadata.as_object().cloned().unwrap_or_default();
    metadata.insert("activated".to_string(), serde_json::Value::Bool(activated));
    metadata.insert(
        "activation_message".to_string(),
        serde_json::Value::String(message.into()),
    );
    metadata.insert("tools_loaded".to_string(), serde_json::json!(tools_loaded));
    status.metadata = serde_json::Value::Object(metadata);
    status
}

#[async_trait]
pub trait ExtensionAuthPort: Send + Sync {
    async fn auth_status(
        &self,
        identity: GatewayConversationRef,
        extension_name: &str,
    ) -> Result<GatewayExtensionAuthStatus, GatewayPortError>;

    async fn submit_auth_token(
        &self,
        identity: GatewayConversationRef,
        extension_name: &str,
        token: String,
    ) -> Result<GatewayExtensionAuthStatus, GatewayPortError>;

    async fn cancel_auth(
        &self,
        identity: GatewayConversationRef,
        extension_name: &str,
    ) -> Result<(), GatewayPortError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayLlmMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

pub fn gateway_message_to_chat_message(message: GatewayLlmMessage) -> ChatMessage {
    let GatewayLlmMessage {
        role,
        content,
        name,
    } = message;
    let original_name = name.clone();
    let mut converted = match role.as_str() {
        "system" => ChatMessage::system(content),
        "assistant" => ChatMessage::assistant(content),
        "tool" => {
            let name = name.unwrap_or_default();
            ChatMessage::tool_result(name.clone(), name, content)
        }
        _ => ChatMessage::user(content),
    };
    if converted.name.is_none() {
        converted.name = original_name;
    }
    converted
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayLlmCompletionRequest {
    pub identity: GatewayConversationRef,
    pub messages: Vec<GatewayLlmMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayLlmCompletionResponse {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayModelSummary {
    pub id: String,
    pub provider: Option<String>,
    pub is_primary: bool,
}

#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn complete(
        &self,
        request: GatewayLlmCompletionRequest,
    ) -> Result<GatewayLlmCompletionResponse, GatewayPortError>;

    async fn list_models(&self) -> Result<Vec<GatewayModelSummary>, GatewayPortError>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayRuntimeStatusSnapshot {
    pub status: String,
    pub version: Option<String>,
    #[serde(default)]
    pub channels: Vec<String>,
    #[serde(default)]
    pub capabilities: serde_json::Value,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
}

#[async_trait]
pub trait RuntimeStatusPort: Send + Sync {
    async fn runtime_status(&self) -> Result<GatewayRuntimeStatusSnapshot, GatewayPortError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayVisibilitySubject {
    pub principal_id: String,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayVisibilityTarget {
    pub conversation_id: Option<Uuid>,
    pub channel: Option<String>,
    pub principal_id: Option<String>,
    pub actor_id: Option<String>,
}

#[async_trait]
pub trait VisibilityPort: Send + Sync {
    async fn can_view_conversation(
        &self,
        subject: GatewayVisibilitySubject,
        target: GatewayVisibilityTarget,
    ) -> Result<bool, GatewayPortError>;
}

/// Normalize a browser Origin/Referer header into the origin string stored on
/// gateway submissions.
pub fn request_origin_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        return Some(origin.trim_end_matches('/').to_string());
    }

    headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<axum::http::Uri>().ok())
        .and_then(|uri| {
            let scheme = uri.scheme_str()?;
            let authority = uri.authority()?;
            Some(format!("{scheme}://{authority}"))
        })
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WebSocketOriginError {
    #[error("WebSocket origin is invalid")]
    Invalid,
    #[error("WebSocket origin must use http or https")]
    UnsupportedScheme,
}

impl WebSocketOriginError {
    pub fn status_code(&self) -> StatusCode {
        StatusCode::FORBIDDEN
    }
}

pub fn validate_websocket_origin(headers: &HeaderMap) -> Result<(), WebSocketOriginError> {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };

    let parsed = origin
        .parse::<Uri>()
        .map_err(|_| WebSocketOriginError::Invalid)?;
    let Some(scheme) = parsed.scheme_str() else {
        return Err(WebSocketOriginError::Invalid);
    };
    if parsed.authority().is_none() {
        return Err(WebSocketOriginError::Invalid);
    }
    if !matches!(scheme, "http" | "https") {
        return Err(WebSocketOriginError::UnsupportedScheme);
    }

    Ok(())
}

/// Whether a persisted user message should be hidden from the main web chat.
pub fn message_hidden_from_main_chat(metadata: &serde_json::Value) -> bool {
    metadata
        .get("hide_user_input_from_webui_chat")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            metadata
                .get("hide_from_webui_chat")
                .and_then(|value| value.as_bool())
        })
        .unwrap_or(false)
}

/// Whether a persisted message came from the startup hook projection.
pub fn message_is_startup_hook(metadata: &serde_json::Value) -> bool {
    metadata
        .get("synthetic_origin")
        .and_then(|value| value.as_str())
        == Some("startup_hook")
}

fn default_history_limit() -> i64 {
    50
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_origin_prefers_origin_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://example.test/".parse().unwrap());
        headers.insert(header::REFERER, "https://other.test/page".parse().unwrap());

        assert_eq!(
            request_origin_from_headers(&headers).as_deref(),
            Some("https://example.test")
        );
    }

    #[test]
    fn request_origin_falls_back_to_referer_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::REFERER,
            "https://example.test:8443/path?q=1".parse().unwrap(),
        );

        assert_eq!(
            request_origin_from_headers(&headers).as_deref(),
            Some("https://example.test:8443")
        );
    }

    #[test]
    fn websocket_origin_accepts_http_origins_and_missing_header() {
        assert_eq!(validate_websocket_origin(&HeaderMap::new()), Ok(()));

        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://example.test".parse().unwrap());
        assert_eq!(validate_websocket_origin(&headers), Ok(()));
    }

    #[test]
    fn websocket_origin_rejects_relative_or_non_http_origins() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "null".parse().unwrap());
        assert_eq!(
            validate_websocket_origin(&headers),
            Err(WebSocketOriginError::Invalid)
        );

        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "ftp://example.test".parse().unwrap());
        assert_eq!(
            validate_websocket_origin(&headers),
            Err(WebSocketOriginError::UnsupportedScheme)
        );
    }

    #[test]
    fn hidden_message_supports_legacy_and_current_keys() {
        assert!(message_hidden_from_main_chat(&serde_json::json!({
            "hide_user_input_from_webui_chat": true
        })));
        assert!(message_hidden_from_main_chat(&serde_json::json!({
            "hide_from_webui_chat": true
        })));
        assert!(!message_hidden_from_main_chat(&serde_json::json!({})));
    }

    #[test]
    fn gateway_port_error_helpers_create_port_errors() {
        assert_eq!(
            gateway_port_error("database", "closed"),
            GatewayPortError::OperationFailed {
                reason: "database: closed".to_string()
            }
        );
        assert_eq!(
            gateway_unavailable("llm"),
            GatewayPortError::Unavailable {
                service: "llm".to_string()
            }
        );
    }

    #[test]
    fn gateway_message_to_chat_message_preserves_roles_and_names() {
        let system = gateway_message_to_chat_message(GatewayLlmMessage {
            role: "system".to_string(),
            content: "rules".to_string(),
            name: Some("sys".to_string()),
        });
        assert_eq!(system.role, thinclaw_llm_core::Role::System);
        assert_eq!(system.name.as_deref(), Some("sys"));

        let tool = gateway_message_to_chat_message(GatewayLlmMessage {
            role: "tool".to_string(),
            content: "result".to_string(),
            name: Some("shell".to_string()),
        });
        assert_eq!(tool.role, thinclaw_llm_core::Role::Tool);
        assert_eq!(tool.name.as_deref(), Some("shell"));
        assert_eq!(tool.tool_call_id.as_deref(), Some("shell"));
    }

    #[test]
    fn activation_metadata_merges_status_metadata() {
        let status = GatewayExtensionAuthStatus {
            extension_name: "demo".to_string(),
            auth_status: "authenticated".to_string(),
            auth_mode: "api_key".to_string(),
            auth_url: None,
            missing_scopes: Vec::new(),
            metadata: serde_json::json!({"kind": "wasm"}),
        };

        let status =
            with_activation_metadata(status, true, "activated", vec!["tool_a".to_string()]);

        assert_eq!(status.metadata["kind"], "wasm");
        assert_eq!(status.metadata["activated"], true);
        assert_eq!(status.metadata["activation_message"], "activated");
        assert_eq!(status.metadata["tools_loaded"][0], "tool_a");
    }

    #[test]
    fn gateway_port_traits_are_object_safe() {
        fn assert_object_safe<T: ?Sized + Send + Sync>() {}

        assert_object_safe::<dyn ConversationPort>();
        assert_object_safe::<dyn SettingsPort>();
        assert_object_safe::<dyn JobPort>();
        assert_object_safe::<dyn ExtensionAuthPort>();
        assert_object_safe::<dyn LlmPort>();
        assert_object_safe::<dyn RuntimeStatusPort>();
        assert_object_safe::<dyn VisibilityPort>();
    }
}
