//! System status API — engine health, model info, diagnostics.

use std::sync::Arc;

use serde::Serialize;

use crate::app::AppComponents;
use crate::llm::LlmProvider;

use super::error::ApiResult;

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
        model_name: llm.model_name().to_string(),
        cheap_model_name: cheap_llm.map(|c| c.model_name().to_string()),
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
        name: llm.model_name().to_string(),
        is_primary: true,
    }];

    if let Some(cheap) = cheap_llm {
        models.push(ModelInfo {
            name: cheap.model_name().to_string(),
            is_primary: false,
        });
    }

    Ok(models)
}
