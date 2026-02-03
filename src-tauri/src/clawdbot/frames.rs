//! WebSocket frame models for Moltbot Gateway protocol
//!
//! These types model the outer framing robustly, parsing most payloads
//! as serde_json::Value for resilience against upstream evolution.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// WebSocket frame envelope - all messages conform to one of these types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsFrame {
    /// Request from client to gateway
    #[serde(rename = "req")]
    Req {
        id: String,
        method: String,
        #[serde(default)]
        params: Value,
    },

    /// Response from gateway to client
    #[serde(rename = "res")]
    Res {
        id: String,
        ok: bool,
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        error: Option<WsError>,
    },

    /// Event pushed from gateway
    #[serde(rename = "event")]
    Event {
        event: String,
        #[serde(default)]
        payload: Value,
        #[serde(default)]
        seq: Option<u64>,
        #[serde(default, rename = "stateVersion")]
        state_version: Option<Value>,
    },
}

/// Error structure returned in failed responses
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Value,
}

impl Default for WsError {
    fn default() -> Self {
        Self {
            code: "UNKNOWN".into(),
            message: "Unknown error".into(),
            details: Value::Null,
        }
    }
}

// ============================================================================
// Request Builders
// ============================================================================

/// Build a connect request for operator role
pub fn build_connect_req(
    id: String,
    token: &str,
    device_id: &str,
    nonce: Option<&str>,
    public_key: Option<&str>,
    signature: Option<&str>,
    signed_at: i64,
) -> WsFrame {
    WsFrame::Req {
        id,
        method: "connect".to_string(),
        params: serde_json::json!({
            "minProtocol": 3,
            "maxProtocol": 3,
            "client": {
                "id": "cli",
                "version": env!("CARGO_PKG_VERSION"),
                "platform": std::env::consts::OS,
                "mode": "cli"
            },
            "role": "operator",
            "scopes": ["operator.read", "operator.write", "operator.approvals", "operator.admin"],
            "auth": { "token": token },
            "locale": "en-US",
            "device": {
                "id": device_id,
                "nonce": nonce,
                "publicKey": public_key.unwrap_or("local-scrappy"),
                "signature": signature.unwrap_or("local-scrappy"),
                "signedAt": signed_at
            }
        }),
    }
}

/// Build a chat.history request
#[allow(dead_code)]
pub fn build_chat_history_req(
    id: String,
    session_key: &str,
    limit: u32,
    _before: Option<&str>, // Note: moltbot's chat.history API doesn't support 'before' pagination
) -> WsFrame {
    // Note: moltbot's ChatHistoryParamsSchema only accepts sessionKey and limit
    // It rejects additional properties like 'before' or 'includeToolMessages'
    WsFrame::Req {
        id,
        method: "chat.history".to_string(),
        params: serde_json::json!({
            "sessionKey": session_key,
            "limit": limit
        }),
    }
}

/// Build a chat.subscribe request
#[allow(dead_code)]
pub fn build_chat_subscribe_req(id: String, session_key: &str) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.subscribe".to_string(),
        params: serde_json::json!({ "sessionKey": session_key }),
    }
}

/// Build a chat.send request with idempotency
#[allow(dead_code)]
pub fn build_chat_send_req(
    id: String,
    session_key: &str,
    idempotency_key: &str,
    text: &str,
    deliver: bool,
) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.send".to_string(),
        params: serde_json::json!({
            "sessionKey": session_key,
            "idempotencyKey": idempotency_key,
            "message": { "role": "user", "text": text, "attachments": [] },
            "options": { "deliver": deliver }
        }),
    }
}

/// Build a chat.abort request
#[allow(dead_code)]
pub fn build_chat_abort_req(id: String, session_key: &str, run_id: Option<&str>) -> WsFrame {
    WsFrame::Req {
        id,
        method: "chat.abort".to_string(),
        params: serde_json::json!({
            "sessionKey": session_key,
            "runId": run_id
        }),
    }
}

/// Build a sessions.list request
#[allow(dead_code)]
pub fn build_sessions_list_req(id: String) -> WsFrame {
    WsFrame::Req {
        id,
        method: "sessions.list".to_string(),
        params: serde_json::json!({}),
    }
}

/// Build a status request
#[allow(dead_code)]
pub fn build_status_req(id: String) -> WsFrame {
    WsFrame::Req {
        id,
        method: "status".to_string(),
        params: serde_json::json!({}),
    }
}

/// Generate an idempotency key for chat.send
#[allow(dead_code)]
pub fn make_idempotency_key(session_key: &str, client_msg_id: &str) -> String {
    format!(
        "scrappy:{}:{}:{}",
        session_key,
        client_msg_id,
        uuid::Uuid::new_v4()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_req_frame() {
        let json = r#"{"type":"req","id":"123","method":"status","params":{}}"#;
        let frame: WsFrame = serde_json::from_str(json).unwrap();
        match frame {
            WsFrame::Req { id, method, .. } => {
                assert_eq!(id, "123");
                assert_eq!(method, "status");
            }
            _ => panic!("Expected Req frame"),
        }
    }

    #[test]
    fn test_parse_event_frame() {
        let json = r#"{"type":"event","event":"chat","payload":{"delta":"Hello"}}"#;
        let frame: WsFrame = serde_json::from_str(json).unwrap();
        match frame {
            WsFrame::Event { event, payload, .. } => {
                assert_eq!(event, "chat");
                assert_eq!(payload.get("delta").unwrap().as_str().unwrap(), "Hello");
            }
            _ => panic!("Expected Event frame"),
        }
    }

    #[test]
    fn test_build_connect_req() {
        let frame = build_connect_req(
            "req-1".into(),
            "test-token",
            "device-1",
            Some("nonce-123"),
            None,
            None,
            0,
        );
        match frame {
            WsFrame::Req { method, params, .. } => {
                assert_eq!(method, "connect");
                assert_eq!(params["auth"]["token"], "test-token");
            }
            _ => panic!("Expected Req frame"),
        }
    }
}
