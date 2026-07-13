//! RPC commands — trajectory viewer and training export (TDO-106/TDO-114).
//!
//! Surfaces the per-turn trajectory archive (JSONL records the agent writes
//! after each turn, used for training-data export and learning feedback) to the
//! desktop. Thin wrappers over `thinclaw_core::agent::learning::TrajectoryLogger`
//! (`stats` / `load_records`). The logger reads from the platform trajectories
//! data dir; file IO is run on a blocking thread.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tauri::State;

use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

const DESKTOP_EXPORT_MAX_RECORDS: usize = 5_000;

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

/// Bounded training export returned to the frontend for an explicit download.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct TrajectoryExportItem {
    pub format: String,
    pub payload: String,
    pub source_record_count: u32,
    pub exported_record_count: u32,
    pub skipped_counts: BTreeMap<String, u32>,
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
pub async fn thinclaw_trajectory_stats(
) -> Result<TrajectoryStatsItem, crate::thinclaw::bridge::BridgeError> {
    let stats = tokio::task::spawn_blocking(|| {
        thinclaw_core::agent::learning::TrajectoryLogger::new().stats()
    })
    .await
    .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?
    .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;
    Ok(TrajectoryStatsItem::from(stats))
}

/// The most recent trajectory turn records (default 100), as raw JSON values.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_trajectory_records(
    limit: Option<u32>,
) -> Result<Vec<serde_json::Value>, crate::thinclaw::bridge::BridgeError> {
    let records = tokio::task::spawn_blocking(|| {
        thinclaw_core::agent::learning::TrajectoryLogger::new().load_records()
    })
    .await
    .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?
    .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))?;

    let limit = limit.unwrap_or(100) as usize;
    let start = records.len().saturating_sub(limit);
    records[start..]
        .iter()
        .map(|r| {
            serde_json::to_value(r)
                .map_err(|e| crate::thinclaw::bridge::BridgeError::from(e.to_string()))
        })
        .collect()
}

/// Export the local trajectory archive as canonical SFT or DPO JSONL.
///
/// The payload is bounded before crossing IPC. The frontend only writes it
/// after an explicit click, so background polling never exposes or downloads
/// training data.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_trajectory_export(
    ironclaw: State<'_, ThinClawRuntimeState>,
    format: String,
) -> Result<TrajectoryExportItem, crate::thinclaw::bridge::BridgeError> {
    let format = desktop_export_format(&format)?;
    let db = match ironclaw.agent().await {
        Ok(agent) => agent.store().cloned(),
        Err(_) => None,
    };
    let rendered = thinclaw_core::cli::trajectory::export_trajectory_archive(
        format,
        thinclaw_core::cli::trajectory::TrajectoryExportOptions {
            min_score: 0.7,
            max_records: Some(DESKTOP_EXPORT_MAX_RECORDS),
        },
        db.as_ref(),
    )
    .await
    .map_err(|error| crate::thinclaw::bridge::BridgeError::from(error.to_string()))?;

    Ok(TrajectoryExportItem {
        format: rendered.format,
        payload: rendered.payload,
        source_record_count: u32::try_from(rendered.source_record_count).unwrap_or(u32::MAX),
        exported_record_count: u32::try_from(rendered.exported_record_count).unwrap_or(u32::MAX),
        skipped_counts: rendered
            .skipped_counts
            .into_iter()
            .map(|(reason, count)| (reason, u32::try_from(count).unwrap_or(u32::MAX)))
            .collect(),
    })
}

fn desktop_export_format(
    format: &str,
) -> Result<&'static str, crate::thinclaw::bridge::BridgeError> {
    match format.trim().to_ascii_lowercase().as_str() {
        "sft" => Ok("sft"),
        "dpo" => Ok("dpo"),
        _ => Err(
            ("Unsupported trajectory export format; expected 'sft' or 'dpo'".to_string()).into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::desktop_export_format;

    #[test]
    fn desktop_export_accepts_only_training_formats() {
        assert_eq!(desktop_export_format(" SFT "), Ok("sft"));
        assert_eq!(desktop_export_format("dPo"), Ok("dpo"));
        assert!(desktop_export_format("jsonl").is_err());
        assert!(desktop_export_format("../secrets").is_err());
    }
}
