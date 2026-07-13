//! Optional local Docker sandbox and orchestrator assembly.

use std::sync::Arc;

use thinclaw_core::channels::web::types::SseEvent;

/// Build a local Docker sandbox `ContainerJobManager` + orchestrator for the
/// desktop runtime (mirrors the server path in `src/main.rs`). Returns `None`
/// when the sandbox is disabled in config. Actual container spawning is further
/// gated by the repo project supervisor (and Docker availability) at runtime.
#[cfg(feature = "docker-sandbox")]
pub(super) async fn build_desktop_container_job_manager(
    config: &thinclaw_core::config::Config,
    llm: Arc<dyn thinclaw_core::llm::LlmProvider>,
    db: Option<Arc<dyn thinclaw_core::db::Database>>,
    secrets: Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
) -> Option<Arc<thinclaw_core::sandbox_types::ContainerJobManager>> {
    use thinclaw_core::orchestrator::api::{OrchestratorState, PendingPrompt};
    use thinclaw_core::orchestrator::OrchestratorApi;
    use thinclaw_core::sandbox_types::{ContainerJobConfig, ContainerJobManager, TokenStore};

    if !config.sandbox.enabled {
        return None;
    }

    let token_store = TokenStore::new();
    let claude_code_api_key =
        resolve_desktop_provider_key("ANTHROPIC_API_KEY", "llm_anthropic_api_key", &secrets).await;
    let codex_code_api_key =
        resolve_desktop_provider_key("OPENAI_API_KEY", "llm_openai_api_key", &secrets).await;

    let job_config = ContainerJobConfig {
        image: config.sandbox.image.clone(),
        memory_limit_mb: config.sandbox.memory_limit_mb,
        cpu_shares: config.sandbox.cpu_shares,
        orchestrator_port: 50051,
        claude_code_api_key,
        claude_code_oauth_token: thinclaw_core::config::ClaudeCodeConfig::extract_oauth_token(),
        claude_code_enabled: config.claude_code.enabled,
        claude_code_model: config.claude_code.model.clone(),
        claude_code_max_turns: config.claude_code.max_turns,
        claude_code_memory_limit_mb: config.claude_code.memory_limit_mb,
        claude_code_allowed_tools: config.claude_code.allowed_tools.clone(),
        codex_code_api_key,
        codex_code_enabled: config.codex_code.enabled,
        codex_code_model: config.codex_code.model.clone(),
        codex_code_memory_limit_mb: config.codex_code.memory_limit_mb,
        codex_code_home_dir: config.codex_code.home_dir.clone(),
        interactive_idle_timeout_secs: config.sandbox.interactive_idle_timeout_secs,
    };
    let job_manager = Arc::new(ContainerJobManager::new(job_config, token_store.clone()));

    {
        let cleanup = Arc::clone(&job_manager);
        tokio::spawn(async move {
            cleanup.cleanup_orphan_containers().await;
        });
    }

    let prompt_queue = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::<
        uuid::Uuid,
        std::collections::VecDeque<PendingPrompt>,
    >::new()));
    let (job_event_tx, _) = tokio::sync::broadcast::channel::<(uuid::Uuid, SseEvent)>(256);

    let orchestrator_state = OrchestratorState {
        llm,
        job_manager: Arc::clone(&job_manager),
        token_store,
        job_event_tx: Some(job_event_tx),
        prompt_queue,
        store: db,
        secrets_store: secrets,
    };
    tokio::spawn(async move {
        if let Err(error) = OrchestratorApi::start(orchestrator_state, 50051).await {
            tracing::error!("desktop orchestrator API failed: {error}");
        }
    });

    tracing::info!("[thinclaw-runtime] Local Docker sandbox orchestrator started on :50051");
    Some(job_manager)
}

#[cfg(feature = "docker-sandbox")]
async fn resolve_desktop_provider_key(
    env_key: &str,
    secret_name: &str,
    secrets: &Option<Arc<dyn thinclaw_core::secrets::SecretsStore + Send + Sync>>,
) -> Option<String> {
    // Desktop crate is edition 2021 (no let-chains), so these are nested.
    if let Ok(value) = std::env::var(env_key) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    if let Some(store) = secrets {
        if let Ok(secret) = store
            .get_for_injection(
                "default",
                secret_name,
                thinclaw_core::secrets::SecretAccessContext::new(
                    "desktop.sandbox",
                    "container_provider_key",
                ),
            )
            .await
        {
            let value = secret.expose().to_string();
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}
