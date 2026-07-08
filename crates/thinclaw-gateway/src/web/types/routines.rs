//! Routine listing, detail, and activity DTOs.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
