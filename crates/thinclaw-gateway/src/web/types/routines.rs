//! Routine listing, detail, and activity DTOs.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutineCreateTriggerType {
    #[default]
    Cron,
    SystemEvent,
}

#[derive(Debug, Serialize)]
pub struct RoutineInfo {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger_type: String,
    pub trigger_summary: String,
    pub action_type: String,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct RoutineListResponse {
    pub routines: Vec<RoutineInfo>,
}

#[derive(Debug, Deserialize)]
pub struct RoutineCreateRequest {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub schedule: String,
    pub task: String,
    #[serde(default)]
    pub trigger_type: RoutineCreateTriggerType,
}

#[derive(Debug, Deserialize)]
pub struct RoutineClearRunsRequest {
    #[serde(default)]
    pub routine_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct ToggleRequest {
    pub enabled: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct RoutineSummaryResponse {
    pub total: u64,
    pub enabled: u64,
    pub disabled: u64,
    pub failing: u64,
    pub runs_today: u64,
}

#[derive(Debug, Serialize)]
pub struct RoutineDetailResponse {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub trigger: serde_json::Value,
    pub action: serde_json::Value,
    pub guardrails: serde_json::Value,
    pub notify: serde_json::Value,
    pub policy: serde_json::Value,
    pub last_run_at: Option<String>,
    pub next_fire_at: Option<String>,
    pub run_count: u64,
    pub consecutive_failures: u32,
    pub created_at: String,
    pub recent_runs: Vec<RoutineRunInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_event_checks: Vec<RoutineEventCheckInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_trigger_checks: Vec<RoutineTriggerCheckInfo>,
}

#[derive(Debug, Serialize)]
pub struct RoutineRunInfo {
    pub id: Uuid,
    pub trigger_type: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub status: String,
    pub result_summary: Option<String>,
    pub tokens_used: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
pub struct RoutineEventCheckInfo {
    pub id: Uuid,
    pub event_id: Uuid,
    pub decision: String,
    pub reason: Option<String>,
    pub details: serde_json::Value,
    pub sequence_num: u32,
    pub channel: String,
    pub content_preview: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct RoutineTriggerCheckInfo {
    pub id: Uuid,
    pub trigger_kind: String,
    pub due_at: String,
    pub status: String,
    pub decision: Option<String>,
    pub claimed_by: Option<String>,
    pub processed_at: Option<String>,
    pub coalesced_count: u32,
    pub backlog_collapsed: bool,
    pub diagnostics: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RoutineEventActivityInfo {
    pub id: Uuid,
    pub channel: String,
    pub content_preview: String,
    pub status: String,
    pub created_at: String,
    pub processed_at: Option<String>,
    pub matched_routines: u32,
    pub fired_routines: u32,
    pub attempt_count: u32,
    pub error_message: Option<String>,
    pub diagnostics: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RoutineEventActivityResponse {
    pub events: Vec<RoutineEventActivityInfo>,
}

#[cfg(test)]
mod tests {
    use super::{RoutineCreateRequest, RoutineCreateTriggerType};

    #[test]
    fn routine_create_request_defaults_to_cron_for_existing_clients() {
        let request: RoutineCreateRequest = serde_json::from_value(serde_json::json!({
            "name": "Daily review",
            "schedule": "0 0 9 * * * *",
            "task": "Review open work"
        }))
        .expect("legacy routine create request should deserialize");

        assert_eq!(request.trigger_type, RoutineCreateTriggerType::Cron);
    }

    #[test]
    fn routine_create_request_accepts_system_event_trigger() {
        let request: RoutineCreateRequest = serde_json::from_value(serde_json::json!({
            "name": "Heartbeat reminder",
            "schedule": "0 0 9 * * * *",
            "task": "Review stalled pull requests",
            "trigger_type": "system_event"
        }))
        .expect("system event routine create request should deserialize");

        assert_eq!(request.trigger_type, RoutineCreateTriggerType::SystemEvent);
    }

    #[test]
    fn routine_create_request_rejects_unknown_trigger_type() {
        let result = serde_json::from_value::<RoutineCreateRequest>(serde_json::json!({
            "name": "Bad trigger",
            "schedule": "0 0 9 * * * *",
            "task": "Do work",
            "trigger_type": "magic"
        }));

        assert!(result.is_err());
    }
}
