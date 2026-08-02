//! Root-independent pairing gateway response policies.

use crate::web::types::{
    ActionResponse, PairingApprovedInfo, PairingListResponse, PairingRequestInfo,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingRequestInfoInput {
    pub request_id: String,
    pub sender_id: String,
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
}

pub fn pairing_request_info(input: PairingRequestInfoInput) -> PairingRequestInfo {
    PairingRequestInfo {
        request_id: input.request_id,
        sender_id: input.sender_id,
        meta: input.meta,
        created_at: input.created_at,
    }
}

pub fn pairing_approved_info(sender_id: impl Into<String>) -> PairingApprovedInfo {
    PairingApprovedInfo {
        sender_id: sender_id.into(),
    }
}

pub fn pairing_list_response(
    channel: impl Into<String>,
    requests: Vec<PairingRequestInfo>,
    approved: Vec<PairingApprovedInfo>,
) -> PairingListResponse {
    PairingListResponse {
        channel: channel.into(),
        requests,
        approved,
    }
}

pub fn pairing_approve_response(sender_id: impl AsRef<str>) -> ActionResponse {
    ActionResponse::ok(format!(
        "Pairing approved for sender '{}'",
        sender_id.as_ref()
    ))
}

pub fn pairing_invalid_request_response() -> ActionResponse {
    ActionResponse::fail("Invalid, expired, or already-consumed pairing request ID")
}

pub fn pairing_error_response(message: impl Into<String>) -> ActionResponse {
    ActionResponse::fail(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_list_response_preserves_shape() {
        let response = pairing_list_response(
            "telegram",
            vec![pairing_request_info(PairingRequestInfoInput {
                request_id: "request-123".to_string(),
                sender_id: "alice".to_string(),
                meta: Some(serde_json::json!({"chat": 1})),
                created_at: "2026-06-02T10:00:00Z".to_string(),
            })],
            vec![pairing_approved_info("bob")],
        );

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            serde_json::json!({
                "channel": "telegram",
                "requests": [{
                    "request_id": "request-123",
                    "sender_id": "alice",
                    "meta": {"chat": 1},
                    "created_at": "2026-06-02T10:00:00Z",
                }],
                "approved": [{"sender_id": "bob"}],
            })
        );
    }

    #[test]
    fn pairing_action_responses_preserve_shape() {
        assert_eq!(
            serde_json::to_value(pairing_approve_response("alice")).unwrap(),
            serde_json::json!({
                "success": true,
                "message": "Pairing approved for sender 'alice'",
            })
        );
        assert_eq!(
            serde_json::to_value(pairing_invalid_request_response()).unwrap(),
            serde_json::json!({
                "success": false,
                "message": "Invalid, expired, or already-consumed pairing request ID",
            })
        );
        assert_eq!(
            serde_json::to_value(pairing_error_response("disk unavailable")).unwrap(),
            serde_json::json!({
                "success": false,
                "message": "disk unavailable",
            })
        );
    }
}
