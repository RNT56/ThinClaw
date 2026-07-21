//! RPC commands — trajectory viewer (TDO-106).
//!
//! Surfaces the per-turn trajectory archive (JSONL records the agent writes
//! after each turn, used for training-data export and learning feedback) to the
//! desktop. Thin wrappers over `thinclaw_core::agent::learning::TrajectoryLogger`
//! (`stats` / `load_records`). The logger reads from the platform trajectories
//! data dir; file IO is run on a blocking thread.

use serde::{Deserialize, Serialize};

/// Frontend-facing aggregate trajectory stats. Mirrors `TrajectoryStats` with
/// paths/timestamps rendered as strings so the type is specta-exportable.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TrajectoryStatsItem {
    pub log_root: String,
    pub file_count: u32,
    pub record_count: u32,
    pub session_count: u32,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
    pub success_count: u32,
    pub failure_count: u32,
    pub neutral_count: u32,
}

impl From<thinclaw_core::agent::learning::TrajectoryStats> for TrajectoryStatsItem {
    fn from(s: thinclaw_core::agent::learning::TrajectoryStats) -> Self {
        Self {
            log_root: s.log_root.to_string_lossy().into_owned(),
            file_count: s.file_count as u32,
            record_count: s.record_count as u32,
            session_count: s.session_count as u32,
            first_seen: s.first_seen.map(|d| d.to_rfc3339()),
            last_seen: s.last_seen.map(|d| d.to_rfc3339()),
            success_count: s.success_count as u32,
            failure_count: s.failure_count as u32,
            neutral_count: s.neutral_count as u32,
        }
    }
}

/// Aggregate stats over the local trajectory archive (counts, span, outcomes).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_trajectory_stats() -> Result<TrajectoryStatsItem, String> {
    let stats = tokio::task::spawn_blocking(|| {
        thinclaw_core::agent::learning::TrajectoryLogger::new().stats()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    Ok(TrajectoryStatsItem::from(stats))
}

/// The most recent trajectory turn records (default 100), as raw JSON values.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_trajectory_records(
    limit: Option<u32>,
) -> Result<Vec<serde_json::Value>, String> {
    let records = tokio::task::spawn_blocking(|| {
        thinclaw_core::agent::learning::TrajectoryLogger::new().load_records()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let limit = limit.unwrap_or(100).clamp(1, 1_000) as usize;
    let start = records.len().saturating_sub(limit);
    records[start..]
        .iter()
        .map(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
        .collect()
}
