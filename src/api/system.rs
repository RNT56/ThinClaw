//! System status API — engine health, model info, diagnostics.

use std::sync::Arc;

use crate::app::AppComponents;
use crate::llm::LlmProvider;

use super::error::{ApiError, ApiResult};

use thinclaw_app::{EngineStatusParts, build_engine_status};

pub use thinclaw_app::{EngineStatus, ModelInfo, SnapshotResult};

/// Get current engine status.
pub async fn get_status(
    components: &AppComponents,
    llm: &Arc<dyn LlmProvider>,
    cheap_llm: Option<&Arc<dyn LlmProvider>>,
) -> EngineStatus {
    let runtime_status = components.llm_runtime.status();
    let active_extensions = if let Some(ref manager) = components.extension_manager {
        manager
            .list(None, false)
            .await
            .map(|extensions| extensions.into_iter().filter(|ext| ext.active).count())
            .unwrap_or(0)
    } else {
        0
    };
    let setup_completed = std::env::var("ONBOARD_COMPLETED")
        .map(|value| value == "true")
        .unwrap_or(false)
        || crate::settings::Settings::load().onboard_completed;

    build_engine_status(EngineStatusParts {
        runtime_revision: runtime_status.revision,
        runtime_last_error: runtime_status.last_error,
        runtime_primary_model: runtime_status.primary_model,
        runtime_cheap_model: runtime_status.cheap_model,
        fallback_model_name: llm.active_model_name(),
        fallback_cheap_model_name: cheap_llm.map(|c| c.active_model_name()),
        setup_completed,
        tool_count: components.tools.count(),
        active_extensions,
        db_connected: components.db.is_some(),
        workspace_available: components.workspace.is_some(),
    })
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

/// Create a portable snapshot of ThinClaw's database.
///
/// This is used by ThinClaw Desktop's cloud migration engine to include
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

#[cfg(test)]
mod tests {
    use super::*;

    fn status_parts() -> EngineStatusParts {
        EngineStatusParts {
            runtime_revision: 3,
            runtime_last_error: None,
            runtime_primary_model: "openai/gpt-test".to_string(),
            runtime_cheap_model: Some("gpt-cheap".to_string()),
            fallback_model_name: "fallback-model".to_string(),
            fallback_cheap_model_name: Some("fallback-cheap".to_string()),
            setup_completed: true,
            tool_count: 7,
            active_extensions: 2,
            db_connected: true,
            workspace_available: true,
        }
    }

    #[test]
    fn status_reports_runtime_model_and_health() {
        let status = build_engine_status(status_parts());

        assert!(status.engine_running);
        assert!(status.llm_runtime_healthy);
        assert_eq!(status.llm_runtime_revision, 3);
        assert_eq!(status.model_name, "openai/gpt-test");
        assert_eq!(status.cheap_model_name.as_deref(), Some("gpt-cheap"));
    }

    #[test]
    fn status_preserves_runtime_error_and_falls_back_for_blank_model() {
        let mut parts = status_parts();
        parts.runtime_revision = 0;
        parts.runtime_last_error = Some("provider reload failed".to_string());
        parts.runtime_primary_model.clear();
        parts.runtime_cheap_model = None;

        let status = build_engine_status(parts);

        assert!(!status.engine_running);
        assert!(!status.llm_runtime_healthy);
        assert_eq!(
            status.llm_last_error.as_deref(),
            Some("provider reload failed")
        );
        assert_eq!(status.model_name, "fallback-model");
        assert_eq!(status.cheap_model_name.as_deref(), Some("fallback-cheap"));
    }
}
