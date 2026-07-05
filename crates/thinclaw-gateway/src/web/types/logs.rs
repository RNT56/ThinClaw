//! Log streaming and log-level control DTOs.

use serde::{Deserialize, Serialize};

use crate::web::log_layer::LogEntry;

#[derive(Debug, Clone, Serialize)]
pub struct LogsRecentResponse {
    pub logs: Vec<LogEntry>,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LogLevelResponse {
    pub level: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct LogLevelRequest {
    pub level: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_responses_preserve_existing_json_shape() {
        let entry = LogEntry {
            level: "INFO".to_string(),
            target: "thinclaw".to_string(),
            message: "ready".to_string(),
            timestamp: "2026-06-02T00:00:00Z".to_string(),
        };

        assert_eq!(
            serde_json::to_value(LogsRecentResponse {
                logs: vec![entry],
                lines: vec!["[INFO] ready".to_string()],
            })
            .unwrap(),
            serde_json::json!({
                "logs": [{
                    "level": "INFO",
                    "target": "thinclaw",
                    "message": "ready",
                    "timestamp": "2026-06-02T00:00:00Z"
                }],
                "lines": ["[INFO] ready"]
            })
        );

        assert_eq!(
            serde_json::to_value(LogLevelResponse {
                level: "debug".to_string(),
            })
            .unwrap(),
            serde_json::json!({ "level": "debug" })
        );

        let request: LogLevelRequest =
            serde_json::from_value(serde_json::json!({ "level": "warn" })).unwrap();
        assert_eq!(request.level, "warn");
    }
}
