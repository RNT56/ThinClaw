pub mod agent;
pub mod chromium_resolver;
pub mod llama_provider;
pub mod orchestrator;
pub mod router;
pub mod sandbox_factory;
pub mod tool_discovery;
pub mod tool_router;
pub mod tools;
pub mod unified_provider;

pub use agent::RigManager;

use crate::rig_lib::tools::web_search::{DDGSearchTool, SearchArgs};
use rig::tool::Tool;
use tauri::command;

#[command]
#[specta::specta]
pub async fn rig_check_web_search(query: String) -> Result<String, String> {
    let tool = DDGSearchTool {
        app: None,
        max_total_chars: 4000,
        summarizer: None,
        conversation_id: None,
    };
    let args = SearchArgs { query };
    tool.call(args).await.map_err(|e| e.to_string())
}

// use crate::rig_lib::llama_provider::LlamaProvider;

#[command]
#[specta::specta]
pub async fn agent_chat(
    state: tauri::State<'_, crate::sidecar::SidecarManager>,
    engine_manager: tauri::State<'_, crate::engine::EngineManager>,
    request: String,
) -> Result<String, String> {
    let snapshot = crate::engine::local_runtime_snapshot(&state, &engine_manager).await;
    let endpoint = snapshot.endpoint.as_ref().ok_or_else(|| {
        format!(
            "Chat runtime not running: {}",
            snapshot
                .unavailable_reason
                .as_deref()
                .unwrap_or("runtime endpoint unavailable")
        )
    })?;
    let base_url = endpoint.base_url.clone();
    let api_key = endpoint
        .api_key
        .clone()
        .filter(|token| !token.is_empty())
        .unwrap_or_else(|| "sk-no-key-required".to_string());
    let model_id = endpoint
        .model_id
        .clone()
        .unwrap_or_else(|| "default".to_string());
    let context_size = endpoint.context_size.unwrap_or(8192) as usize;
    let model_family = endpoint.model_family.clone();

    // Check for summarizer
    let summarizer_provider = if let Some((sum_port, _sum_token)) = state.get_summarizer_config() {
        let sum_url = format!("http://127.0.0.1:{}/v1", sum_port);
        Some(crate::rig_lib::unified_provider::UnifiedProvider::new(
            crate::rig_lib::unified_provider::ProviderKind::Local,
            &sum_url,
            "sk-no-key-required",
            "default",
            model_family.clone(),
        ))
    } else {
        None
    };

    // Create a new manager/agent for this request
    // TODO: Ideally we should cache this or manage it in state, but updating it on port change is tricky.
    // For now, cheap construction is fine.
    let manager = RigManager::new(
        crate::rig_lib::unified_provider::ProviderKind::Local,
        base_url,
        model_id,
        None,
        Some(api_key),
        context_size,
        summarizer_provider,
        false,
        None,
        None,
        model_family,
    );

    manager.chat(&request).await
}
