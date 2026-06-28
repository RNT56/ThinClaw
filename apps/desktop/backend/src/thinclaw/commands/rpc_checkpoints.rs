//! RPC commands — filesystem checkpoints / `/rollback` (TDO-103).
//!
//! Surfaces the core shadow-git checkpoint system (reversible pre-mutation
//! snapshots) to the desktop. The core implementation lives in
//! `thinclaw_core::agent::checkpoint` (re-exported from `thinclaw-agent`); these
//! are thin command wrappers over its `list_checkpoints` / `diff` / `restore`
//! free functions, which operate on a project directory.
//!
//! Note: checkpoints are disabled unless `checkpoints_enabled` is set in the
//! agent config; in that case the core returns a "checkpoints are disabled"
//! error, which surfaces here as the command error string.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Frontend-facing checkpoint record. Mirrors `thinclaw_core` `CheckpointEntry`
/// but renders the timestamp as an RFC3339 string so the type is specta-exportable.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct CheckpointEntryItem {
    pub commit_hash: String,
    pub timestamp: String,
    pub summary: String,
}

impl From<thinclaw_core::agent::checkpoint::CheckpointEntry> for CheckpointEntryItem {
    fn from(entry: thinclaw_core::agent::checkpoint::CheckpointEntry) -> Self {
        Self {
            commit_hash: entry.commit_hash,
            timestamp: entry.timestamp.to_rfc3339(),
            summary: entry.summary,
        }
    }
}

/// List filesystem checkpoints (shadow-git snapshots) for a project directory,
/// newest first as returned by the core.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_checkpoints_list(
    project_dir: String,
) -> Result<Vec<CheckpointEntryItem>, String> {
    let entries = thinclaw_core::agent::checkpoint::list_checkpoints(&PathBuf::from(project_dir))
        .await
        .map_err(|e| e.to_string())?;
    Ok(entries.into_iter().map(CheckpointEntryItem::from).collect())
}

/// Diff the current project state against a checkpoint commit (unified diff text).
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_checkpoint_diff(
    project_dir: String,
    commit_hash: String,
) -> Result<String, String> {
    thinclaw_core::agent::checkpoint::diff(&PathBuf::from(project_dir), &commit_hash)
        .await
        .map_err(|e| e.to_string())
}

/// Restore a project (or a single `file`) to a checkpoint commit. The core
/// creates a safety snapshot automatically before applying the restore.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_checkpoint_restore(
    project_dir: String,
    commit_hash: String,
    file: Option<String>,
) -> Result<(), String> {
    thinclaw_core::agent::checkpoint::restore(
        &PathBuf::from(project_dir),
        &commit_hash,
        file.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())
}
