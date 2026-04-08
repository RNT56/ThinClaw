//! System status API — engine health, model info, diagnostics.

use std::sync::Arc;

use serde::Serialize;

use crate::app::AppComponents;
use crate::llm::LlmProvider;

use super::error::{ApiError, ApiResult};

/// Snapshot of the engine's current state.
#[derive(Debug, Clone, Serialize)]
pub struct EngineStatus {
    pub engine_running: bool,
    pub setup_completed: bool,
    pub tool_count: usize,
    pub active_extensions: usize,
    pub model_name: String,
    pub cheap_model_name: Option<String>,
    pub db_connected: bool,
    pub workspace_available: bool,
}

/// Information about an available LLM model.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub is_primary: bool,
}

/// Get current engine status.
pub fn get_status(
    components: &AppComponents,
    llm: &Arc<dyn LlmProvider>,
    cheap_llm: Option<&Arc<dyn LlmProvider>>,
) -> EngineStatus {
    EngineStatus {
        engine_running: true,
        setup_completed: true,
        tool_count: components.tools.count(),
        active_extensions: components
            .extension_manager
            .as_ref()
            .map(|_| 1) // placeholder — real count requires async
            .unwrap_or(0),
        model_name: llm.active_model_name(),
        cheap_model_name: cheap_llm.map(|c| c.active_model_name()),
        db_connected: components.db.is_some(),
        workspace_available: components.workspace.is_some(),
    }
}

/// List available models.
pub fn list_models(
    llm: &Arc<dyn LlmProvider>,
    cheap_llm: Option<&Arc<dyn LlmProvider>>,
) -> ApiResult<Vec<ModelInfo>> {
    let mut models = vec![ModelInfo {
        name: llm.active_model_name(),
        is_primary: true,
    }];

    if let Some(cheap) = cheap_llm {
        models.push(ModelInfo {
            name: cheap.active_model_name(),
            is_primary: false,
        });
    }

    Ok(models)
}

/// Result of a database snapshot operation.
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotResult {
    /// Number of bytes written to the snapshot file.
    pub bytes_written: u64,
    /// Path where the snapshot was saved.
    pub path: String,
}

/// Create a portable snapshot of ThinClaw's database.
///
/// This is used by Scrappy's cloud migration engine to include
/// thinclaw.db in cross-device sync. The snapshot is a self-contained
/// SQLite file (WAL is flushed before copy).
///
/// # Arguments
/// * `db` - The database instance to snapshot
/// * `dest` - Destination path for the snapshot file
pub async fn snapshot_database(
    db: &dyn crate::db::Database,
    dest: &std::path::Path,
) -> ApiResult<SnapshotResult> {
    let bytes = db
        .snapshot(dest)
        .await
        .map_err(|e| ApiError::Internal(format!("Database snapshot failed: {}", e)))?;

    Ok(SnapshotResult {
        bytes_written: bytes,
        path: dest.display().to_string(),
    })
}
