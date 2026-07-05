//! Desktop autonomy pause/resume DTOs.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Default)]
pub struct AutonomyPauseRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AutonomyPauseResponse {
    pub paused: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autonomy_pause_response_preserves_existing_json_shape() {
        assert_eq!(
            serde_json::to_value(AutonomyPauseResponse { paused: true }).unwrap(),
            serde_json::json!({ "paused": true })
        );
        assert_eq!(
            serde_json::to_value(AutonomyPauseResponse { paused: false }).unwrap(),
            serde_json::json!({ "paused": false })
        );
    }
}
