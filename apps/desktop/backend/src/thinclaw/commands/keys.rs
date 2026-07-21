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
// ws_rpc removed — ThinClaw is in-process, no remote WS gateway
use super::ThinClawManager;
use crate::sidecar::SidecarManager;
use crate::thinclaw::bridge::{gated, BridgeError, RouteMode};
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

type BedrockCredentials = (Option<String>, Option<String>, Option<String>);

async fn remote_secret_reads_are_opaque(ironclaw: &ThinClawRuntimeState) -> bool {
    ironclaw.remote_proxy().await.is_some()
}

async fn save_remote_provider_key_if_needed(
    ironclaw: &ThinClawRuntimeState,
    provider_slug: &str,
    key: Option<&str>,
) -> Result<bool, String> {
    let Some(proxy) = ironclaw.remote_proxy().await else {
        return Ok(false);
    };

    let trimmed = key.unwrap_or("").trim();
    let provider_slug = normalize_provider_slug(provider_slug);
    if trimmed.is_empty() {
        proxy
            .delete_provider_key(&provider_slug)
            .await
            .map_err(|err| format!("remote provider key delete failed: {}", err))?;
    } else {
        proxy
            .save_provider_key(&provider_slug, trimmed)
            .await
            .map_err(|err| format!("remote provider key save failed: {}", err))?;
    }
    Ok(true)
}

/// Get OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_openai_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok(None);
    }
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openai_api_key.clone()))
}

/// Save OpenAI API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_openai_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    key: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if save_remote_provider_key_if_needed(&ironclaw, "openai", key.as_deref()).await? {
        return Ok(());
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openai_key(key);

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_openrouter_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok(None);
    }
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.openrouter_api_key.clone()))
}

/// Save OpenRouter API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_openrouter_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    key: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if save_remote_provider_key_if_needed(&ironclaw, "openrouter", key.as_deref()).await? {
        return Ok(());
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_openrouter_key(key);

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Get Gemini API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_gemini_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok(None);
    }
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.gemini_api_key.clone()))
}

/// Save Gemini API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_gemini_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    key: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if save_remote_provider_key_if_needed(&ironclaw, "gemini", key.as_deref()).await? {
        return Ok(());
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_gemini_key(key);

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get Groq API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_groq_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok(None);
    }
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.groq_api_key.clone()))
}

/// Save Groq API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_groq_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    key: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if save_remote_provider_key_if_needed(&ironclaw, "groq", key.as_deref()).await? {
        return Ok(());
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let result = cfg.update_groq_key(key);

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Get Anthropic API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_anthropic_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok(None);
    }
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.anthropic_api_key.clone()))
}

/// Get Brave Search API key
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_brave_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok(None);
    }
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.brave_search_api_key.clone()))
}

/// Save Slack configuration
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_anthropic_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    key: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    if save_remote_provider_key_if_needed(&ironclaw, "anthropic", key.as_deref()).await? {
        return Ok(());
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Remote WS gateway path removed — ThinClaw is in-process

    println!(
        "[thinclaw] save_anthropic_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    // Update config structure on disk
    let result = cfg.update_anthropic_key(key);

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
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
pub async fn thinclaw_save_brave_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    key: Option<String>,
) -> Result<(), BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Err(gated(
            "Brave Search API key save",
            "saving the Brave Search API key has no ThinClaw gateway endpoint in remote mode",
            "switch ThinClaw Desktop to embedded (local) mode to save the Brave Search API key",
            RouteMode::LocalOnly,
        ));
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Remote WS gateway path removed — ThinClaw is in-process

    println!(
        "[thinclaw] save_brave_key called with: {:?}",
        key.as_ref().map(|_| "REDACTED")
    );

    let result = cfg.update_brave_key(key);

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle secret access for ThinClaw
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_toggle_secret_access(
    state: State<'_, ThinClawManager>,
    secret: String,
    granted: bool,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    // Remote WS gateway path removed — ThinClaw is in-process

    let result = cfg.toggle_secret_access(&secret, granted);

    // Regenerate config to reflect access change
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Select the cloud brain to use for the agent
#[tauri::command]
#[specta::specta]
pub async fn select_thinclaw_brain(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    brain: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
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
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Save HuggingFace token
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_set_hf_token(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    token: String,
) -> Result<(), BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Err(gated(
            "Hugging Face token save",
            "saving the Hugging Face token has no ThinClaw gateway endpoint in remote mode",
            "switch ThinClaw Desktop to embedded (local) mode to save the Hugging Face token",
            RouteMode::LocalOnly,
        ));
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[thinclaw] set_hf_token: attempting to set (empty: {})",
        token.trim().is_empty()
    );

    let val = if token.trim().is_empty() {
        None
    } else {
        Some(token.trim().to_string())
    };

    let result = cfg.update_huggingface_token(val);

    // Regenerate config/profiles
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;

    // Update in-memory state
    *state.config.write().await = Some(cfg);
    println!("[thinclaw] set_hf_token: successfully saved and updated state");

    Ok(())
}

/// Save an implicit cloud provider API key (generic)
/// Supports: xai, venice, together, moonshot, minimax, nvidia, qianfan, mistral,
/// cohere, voyage, deepgram, elevenlabs, stability, fal.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_implicit_provider_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    provider: String,
    key: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let valid_providers = [
        "xai",
        "venice",
        "together",
        "moonshot",
        "minimax",
        "nvidia",
        "qianfan",
        "mistral",
        "cohere",
        "voyage",
        "deepgram",
        "elevenlabs",
        "stability",
        "fal",
    ];
    if !valid_providers.contains(&provider.as_str()) {
        return Err(format!("Unknown implicit provider: {}", provider).into());
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
        return Ok(());
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[thinclaw] save_implicit_provider_key: {} (empty: {})",
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
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    result.map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    println!(
        "[thinclaw] save_implicit_provider_key: {} saved successfully",
        provider
    );
    Ok(())
}

/// Get an implicit cloud provider API key (generic)
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_implicit_provider_key(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    provider: String,
) -> Result<Option<String>, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok(None);
    }
    let config = state.get_config().await;
    Ok(config.and_then(|cfg| cfg.get_implicit_provider_key(&provider)))
}

