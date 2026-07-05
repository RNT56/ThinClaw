//! Channel pairing request/approval DTOs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct PairingListResponse {
    pub channel: String,
    pub requests: Vec<PairingRequestInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approved: Vec<PairingApprovedInfo>,
}

#[derive(Debug, Serialize)]
pub struct PairingRequestInfo {
    pub code: String,
    pub sender_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PairingApprovedInfo {
    pub sender_id: String,
}

#[derive(Debug, Deserialize)]
pub struct PairingApproveRequest {
    pub code: String,
}
