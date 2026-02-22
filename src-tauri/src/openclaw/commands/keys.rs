//! API key management, secrets, toggles, cloud config, and channel settings
//!
//! Contains all commands for getting/saving API keys, managing custom secrets,
//! toggling access, saving channel configs (Slack/Telegram), gateway settings,
//! agent profiles, and cloud provider config.

use tauri::State;
use tracing::{error, info, warn};

use super::super::config::*;
use super::types::*;
use super::ws_rpc;
use super::OpenClawManager;
use crate::sidecar::SidecarManager;

/// Get OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_openai_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openai_api_key))
}

/// Save OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_openai_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openai_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_openrouter_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openrouter_api_key))
}

/// Save OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_openrouter_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openrouter_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Get Gemini API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_gemini_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.gemini_api_key))
}

/// Save Gemini API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_gemini_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_gemini_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get Groq API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_groq_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.groq_api_key))
}

/// Save Groq API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_groq_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_groq_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get Anthropic API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_anthropic_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.anthropic_api_key))
}

/// Get Brave Search API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_brave_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.brave_search_api_key))
}

/// Save Slack configuration
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_anthropic_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let val = key.unwrap_or_else(|| "".to_string());
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "anthropicApiKey": val }))
                .await
        })
        .await?;
        return Ok(());
    }

    println!(
        "[openclaw] save_anthropic_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    // Update config structure on disk
    let result = cfg.update_anthropic_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;

    // If running, we might want to update the running config too
    // For now, we'll just update the manager's config so it's used on next start
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Save Brave Search API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_brave_key(
    state: State<'_, OpenClawManager>,
    key: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        let val = key.unwrap_or_else(|| "".to_string());
        let _ = ws_rpc(state, |h| async move {
            h.config_patch(serde_json::json!({ "braveSearchApiKey": val }))
                .await
        })
        .await?;
        return Ok(());
    }

    println!(
        "[openclaw] save_brave_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    let result = cfg.update_brave_key(key);

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle secret access for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_secret_access(
    state: State<'_, OpenClawManager>,
    secret: String,
    granted: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if cfg.gateway_mode == "remote" {
        // Map secret IDs to config fields if possible
        let patch_key = match secret.as_str() {
            "anthropic" => Some("anthropicGranted"),
            "openai" => Some("openaiGranted"),
            "openrouter" => Some("openrouterGranted"),
            "gemini" => Some("geminiGranted"),
            "groq" => Some("groqGranted"),
            "huggingface" => Some("huggingfaceGranted"),
            "brave" => Some("braveGranted"),
            "xai" => Some("xaiGranted"),
            "venice" => Some("veniceGranted"),
            "together" => Some("togetherGranted"),
            "moonshot" => Some("moonshotGranted"),
            "minimax" => Some("minimaxGranted"),
            "nvidia" => Some("nvidiaGranted"),
            "qianfan" => Some("qianfanGranted"),
            "mistral" => Some("mistralGranted"),
            "xiaomi" => Some("xiaomiGranted"),
            "amazon-bedrock" | "bedrock" => Some("bedrockGranted"),
            _ => None, // Custom secrets or unknown
        };

        if let Some(key) = patch_key {
            let _ = ws_rpc(state, |h| async move {
                h.config_patch(serde_json::json!({ key: granted })).await
            })
            .await?;
            return Ok(());
        } else if secret.starts_with("custom-") {
            // For custom secrets, we might need a specialized RPC or a complex patch
            // For now, let's assume specific RPC support or just fail gracefully warning
            warn!(
                "Remote toggling of custom secret '{}' not yet supported via simple patch",
                secret
            );
            // Alternatively, if the backend supports "customSecrets" array patch, we could send that, but it's race-condition prone.
            return Err("Remote toggling of custom secrets not yet supported".into());
        }
    }

    let result = cfg.toggle_secret_access(&secret, granted);

    // Regenerate config to reflect access change in auth-profiles.json
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Select the cloud brain to use for the agent
#[tauri::command]
#[specta::specta]
pub async fn select_openclaw_brain(
    state: State<'_, OpenClawManager>,
    brain: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.update_selected_cloud_brain(brain)
        .map_err(|e| e.to_string())?;

    // Regenerate config/profiles
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Save HuggingFace token
#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_hf_token(
    state: State<'_, OpenClawManager>,
    token: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[openclaw] set_hf_token: attempting to set (empty: {})",
        token.trim().is_empty()
    );

    let val = if token.trim().is_empty() {
        None
    } else {
        Some(token.trim().to_string())
    };

    let result = cfg.update_huggingface_token(val);

    // Regenerate config/profiles
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;

    // Update in-memory state
    *state.config.write().await = Some(cfg);
    println!("[openclaw] set_hf_token: successfully saved and updated state");

    Ok(())
}

/// Save an implicit cloud provider API key (generic)
/// Supports: xai, venice, together, moonshot, minimax, nvidia, qianfan, mistral, xiaomi
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_implicit_provider_key(
    state: State<'_, OpenClawManager>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let valid_providers = [
        "xai", "venice", "together", "moonshot", "minimax", "nvidia", "qianfan", "mistral",
        "xiaomi",
    ];
    if !valid_providers.contains(&provider.as_str()) {
        return Err(format!("Unknown implicit provider: {}", provider));
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[openclaw] save_implicit_provider_key: {} (empty: {})",
        provider,
        key.trim().is_empty()
    );

    let val = if key.trim().is_empty() {
        None
    } else {
        Some(key.trim().to_string())
    };

    let result = cfg.update_implicit_provider_key(&provider, val);

    // Regenerate config/profiles
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    println!(
        "[openclaw] save_implicit_provider_key: {} saved successfully",
        provider
    );
    Ok(())
}

/// Get an implicit cloud provider API key (generic)
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_implicit_provider_key(
    state: State<'_, OpenClawManager>,
    provider: String,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.get_implicit_provider_key(&provider)))
}

