//! WebSocket frame models for OpenClawEngine Gateway protocol
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
