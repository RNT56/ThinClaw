pub mod agent;
pub mod chromium_resolver;
pub mod llama_provider;
pub mod orchestrator;
pub mod router;
pub mod tools;

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

use crate::rig_lib::llama_provider::LlamaProvider;

#[command]
#[specta::specta]
pub async fn agent_chat(
    state: tauri::State<'_, crate::sidecar::SidecarManager>,
    request: String,
) -> Result<String, String> {
    let (port, _token, _) = state.get_chat_config().ok_or("Chat server not running")?;
    let base_url = format!("http://127.0.0.1:{}/v1", port);

    // Check for summarizer
    let summarizer_provider = if let Some((sum_port, _sum_token)) = state.get_summarizer_config() {
        let sum_url = format!("http://127.0.0.1:{}/v1", sum_port);
        Some(LlamaProvider::new(&sum_url, "sk-no-key-required"))
    } else {
        None
    };

    // Create a new manager/agent for this request
    // TODO: Ideally we should cache this or manage it in state, but updating it on port change is tricky.
    // For now, cheap construction is fine.
    let manager = RigManager::new(
        base_url,
        "default".to_string(),
        None,
        None,
        8192,
        summarizer_provider,
        false,
        None,
        None,
    );

    manager.chat(&request).await
}
