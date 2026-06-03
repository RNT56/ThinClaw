//! Root-independent Nostr gateway response DTOs.

use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

pub const NOSTR_SECRETS_STORE_UNAVAILABLE_STATUS: StatusCode = StatusCode::SERVICE_UNAVAILABLE;
pub const INVALID_NOSTR_PRIVATE_KEY_STATUS: StatusCode = StatusCode::BAD_REQUEST;

pub fn nostr_secrets_store_unavailable_status() -> StatusCode {
    NOSTR_SECRETS_STORE_UNAVAILABLE_STATUS
}

pub fn invalid_nostr_private_key_status() -> StatusCode {
    INVALID_NOSTR_PRIVATE_KEY_STATUS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NostrSaveKeyResponse {
    pub status: String,
    pub message: String,
    pub public_key_hex: String,
    pub public_key_npub: String,
}

pub fn nostr_save_key_response(
    public_key_hex: impl Into<String>,
    public_key_npub: impl Into<String>,
) -> NostrSaveKeyResponse {
    NostrSaveKeyResponse {
        status: "ok".to_string(),
        message: "Nostr private key saved".to_string(),
        public_key_hex: public_key_hex.into(),
        public_key_npub: public_key_npub.into(),
    }
}

pub fn nostr_save_key_partial_failure_response(
    reconcile_error: impl std::fmt::Display,
    public_key_hex: impl Into<String>,
    public_key_npub: impl Into<String>,
) -> NostrSaveKeyResponse {
    NostrSaveKeyResponse {
        status: "partial_failure".to_string(),
        message: format!(
            "The Nostr private key was saved, but the live Nostr runtime could not be reconciled: {}",
            reconcile_error
        ),
        public_key_hex: public_key_hex.into(),
        public_key_npub: public_key_npub.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NostrDeleteKeyResponse {
    pub status: String,
    pub message: String,
}

pub fn nostr_delete_key_response() -> NostrDeleteKeyResponse {
    NostrDeleteKeyResponse {
        status: "ok".to_string(),
        message: "Nostr private key removed".to_string(),
    }
}

pub fn nostr_delete_key_partial_failure_response(
    reconcile_error: impl std::fmt::Display,
) -> NostrDeleteKeyResponse {
    NostrDeleteKeyResponse {
        status: "partial_failure".to_string(),
        message: format!(
            "The Nostr private key was removed, but the live Nostr runtime could not be reconciled: {}",
            reconcile_error
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nostr_save_key_response_preserves_success_json_shape() {
        let value = serde_json::to_value(nostr_save_key_response("hex", "npub")).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "status": "ok",
                "message": "Nostr private key saved",
                "public_key_hex": "hex",
                "public_key_npub": "npub",
            })
        );
    }

    #[test]
    fn nostr_save_key_response_preserves_partial_failure_json_shape() {
        let value = serde_json::to_value(nostr_save_key_partial_failure_response(
            "offline", "hex", "npub",
        ))
        .unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "status": "partial_failure",
                "message": "The Nostr private key was saved, but the live Nostr runtime could not be reconciled: offline",
                "public_key_hex": "hex",
                "public_key_npub": "npub",
            })
        );
    }

    #[test]
    fn nostr_delete_key_response_preserves_success_json_shape() {
        let value = serde_json::to_value(nostr_delete_key_response()).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "status": "ok",
                "message": "Nostr private key removed",
            })
        );
    }

    #[test]
    fn nostr_delete_key_response_preserves_partial_failure_json_shape() {
        let value =
            serde_json::to_value(nostr_delete_key_partial_failure_response("offline")).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "status": "partial_failure",
                "message": "The Nostr private key was removed, but the live Nostr runtime could not be reconciled: offline",
            })
        );
    }

    #[test]
    fn nostr_status_helpers_preserve_existing_statuses() {
        assert_eq!(
            nostr_secrets_store_unavailable_status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(invalid_nostr_private_key_status(), StatusCode::BAD_REQUEST);
    }
}
