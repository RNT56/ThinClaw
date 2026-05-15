//! API key management, secrets, toggles, cloud config, and channel settings
//!
//! Contains all commands for getting/saving API keys, managing custom secrets,
//! toggling access, saving channel configs (Slack/Telegram), gateway settings,
//! agent profiles, and cloud provider config.

use tauri::State;
use tracing::{error, info};

use super::super::config::*;
use super::remote_provider_config::{apply_remote_selected_brain, normalize_provider_slug};
use super::types::*;
// ws_rpc removed — IronClaw is in-process, no remote WS gateway
use super::OpenClawManager;
use crate::openclaw::ironclaw_bridge::IronClawState;
use crate::sidecar::SidecarManager;

/// Get OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_openai_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openai_api_key.clone()))
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
    Ok(config.and_then(|cfg| cfg.openrouter_api_key.clone()))
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
    Ok(config.and_then(|cfg| cfg.gemini_api_key.clone()))
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
    Ok(config.and_then(|cfg| cfg.groq_api_key.clone()))
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
    Ok(config.and_then(|cfg| cfg.anthropic_api_key.clone()))
}

/// Get Brave Search API key
#[tauri::command]
#[specta::specta]
pub async fn openclaw_get_brave_key(
    state: State<'_, OpenClawManager>,
) -> Result<Option<String>, String> {
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.brave_search_api_key.clone()))
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

    // Remote WS gateway path removed — IronClaw is in-process

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

    // Remote WS gateway path removed — IronClaw is in-process

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

    // Remote WS gateway path removed — IronClaw is in-process

    let result = cfg.toggle_secret_access(&secret, granted);

    // Regenerate config to reflect access change
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
    ironclaw: State<'_, IronClawState>,
    brain: Option<String>,
) -> Result<(), String> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let mut remote_config = proxy
            .get_providers_config()
            .await
            .map_err(|err| format!("unavailable: remote provider config: {}", err))?;
        apply_remote_selected_brain(&mut remote_config, brain.as_deref());
        proxy
            .set_providers_config(&remote_config)
            .await
            .map_err(|err| format!("remote provider config update failed: {}", err))?;
    }

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
/// Supports: xai, venice, together, moonshot, minimax, nvidia, qianfan, mistral,
/// xiaomi, cohere, voyage, deepgram, elevenlabs, stability, fal.
#[tauri::command]
#[specta::specta]
pub async fn openclaw_save_implicit_provider_key(
    state: State<'_, OpenClawManager>,
    ironclaw: State<'_, IronClawState>,
    provider: String,
    key: String,
) -> Result<(), String> {
    let valid_providers = [
        "xai",
        "venice",
        "together",
        "moonshot",
        "minimax",
        "nvidia",
        "qianfan",
        "mistral",
        "xiaomi",
        "cohere",
        "voyage",
        "deepgram",
        "elevenlabs",
        "stability",
        "fal",
    ];
    if !valid_providers.contains(&provider.as_str()) {
        return Err(format!("Unknown implicit provider: {}", provider));
    }

    if let Some(proxy) = ironclaw.remote_proxy().await {
        let key = key.trim();
        let provider_slug = normalize_provider_slug(&provider);
        if key.is_empty() {
            proxy
                .delete_provider_key(&provider_slug)
                .await
                .map_err(|err| format!("remote provider key delete failed: {}", err))?;
        } else {
            proxy
                .save_provider_key(&provider_slug, key)
                .await
                .map_err(|err| format!("remote provider key save failed: {}", err))?;
        }
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

    // Store the secret value in the Keychain, not in identity.json
    crate::openclaw::config::keychain::set_key(&id, Some(&value))
        .map_err(|e| format!("Keychain error: {}", e))?;

    cfg.custom_secrets.push(CustomSecret {
        id: id.clone(),
        name,
        value, // kept in memory only; #[serde(skip)] prevents JSON persistence
        description,
        granted: false,
    });

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect changes
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

    // Delete the secret value from the Keychain
    let _ = crate::openclaw::config::keychain::set_key(&id, None);

    cfg.custom_secrets.retain(|s| s.id != id);

    cfg.save_identity().map_err(|e| e.to_string())?;

    // Regenerate config to reflect changes
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

    // Regenerate config to reflect access change
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

/// Toggle local dev tools (shell, write_file, read_file, etc.) for OpenClaw
#[tauri::command]
#[specta::specta]
pub async fn openclaw_toggle_local_tools(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, SidecarManager>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[openclaw] Toggling local tools to: {}", enabled);
    cfg.allow_local_tools = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect the change
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

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Set workspace mode and optional root directory for the agent
#[tauri::command]
#[specta::specta]
pub async fn openclaw_set_workspace_mode(
    state: State<'_, OpenClawManager>,
    sidecar: State<'_, SidecarManager>,
    mode: String,
    root: Option<String>,
) -> Result<String, String> {
    // Validate mode
    if !matches!(mode.as_str(), "unrestricted" | "sandboxed" | "project") {
        return Err(format!(
            "Invalid workspace mode '{}'. Must be 'unrestricted', 'sandboxed', or 'project'.",
            mode
        ));
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Resolve the workspace root:
    // - If the user provided one, validate and use it
    // - If not and mode needs one, auto-generate in the app data dir
    let resolved_root = if mode == "unrestricted" {
        None
    } else if let Some(ref root_path) = root {
        if !root_path.is_empty() {
            let path = std::path::Path::new(root_path);
            if !path.is_absolute() {
                return Err("Workspace root must be an absolute path.".to_string());
            }
            if let Err(e) = std::fs::create_dir_all(path) {
                return Err(format!("Failed to create workspace directory: {}", e));
            }
            Some(root_path.clone())
        } else {
            // Empty string → auto-generate
            let default_dir = cfg.base_dir.join("agent_workspace");
            let _ = std::fs::create_dir_all(&default_dir);
            Some(default_dir.to_string_lossy().to_string())
        }
    } else {
        // No root provided → auto-generate default workspace inside app data
        let default_dir = cfg.base_dir.join("agent_workspace");
        let _ = std::fs::create_dir_all(&default_dir);
        Some(default_dir.to_string_lossy().to_string())
    };

    let display_root = resolved_root.clone().unwrap_or_else(|| "none".to_string());

    info!(
        "[openclaw] Setting workspace mode to: {} (root: {})",
        mode, display_root
    );
    cfg.workspace_mode = mode;
    cfg.workspace_root = resolved_root;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[openclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect the change
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

    let result_path = cfg
        .workspace_root
        .clone()
        .unwrap_or_else(|| "none".to_string());
    *state.config.write().await = Some(cfg);

    Ok(result_path)
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