/// Save Amazon Bedrock AWS credentials
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_bedrock_credentials(
    state: State<'_, OpenClawManager>,
    access_key_id: String,
    secret_access_key: String,
    region: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[openclaw] save_bedrock_credentials: ak={} sk=*** region={}",
        if access_key_id.trim().is_empty() {
            "(empty)"
        } else {
            "(set)"
        },
        if region.trim().is_empty() {
            "us-east-1"
        } else {
            &region
        },
    );

    let ak = if access_key_id.trim().is_empty() {
        None
    } else {
        Some(access_key_id.trim().to_string())
    };
    let sk = if secret_access_key.trim().is_empty() {
        None
    } else {
        Some(secret_access_key.trim().to_string())
    };
    let r = if region.trim().is_empty() {
        None
    } else {
        Some(region.trim().to_string())
    };

    cfg.update_bedrock_credentials(ak, sk, r)
        .map_err(|e| e.to_string())?;

    // Regenerate config/profiles
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    println!("[openclaw] save_bedrock_credentials: saved successfully");
    Ok(())
}

/// Get Amazon Bedrock credentials
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_bedrock_credentials(
    state: State<'_, OpenClawManager>,
) -> Result<(Option<String>, Option<String>, Option<String>), String> {
    let config = state.get_config().await;
    Ok(config
        .map(|cfg| cfg.get_bedrock_credentials())
        .unwrap_or((None, None, None)))
}

