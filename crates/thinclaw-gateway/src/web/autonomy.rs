//! Root-independent desktop autonomy gateway response helpers.

use axum::http::StatusCode;

use crate::web::types::AutonomyPauseResponse;

pub const DESKTOP_AUTONOMY_MANAGER_INACTIVE_MESSAGE: &str =
    "desktop autonomy manager is not active";
pub const NO_PREVIOUS_PROMOTED_BUILD_MESSAGE: &str = "no previous promoted build";

pub fn desktop_autonomy_manager_inactive_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        DESKTOP_AUTONOMY_MANAGER_INACTIVE_MESSAGE.to_string(),
    )
}

pub fn autonomy_bad_request_error(error: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, error.into())
}

pub fn autonomy_internal_error(error: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.into())
}

pub fn autonomy_rollback_error(error: impl Into<String>) -> (StatusCode, String) {
    let error = error.into();
    let status = if error.contains(NO_PREVIOUS_PROMOTED_BUILD_MESSAGE) {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, error)
}

pub fn autonomy_pause_response(paused: bool) -> AutonomyPauseResponse {
    AutonomyPauseResponse { paused }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autonomy_pause_response_preserves_shape() {
        assert_eq!(
            serde_json::to_value(autonomy_pause_response(true)).unwrap(),
            serde_json::json!({ "paused": true })
        );
    }

    #[test]
    fn autonomy_errors_preserve_existing_statuses() {
        assert_eq!(
            desktop_autonomy_manager_inactive_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                DESKTOP_AUTONOMY_MANAGER_INACTIVE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            autonomy_rollback_error("no previous promoted build exists"),
            (
                StatusCode::BAD_REQUEST,
                "no previous promoted build exists".to_string()
            )
        );
        assert_eq!(
            autonomy_rollback_error("rollback failed"),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "rollback failed".to_string()
            )
        );
        assert_eq!(
            autonomy_bad_request_error("bad request"),
            (StatusCode::BAD_REQUEST, "bad request".to_string())
        );
        assert_eq!(
            autonomy_internal_error("failed"),
            (StatusCode::INTERNAL_SERVER_ERROR, "failed".to_string())
        );
    }
}