/// Save Amazon Bedrock AWS credentials
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_bedrock_credentials(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
    access_key_id: String,
    secret_access_key: String,
    region: String,
) -> Result<(), BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        if access_key_id.trim().is_empty() && secret_access_key.trim().is_empty() {
            if let Some(proxy) = ironclaw.remote_proxy().await {
                proxy
                    .delete_provider_key("bedrock")
                    .await
                    .map_err(|err| format!("remote Bedrock key delete failed: {}", err))?;
                return Ok(());
            }
        }
        return Err(gated(
            "Bedrock AWS credential save",
            "saving Amazon Bedrock AWS credentials (access key id, secret access key, region) has no ThinClaw gateway endpoint in remote mode",
            "switch ThinClaw Desktop to embedded (local) mode to save Bedrock AWS credentials",
            RouteMode::LocalOnly,
        ));
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    println!(
        "[thinclaw] save_bedrock_credentials: ak={} sk=*** region={}",
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
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    println!("[thinclaw] save_bedrock_credentials: saved successfully");
    Ok(())
}

/// Get Amazon Bedrock credentials
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_get_bedrock_credentials(
    state: State<'_, ThinClawManager>,
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<BedrockCredentials, crate::thinclaw::bridge::BridgeError> {
    if remote_secret_reads_are_opaque(&ironclaw).await {
        return Ok((None, None, None));
    }
    let config = state.get_config().await;
    Ok(config
        .map(|cfg| cfg.get_bedrock_credentials())
        .unwrap_or((None, None, None)))
}

/// Add a custom secret
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_add_custom_secret(
    state: State<'_, ThinClawManager>,
    name: String,
    value: String,
    description: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let name = name.trim().to_string();
    if name.is_empty()
        || name.len() > 256
        || name.chars().any(char::is_control)
        || value.is_empty()
        || value.len() > 64 * 1024
        || value.contains('\0')
        || description.as_deref().is_some_and(|description| {
            description.len() > 4_096
                || description.contains('\0')
                || description.chars().any(|character| {
                    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
                })
        })
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "custom secret metadata or value is empty, malformed, or oversized"
                .to_string(),
        });
    }
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };
    if cfg.custom_secrets.len() >= 128 {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "custom secret limit of 128 has been reached".to_string(),
        });
    }

    let id = format!("custom-{}", uuid::Uuid::new_v4());

    // Store the secret value in the Keychain, not in identity.json
    crate::thinclaw::config::keychain::set_key(&id, Some(&value))
        .map_err(|e| format!("Keychain error: {}", e))?;

    cfg.custom_secrets.push(CustomSecret {
        id: id.clone(),
        name,
        value, // kept in memory only; #[serde(skip)] prevents JSON persistence
        description,
        granted: false,
    });

    if let Err(error) = cfg.save_identity() {
        let rollback = crate::thinclaw::config::keychain::set_key(&id, None);
        return match rollback {
            Ok(()) => Err(crate::thinclaw::bridge::BridgeError::Runtime { message: error.to_string() }),
            Err(rollback_error) => Err(format!(
                "failed to persist custom secret ({error}); credential rollback also failed: {rollback_error}"
            ).into()),
        };
    }

    // Regenerate config to reflect changes
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Update an existing custom secret value without changing its identity or grant.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_update_custom_secret(
    state: State<'_, ThinClawManager>,
    secret_store: State<'_, crate::secret_store::SecretStore>,
    id: String,
    value: String,
) -> Result<(), BridgeError> {
    if id.is_empty()
        || id.len() > 256
        || id.chars().any(char::is_control)
        || value.is_empty()
        || value.len() > 64 * 1024
        || value.contains('\0')
    {
        return Err("custom secret identity or value is empty, malformed, or oversized".into());
    }

    let mut cfg = if let Some(config) = state.get_config().await {
        config
    } else {
        state.init_config().await?
    };
    let secret = cfg
        .custom_secrets
        .iter_mut()
        .find(|secret| secret.id == id)
        .ok_or_else(|| "Secret not found".to_string())?;
    let old_value = crate::thinclaw::config::keychain::get_key(&id)
        .or_else(|| (!secret.value.is_empty()).then(|| secret.value.clone()));

    crate::thinclaw::config::keychain::set_key(&id, Some(&value))
        .map_err(|error| format!("Keychain error: {error}"))?;
    secret.value = value;

    if let Err(error) = cfg.save_identity() {
        let rollback = crate::thinclaw::config::keychain::set_key(&id, old_value.as_deref());
        return match rollback {
            Ok(()) => Err(error.to_string().into()),
            Err(rollback_error) => Err(format!(
                "failed to persist custom secret update ({error}); credential rollback also failed: {rollback_error}"
            )
            .into()),
        };
    }

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|model| model.get_local_llm_config());
    let thinclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|error| error.to_string())?;

    secret_store.apply_thinclaw_config(&cfg);
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Remove a custom secret
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_remove_custom_secret(
    state: State<'_, ThinClawManager>,
    id: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let secret_index = cfg
        .custom_secrets
        .iter()
        .position(|secret| secret.id == id)
        .ok_or_else(|| "Secret not found".to_string())?;
    let old_value = crate::thinclaw::config::keychain::get_key(&id)
        .or_else(|| Some(cfg.custom_secrets[secret_index].value.clone()))
        .filter(|value| !value.is_empty());
    crate::thinclaw::config::keychain::set_key(&id, None)
        .map_err(|error| format!("failed to remove custom secret credential: {error}"))?;
    cfg.custom_secrets.remove(secret_index);

    if let Err(error) = cfg.save_identity() {
        let rollback = crate::thinclaw::config::keychain::set_key(&id, old_value.as_deref());
        return match rollback {
            Ok(()) => Err(crate::thinclaw::bridge::BridgeError::Runtime { message: error.to_string() }),
            Err(rollback_error) => Err(format!(
                "failed to persist custom secret removal ({error}); credential rollback also failed: {rollback_error}"
            ).into()),
        };
    }

    // Regenerate config to reflect changes
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle custom secret access for ThinClaw
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_toggle_custom_secret(
    state: State<'_, ThinClawManager>,
    id: String,
    granted: bool,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
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
    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());

    let thinclaw_engine = cfg.generate_config(None, None, local_llm.clone());
    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Toggle local dev tools (shell, write_file, read_file, etc.) for ThinClaw
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_toggle_local_tools(
    state: State<'_, ThinClawManager>,
    sidecar: State<'_, SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    enabled: bool,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[thinclaw] Toggling local tools to: {}", enabled);
    cfg.allow_local_tools = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[thinclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect the change
    let existing_thinclaw_engine = cfg.load_config().ok();
    let snapshot = crate::engine::local_runtime_snapshot(&sidecar, &engine_manager).await;
    let local_llm = crate::engine::local_runtime_snapshot_to_local_llm(&snapshot);
    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&thinclaw_engine, local_llm).map_err(|e| {
        let err = format!("Failed to write thinclaw_engine config: {}", e);
        error!("[thinclaw] {}", err);
        err
    })?;

    *state.config.write().await = Some(cfg);

    Ok(())
}

/// Set workspace mode and optional root directory for the agent
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_set_workspace_mode(
    state: State<'_, ThinClawManager>,
    sidecar: State<'_, SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    mode: String,
    root: Option<String>,
) -> Result<String, crate::thinclaw::bridge::BridgeError> {
    // Validate mode
    if !matches!(mode.as_str(), "unrestricted" | "sandboxed" | "project") {
        return Err(format!(
            "Invalid workspace mode '{}'. Must be 'unrestricted', 'sandboxed', or 'project'.",
            mode
        )
        .into());
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
                return Err(crate::thinclaw::bridge::BridgeError::Runtime {
                    message: "Workspace root must be an absolute path.".to_string(),
                });
            }
            if let Err(e) = std::fs::create_dir_all(path) {
                return Err(format!("Failed to create workspace directory: {}", e).into());
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
        "[thinclaw] Setting workspace mode to: {} (root: {})",
        mode, display_root
    );
    cfg.workspace_mode = mode;
    cfg.workspace_root = resolved_root;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[thinclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect the change
    let existing_thinclaw_engine = cfg.load_config().ok();
    let snapshot = crate::engine::local_runtime_snapshot(&sidecar, &engine_manager).await;
    let local_llm = crate::engine::local_runtime_snapshot_to_local_llm(&snapshot);
    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&thinclaw_engine, local_llm).map_err(|e| {
        let err = format!("Failed to write thinclaw_engine config: {}", e);
        error!("[thinclaw] {}", err);
        err
    })?;

    let result_path = cfg
        .workspace_root
        .clone()
        .unwrap_or_else(|| "none".to_string());
    *state.config.write().await = Some(cfg);

    Ok(result_path)
}

/// Toggle local inference (exposing local LLM) for ThinClaw
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_toggle_local_inference(
    state: State<'_, ThinClawManager>,
    sidecar: State<'_, SidecarManager>,
    engine_manager: State<'_, crate::engine::EngineManager>,
    enabled: bool,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    info!("[thinclaw] Toggling local inference to: {}", enabled);
    cfg.local_inference_enabled = enabled;
    cfg.save_identity().map_err(|e| {
        let err = format!("Failed to save identity: {}", e);
        error!("[thinclaw] {}", err);
        err
    })?;

    // Regenerate config to reflect priority change
    let existing_thinclaw_engine = cfg.load_config().ok();
    let snapshot = crate::engine::local_runtime_snapshot(&sidecar, &engine_manager).await;
    let local_llm = crate::engine::local_runtime_snapshot_to_local_llm(&snapshot);
    let thinclaw_engine = cfg.generate_config(
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.slack.clone()),
        existing_thinclaw_engine
            .as_ref()
            .map(|m| m.channels.telegram.clone()),
        local_llm.clone(),
    );

    cfg.write_config(&thinclaw_engine, local_llm).map_err(|e| {
        let err = format!("Failed to write thinclaw_engine config: {}", e);
        error!("[thinclaw] {}", err);
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
pub async fn thinclaw_save_slack_config(
    state: State<'_, ThinClawManager>,
    config_input: SlackConfigInput,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());
    let mut thinclaw_engine = existing_thinclaw_engine
        .unwrap_or_else(|| cfg.generate_config(None, None, local_llm.clone()));

    thinclaw_engine.channels.slack = SlackConfig {
        enabled: config_input.enabled,
        bot_token: config_input.bot_token,
        app_token: config_input.app_token,
        ..Default::default()
    };

    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;
    info!("Saved Slack config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Telegram configuration
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_telegram_config(
    state: State<'_, ThinClawManager>,
    config_input: TelegramConfigInput,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let cfg = state.get_config().await.ok_or("Config not initialized")?;

    let existing_thinclaw_engine = cfg.load_config().ok();
    let local_llm = existing_thinclaw_engine
        .as_ref()
        .and_then(|m| m.get_local_llm_config());
    let mut thinclaw_engine = existing_thinclaw_engine
        .unwrap_or_else(|| cfg.generate_config(None, None, local_llm.clone()));

    thinclaw_engine.channels.telegram = TelegramConfig {
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

    cfg.write_config(&thinclaw_engine, local_llm)
        .map_err(|e| e.to_string())?;
    info!("Saved Telegram config, enabled: {}", config_input.enabled);

    Ok(())
}

/// Save Gateway configuration
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_save_gateway_settings(
    state: State<'_, ThinClawManager>,
    mode: String,
    url: Option<String>,
    token: Option<String>,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
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
pub async fn thinclaw_add_agent_profile(
    state: State<'_, ThinClawManager>,
    mut profile: AgentProfile,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    profile.id = profile.id.trim().to_string();
    profile.name = profile.name.trim().to_string();
    profile.url = profile.url.trim().trim_end_matches('/').to_string();
    if profile.id.is_empty()
        || profile.id.len() > 64
        || !profile
            .id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "agent profile ID must be 1-64 ASCII letters, digits, '.', '_' or '-'"
                .to_string(),
        });
    }
    if profile.name.is_empty()
        || profile.name.len() > 128
        || profile.name.chars().any(char::is_control)
    {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "agent profile name must be 1-128 printable characters".to_string(),
        });
    }
    if !matches!(profile.mode.as_str(), "local" | "remote") {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "agent profile mode must be 'local' or 'remote'".to_string(),
        });
    }
    if profile.url.len() > 2048 {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "agent profile URL exceeds the 2048-byte limit".to_string(),
        });
    }
    if profile.url.chars().any(char::is_control) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "agent profile URL contains control characters".to_string(),
        });
    }

    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };
    let replacing_existing = cfg
        .profiles
        .iter()
        .any(|existing| existing.id == profile.id);
    if !replacing_existing && cfg.profiles.len() >= 64 {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "agent profile limit of 64 has been reached".to_string(),
        });
    }

    let token_key = crate::thinclaw::config::keychain::profile_token_key(&profile.id);
    let old_stored_token = crate::thinclaw::config::keychain::get_key(&token_key).or_else(|| {
        cfg.profiles
            .iter()
            .find(|existing| existing.id == profile.id)
            .and_then(|existing| existing.token.clone())
    });
    let supplied_token = profile.token.take();
    let resolved_token = match supplied_token.as_deref() {
        None => old_stored_token.clone(),
        Some(token) if token.trim().is_empty() => None,
        Some(token) => Some(token.trim().to_string()),
    };
    if resolved_token.as_deref().is_some_and(|token| {
        token.is_empty() || token.len() > 8 * 1024 || token.chars().any(char::is_control)
    }) {
        return Err(crate::thinclaw::bridge::BridgeError::Runtime {
            message: "agent profile token is malformed or exceeds 8192 bytes".to_string(),
        });
    }

    if profile.mode == "remote" {
        let token = resolved_token
            .as_deref()
            .ok_or_else(|| "remote agent profiles require a bearer token".to_string())?;
        crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(&profile.url, token)
            .map_err(|error| format!("invalid remote agent profile: {error}"))?;
    }
    profile.token = resolved_token.clone();

    if supplied_token.is_some() || old_stored_token.is_none() && resolved_token.is_some() {
        crate::thinclaw::config::keychain::set_key(&token_key, resolved_token.as_deref())
            .map_err(|error| format!("failed to secure agent profile token: {error}"))?;
    }

    if let Some(existing) = cfg
        .profiles
        .iter_mut()
        .find(|existing| existing.id == profile.id)
    {
        *existing = profile;
    } else {
        cfg.profiles.push(profile);
    }

    if let Err(error) = cfg.save_identity() {
        let rollback =
            crate::thinclaw::config::keychain::set_key(&token_key, old_stored_token.as_deref());
        return match rollback {
            Ok(()) => Err(crate::thinclaw::bridge::BridgeError::Runtime { message: error.to_string() }),
            Err(rollback_error) => Err(format!(
                "failed to persist agent profile ({error}); credential rollback also failed: {rollback_error}"
            ).into()),
        };
    }
    *state.config.write().await = Some(cfg);
    Ok(())
}