/// Add a custom secret
#[tauri::command]
#[specta::specta]
pub async fn openclaw_add_custom_secret(
    state: State<'_, OpenClawManager>,
    name: String,
    value: String,
    description: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let id = format!("custom-{}", uuid::Uuid::new_v4());
    cfg.custom_secrets.push(CustomSecret {
        id: id.clone(),
        name,
        value,
        description,
        granted: false,
    });

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect changes in auth-profiles.json
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Remove a custom secret
#[tauri::command]
#[specta::specta]
pub async fn openclaw_remove_custom_secret(
    state: State<'_, OpenClawManager>,
    id: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.custom_secrets.retain(|s| s.id != id);

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect changes in auth-profiles.json
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle custom secret access for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_custom_secret(
    state: State<'_, OpenClawManager>,
    id: String,
    granted: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if let Some(secret) = cfg.custom_secrets.iter_mut().find(|s| s.id == id) {
        secret.granted = granted;
    } else {
        return Err("Secret not found".into());
    }

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect access change in auth-profiles.json
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let openclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle node host (OS automation) for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_node_host(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, SidecarManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[openclaw] Toggling node host to: {}", enabled);
    cfg.node_host_enabled = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect policy change
    // Preserve channel settings from existing openclaw_engine.json if it exists
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = sidecar.get_chat_config();
    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&openclaw_engine, local_llm).map_err(|e| {
        let err = format!("Failed to write openclaw_engine config: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // If already running in remote mode, start/stop the node host immediately
    if *state.running.read().await && cfg.gateway_mode == "remote" {
        if enabled {
            state.start_openclaw_engine_process(&cfg, "node").await?;
        } else if let Some(proc) = state.node_host_process.lock().await.take() {
            let _ = proc.kill();
        }
    }

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle local inference (exposing local LLM) for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_local_inference(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, SidecarManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[openclaw] Toggling local inference to: {}", enabled);
    cfg.local_inference_enabled = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect priority change
    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = sidecar.get_chat_config();
    let openclaw_engine = cfg.generate_config(
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_openclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&openclaw_engine, local_llm).map_err(|e| {
        let err = format!("Failed to write openclaw_engine config: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // If turning off local inference, we can kill the chat server to free resources
    if !enabled {
        let _ = sidecar.stop_chat_server();
    }

    *state.config.write().await = Some(cfg);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_slack_config(
    state: State<'_, OpenClawManager>,
    config_input: SlackConfigInput,
) -> Result<(), String> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());
    let mut openclaw_engine = existing_openclaw_engine
        .unwrap_or_else(|| cfg.generate_config(None, None, local_llm.clone()));

    openclaw_engine.channels.slack = SlackConfig {
        enabled: config_input.enabled,
        bot_token: config_input.bot_token,
        app_token: config_input.app_token,
        ..Default::default()
    };

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;
    info!("Saved Slack config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Telegram configuration
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_telegram_config(
    state: State<'_, OpenClawManager>,
    config_input: TelegramConfigInput,
) -> Result<(), String> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let existing_openclaw_engine = cfg.load_config().ok();
    let local_llm = existing_openclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());
    let mut openclaw_engine = existing_openclaw_engine
        .unwrap_or_else(|| cfg.generate_config(None, None, local_llm.clone()));

    openclaw_engine.channels.telegram = TelegramConfig {
        enabled: config_input.enabled,
        bot_token: config_input.bot_token,
        dm_policy: config_input.dm_policy,
        groups: if config_input.groups_enabled {
            TelegramGroupsConfig::default()
        } else {
            TelegramGroupsConfig {
                wildcard: TelegramGroupConfig {
                    require_mention: true,
                },
            }
        },
    };

    cfg.write_config(&openclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;
    info!("Saved Telegram config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Gateway configuration
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_gateway_settings(
    state: State<'_, OpenClawManager>,
    mode: String,
    url: Option<String>,
    token: Option<String>,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let url_opt = url.filter(|s| !s.trim().is_empty());
    let token_opt = token.filter(|s| !s.trim().is_empty());

    cfg.update_gateway_settings(mode, url_opt, token_opt)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Add or update an agent profile
#[tauri::command]
#[specta::specta]
pub async fn openclaw_add_agent_profile(
    state: State<'_, OpenClawManager>,
    profile: AgentProfile,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    if let Some(existing) = cfg.profiles.iter_mut().find(|p| p.id == profile.id) {
        *existing = profile;
    } else {
        cfg.profiles.push(profile);
    }

    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Remove an agent profile
#[tauri::command]
#[specta::specta]
pub async fn openclaw_remove_agent_profile(
    state: State<'_, OpenClawManager>,
    id: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    cfg.profiles.retain(|p| p.id != id);

    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}
