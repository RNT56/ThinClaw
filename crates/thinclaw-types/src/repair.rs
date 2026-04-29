use std::time::Duration;

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A job that has been detected as stuck.
#[derive(Debug, Clone)]
pub struct StuckJob {
    pub job_id: Uuid,
    pub last_activity: DateTime<Utc>,
    pub stuck_duration: Duration,
    pub last_error: Option<String>,
    pub repair_attempts: u32,
}

/// A tool that has been detected as broken.
#[derive(Debug, Clone)]
pub struct BrokenTool {
    pub name: String,
    pub failure_count: u32,
    pub last_error: Option<String>,
    pub first_failure: DateTime<Utc>,
    pub last_failure: DateTime<Utc>,
    pub last_build_result: Option<serde_json::Value>,
    pub repair_attempts: u32,
}
