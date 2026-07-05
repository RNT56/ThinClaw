//! WebSocket client/server message envelopes for `/api/chat/ws`.

use serde::{Deserialize, Serialize};

use super::common::ModelInfo;
use super::sse::SseEvent;

/// Message sent by a WebSocket client to the server.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WsClientMessage {
    /// Send a chat message to the agent.
    #[serde(rename = "message")]
    Message {
        content: String,
        thread_id: Option<String>,
    },
    /// Approve or deny a pending tool execution.
    #[serde(rename = "approval")]
    Approval {
        request_id: String,
        /// "approve", "always", or "deny"
        action: String,
        /// Thread that owns the pending approval.
        thread_id: Option<String>,
    },
    /// Submit an auth token for an extension (bypasses message pipeline).
    #[serde(rename = "auth_token")]
    AuthToken {
        extension_name: String,
        token: String,
    },
    /// Cancel an in-progress auth flow.
    #[serde(rename = "auth_cancel")]
    AuthCancel { extension_name: String },
    /// Client heartbeat ping.
    #[serde(rename = "ping")]
    Ping,
    /// Protocol version handshake (sent on connect).
    #[serde(rename = "version")]
    Version {
        /// Client protocol version (semver, e.g. "1.0.0").
        protocol_version: String,
        /// Client name/identifier.
        client_name: Option<String>,
    },
    /// Set a configuration value on the orchestrator.
    #[serde(rename = "config_set")]
    ConfigSet {
        /// Dot-separated config key (e.g. "agent.model", "agent.temperature").
        key: String,
        /// New value (JSON).
        value: serde_json::Value,
    },
    /// Set a secret on the orchestrator's keychain.
    #[serde(rename = "secret_set")]
    SecretSet {
        /// Secret key name (e.g. "OPENAI_API_KEY").
        key: String,
        /// The secret value.
        value: String,
    },
    /// Request the list of available models.
    #[serde(rename = "model_list")]
    ModelList,
}

/// Message sent by the server to a WebSocket client.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WsServerMessage {
    /// An SSE-style event forwarded over WebSocket.
    #[serde(rename = "event")]
    Event {
        /// The event sub-type (response, thinking, tool_started, etc.)
        event_type: String,
        /// The event payload as a JSON value.
        data: serde_json::Value,
    },
    /// Server heartbeat pong.
    #[serde(rename = "pong")]
    Pong,
    /// Error message.
    #[serde(rename = "error")]
    Error { message: String },
    /// Protocol version info (response to client's Version message).
    #[serde(rename = "version_info")]
    VersionInfo {
        /// Server protocol version.
        protocol_version: String,
        /// Server name.
        server_name: String,
        /// Server software version.
        server_version: String,
        /// Whether the client version is compatible.
        compatible: bool,
    },
    /// Result of a config_set operation.
    #[serde(rename = "config_result")]
    ConfigResult {
        /// The key that was set.
        key: String,
        /// Whether the operation succeeded.
        success: bool,
        /// Error message if failed.
        error: Option<String>,
    },
    /// Result of a secret_set operation.
    #[serde(rename = "secret_result")]
    SecretResult {
        /// The key that was set.
        key: String,
        /// Whether the operation succeeded.
        success: bool,
        /// Error message if failed.
        error: Option<String>,
    },
    /// List of available models.
    #[serde(rename = "model_list_result")]
    ModelListResult { models: Vec<ModelInfo> },
}

impl WsServerMessage {
    /// Create a WsServerMessage from an SseEvent.
    pub fn from_sse_event(event: &SseEvent) -> Self {
        let data = serde_json::to_value(event).unwrap_or(serde_json::Value::Null);
        WsServerMessage::Event {
            event_type: event.event_type().to_string(),
            data,
        }
    }
}
