//! Root-independent log gateway response policies.

use axum::http::StatusCode;

use crate::web::log_layer::LogEntry;
use crate::web::types::{LogLevelResponse, LogsRecentResponse};

pub const LOG_BROADCASTER_UNAVAILABLE_MESSAGE: &str = "Log broadcaster not available";
pub const LOG_LEVEL_CONTROL_UNAVAILABLE_MESSAGE: &str = "Log level control not available";

pub fn log_broadcaster_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        LOG_BROADCASTER_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn log_level_control_unavailable_error() -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        LOG_LEVEL_CONTROL_UNAVAILABLE_MESSAGE.to_string(),
    )
}

pub fn invalid_log_level_error(error: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, error.into())
}

pub fn logs_recent_response(logs: Vec<LogEntry>) -> LogsRecentResponse {
    let lines = logs
        .iter()
        .map(|entry| format!("[{}] {}", entry.level, entry.message))
        .collect();
    LogsRecentResponse { logs, lines }
}

pub fn log_level_response(level: impl Into<String>) -> LogLevelResponse {
    LogLevelResponse {
        level: level.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_recent_response_includes_legacy_lines() {
        let logs = vec![LogEntry {
            timestamp: "2026-06-02T10:00:00Z".to_string(),
            level: "INFO".to_string(),
            target: "gateway".to_string(),
            message: "ready".to_string(),
        }];
        let value = serde_json::to_value(logs_recent_response(logs)).expect("serialize logs");

        assert_eq!(value["lines"], serde_json::json!(["[INFO] ready"]));
        assert_eq!(value["logs"][0]["message"], "ready");
    }

    #[test]
    fn log_unavailable_errors_use_gateway_status_codes() {
        assert_eq!(
            log_broadcaster_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                LOG_BROADCASTER_UNAVAILABLE_MESSAGE.to_string()
            )
        );
        assert_eq!(
            log_level_control_unavailable_error(),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                LOG_LEVEL_CONTROL_UNAVAILABLE_MESSAGE.to_string()
            )
        );
    }

    #[test]
    fn log_level_response_preserves_shape() {
        assert_eq!(
            serde_json::to_value(log_level_response("debug")).unwrap(),
            serde_json::json!({"level": "debug"})
        );
    }

    #[test]
    fn invalid_log_level_error_uses_bad_request() {
        assert_eq!(
            invalid_log_level_error("invalid level"),
            (StatusCode::BAD_REQUEST, "invalid level".to_string())
        );
    }
}