/// Remove an agent profile
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_remove_agent_profile(
    state: State<'_, ThinClawManager>,
    id: String,
) -> Result<(), crate::thinclaw::bridge::BridgeError> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let profile_index = cfg
        .profiles
        .iter()
        .position(|profile| profile.id == id)
        .ok_or_else(|| "agent profile not found".to_string())?;
    let token_key = crate::thinclaw::config::keychain::profile_token_key(&id);
    let old_token = crate::thinclaw::config::keychain::get_key(&token_key)
        .or_else(|| cfg.profiles[profile_index].token.clone());
    crate::thinclaw::config::keychain::set_key(&token_key, None)
        .map_err(|error| format!("failed to remove agent profile token: {error}"))?;

    cfg.profiles.remove(profile_index);

    if let Err(error) = cfg.save_identity() {
        let rollback = crate::thinclaw::config::keychain::set_key(&token_key, old_token.as_deref());
        return match rollback {
            Ok(()) => Err(crate::thinclaw::bridge::BridgeError::Runtime { message: error.to_string() }),
            Err(rollback_error) => Err(format!(
                "failed to persist profile removal ({error}); credential rollback also failed: {rollback_error}"
            ).into()),
        };
    }
    *state.config.write().await = Some(cfg);
    Ok(())
}
