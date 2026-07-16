//! Channel-status and Gmail dashboard RPC commands, including the remote
//! gateway channel/Gmail status mappers.

use tauri::State;
use tracing::{info, warn};
use zeroize::Zeroizing;

use super::helpers::{json_bool_field, json_string_vec_field, setting_value};
use crate::thinclaw::commands::types::*;
use crate::thinclaw::remote_proxy::RemoteGatewayProxy;
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

fn remote_channel_status_entries(status: &serde_json::Value) -> Vec<ChannelStatusEntry> {
    let Some(setup) = status
        .get("channel_setup")
        .and_then(|value| value.as_object())
    else {
        return Vec::new();
    };

    [
        ("slack", "Slack", "wasm"),
        ("telegram", "Telegram", "wasm"),
        ("gmail", "Gmail", "builtin"),
        ("apple_mail", "Apple Mail", "native"),
        ("nostr", "Nostr", "native"),
        ("matrix", "Matrix", "native"),
        ("voice_call", "Voice Call", "native"),
        ("apns", "Apple Push", "native"),
        ("browser_push", "Browser Push", "native"),
    ]
    .into_iter()
    .filter_map(|(id, name, channel_type)| {
        setup
            .get(id)
            .map(|channel| remote_channel_status_entry(id, name, channel_type, channel))
    })
    .collect()
}

fn remote_channel_status_entry(
    id: &str,
    name: &str,
    channel_type: &str,
    setup: &serde_json::Value,
) -> ChannelStatusEntry {
    let enabled = json_bool_field(setup, "enabled");
    let configured = json_bool_field(setup, "configured");
    let missing_fields = json_string_vec_field(setup, "missing_fields");
    let needs_oauth = json_bool_field(setup, "needs_oauth");
    let invalid_private_key = json_bool_field(setup, "invalid_private_key");
    let connected_relays = setup
        .get("connected_relay_count")
        .and_then(|value| value.as_u64());

    let state = if enabled && configured {
        match connected_relays {
            Some(0) => "Degraded",
            _ => "Running",
        }
    } else if enabled {
        "Error"
    } else {
        "Disconnected"
    }
    .to_string();

    let last_error = if !missing_fields.is_empty() {
        Some(format!("missing fields: {}", missing_fields.join(", ")))
    } else if needs_oauth {
        Some("OAuth authorization required".to_string())
    } else if invalid_private_key {
        Some("invalid private key".to_string())
    } else {
        None
    };

    ChannelStatusEntry {
        id: id.to_string(),
        name: name.to_string(),
        channel_type: channel_type.to_string(),
        state,
        enabled,
        uptime_secs: None,
        messages_sent: 0,
        messages_received: 0,
        last_error,
        stream_mode: String::new(),
    }
}

async fn remote_gmail_status(
    proxy: &RemoteGatewayProxy,
    gateway_status: &serde_json::Value,
) -> Result<GmailStatusResponse, crate::thinclaw::bridge::BridgeError> {
    let gmail = gateway_status
        .get("channel_setup")
        .and_then(|value| value.get("gmail"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));

    async fn remote_setting_string(proxy: &RemoteGatewayProxy, key: &str) -> Option<String> {
        proxy
            .get_setting(key)
            .await
            .ok()
            .map(setting_value)
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    }

    let project_id = remote_setting_string(proxy, "channels.gmail_project_id")
        .await
        .unwrap_or_default();
    let subscription_id = remote_setting_string(proxy, "channels.gmail_subscription_id")
        .await
        .unwrap_or_default();
    let allowed_senders = remote_setting_string(proxy, "channels.gmail_allowed_senders")
        .await
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let enabled = json_bool_field(&gmail, "enabled");
    let configured = json_bool_field(&gmail, "configured");
    let missing_fields = json_string_vec_field(&gmail, "missing_fields");
    let oauth_configured = enabled && !json_bool_field(&gmail, "needs_oauth");

    let status = if !enabled {
        "disabled".to_string()
    } else if configured {
        if subscription_id.is_empty() {
            "ready".to_string()
        } else {
            format!("ready ({})", subscription_id)
        }
    } else if json_bool_field(&gmail, "needs_oauth") {
        "configured but OAuth not completed".to_string()
    } else if !missing_fields.is_empty() {
        format!("missing credentials: {}", missing_fields.join(", "))
    } else {
        "unavailable: remote gateway did not report Gmail setup details".to_string()
    };

    Ok(GmailStatusResponse {
        enabled,
        configured,
        status,
        project_id,
        subscription_id,
        label_filters: Vec::new(),
        allowed_senders,
        missing_fields,
        oauth_configured,
    })
}

/// List channel statuses from the live ThinClaw runtime.
///
/// Queries the agent's ChannelManager for actually registered channels
/// instead of reading static config/env vars.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_channel_status_list(
    ironclaw: State<'_, ThinClawRuntimeState>,
) -> Result<Vec<ChannelStatusEntry>, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status = proxy.get_status().await?;
        let entries = remote_channel_status_entries(&status);
        if entries.is_empty() {
            return Err(
                "unavailable: remote ThinClaw gateway did not include channel setup status"
                    .to_string()
                    .into(),
            );
        }
        return Ok(entries);
    }

    let agent = ironclaw.agent().await?;
    let channel_mgr = agent.channels();
    let ic_entries = channel_mgr.status_entries().await;

    let entries: Vec<ChannelStatusEntry> = ic_entries
        .into_iter()
        .map(|e| {
            let (state_str, uptime) = match &e.state {
                thinclaw_core::channels::status_view::ChannelViewState::Running { uptime_secs } => {
                    ("Running".to_string(), Some(*uptime_secs as u32))
                }
                thinclaw_core::channels::status_view::ChannelViewState::Connecting { attempt } => {
                    (format!("Connecting (attempt {})", attempt), None)
                }
                thinclaw_core::channels::status_view::ChannelViewState::Reconnecting {
                    attempt,
                    ..
                } => (format!("Reconnecting (attempt {})", attempt), None),
                thinclaw_core::channels::status_view::ChannelViewState::Failed {
                    error, ..
                } => (format!("Failed: {}", error), None),
                thinclaw_core::channels::status_view::ChannelViewState::Disabled => {
                    ("Disabled".to_string(), None)
                }
                thinclaw_core::channels::status_view::ChannelViewState::Draining => {
                    ("Draining".to_string(), None)
                }
            };
            ChannelStatusEntry {
                id: e.name.to_lowercase().replace(' ', "_"),
                name: e.name,
                channel_type: e.channel_type,
                state: state_str,
                enabled: e.state.is_healthy(),
                uptime_secs: uptime,
                messages_sent: e.messages_sent as u32,
                messages_received: e.messages_received as u32,
                last_error: e.last_error,
                stream_mode: String::new(),
            }
        })
        .collect();

    Ok(entries)
}

/// Start the Gmail OAuth PKCE flow via ThinClaw.
///
/// This opens the user's browser for Google consent, waits for the
/// callback, exchanges the auth code for tokens, and returns them.
/// On success, the tokens are also stored in the Keychain.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_gmail_oauth_start(
    ironclaw: State<'_, ThinClawRuntimeState>,
    secret_store: State<'_, crate::secret_store::SecretStore>,
) -> Result<GmailOAuthResult, crate::thinclaw::bridge::BridgeError> {
    if ironclaw.remote_proxy().await.is_some() {
        return Ok(GmailOAuthResult {
            success: false,
            access_token: None,
            refresh_token: None,
            expires_in: None,
            scope: None,
            error: Some(
                "unavailable: remote Gmail OAuth must be completed on the gateway host".to_string(),
            ),
        });
    }

    // Call ThinClaw's gmail_oauth_start which handles the full PKCE flow:
    // 1. Generates PKCE verifier/challenge
    // 2. Builds Google auth URL
    // 3. Opens browser
    // 4. Binds localhost callback listener
    // 5. Exchanges code for tokens
    let mut ic_result = thinclaw_core::desktop_api::gmail_oauth_start()
        .await
        .map_err(|e| format!("Gmail OAuth failed: {}", e))?;

    let access_token = ic_result.access_token.take().map(Zeroizing::new);
    let refresh_token = ic_result.refresh_token.take().map(Zeroizing::new);

    // Persist OAuth credentials in the authenticated Keychain envelope. They
    // are intentionally not returned over broad IPC or written into the
    // runtime settings database.
    if ic_result.success {
        let previous_access = secret_store.get("gmail_oauth_token").map(Zeroizing::new);
        let previous_refresh = secret_store.get("gmail_refresh_token").map(Zeroizing::new);
        let persist_result = (|| {
            if let Some(access_token) = access_token.as_ref().map(|token| token.as_str()) {
                secret_store.set("gmail_oauth_token", Some(access_token))?;
            }
            if let Some(refresh_token) = refresh_token.as_ref().map(|token| token.as_str()) {
                secret_store.set("gmail_refresh_token", Some(refresh_token))?;
            }
            Ok::<(), crate::thinclaw::bridge::BridgeError>(())
        })();
        if let Err(persist_error) = persist_result {
            let access_rollback = secret_store
                .set(
                    "gmail_oauth_token",
                    previous_access.as_ref().map(|token| token.as_str()),
                )
                .err();
            let refresh_rollback = secret_store
                .set(
                    "gmail_refresh_token",
                    previous_refresh.as_ref().map(|token| token.as_str()),
                )
                .err();
            if access_rollback.is_some() || refresh_rollback.is_some() {
                return Err((format!(
                    "Gmail OAuth credential save failed ({persist_error}); rollback also failed (access={access_rollback:?}, refresh={refresh_rollback:?})"
                )).into());
            }
            return Err((format!("Gmail OAuth credential save failed: {persist_error}")).into());
        }
        // Clean up the legacy plaintext setting if an older build created it.
        if let Ok(agent) = ironclaw.agent().await {
            if let Some(store) = agent.store() {
                let _ = store
                    .delete_setting("local_user", "gmail_refresh_token")
                    .await;
            }
        }
        info!("[thinclaw-runtime] Gmail OAuth completed successfully");
    } else {
        let err_msg = ic_result.error.as_deref().unwrap_or("unknown error");
        warn!("[thinclaw-runtime] Gmail OAuth failed: {}", err_msg);
    }

    let result = GmailOAuthResult {
        success: ic_result.success,
        access_token: None,
        refresh_token: None,
        expires_in: ic_result.expires_in.map(|e| e as u32),
        scope: ic_result.scope.take(),
        error: ic_result.error.take(),
    };
    Ok(result)
}

/// Get Gmail channel configuration status.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_gmail_status(
    ironclaw: State<'_, ThinClawRuntimeState>,
    secret_store: State<'_, crate::secret_store::SecretStore>,
) -> Result<GmailStatusResponse, crate::thinclaw::bridge::BridgeError> {
    if let Some(proxy) = ironclaw.remote_proxy().await {
        let status = proxy.get_status().await?;
        return remote_gmail_status(&proxy, &status).await;
    }

    let mut enabled = false;
    let mut project_id = String::new();
    let mut subscription_id = String::new();
    let mut label_filters: Vec<String> = Vec::new();
    let mut allowed_senders: Vec<String> = Vec::new();
    let mut missing_fields: Vec<String> = Vec::new();

    // Read Gmail config from environment variables (ThinClaw pattern)
    if let Ok(val) = std::env::var("GMAIL_ENABLED") {
        enabled = val == "true" || val == "1";
    }
    if let Ok(val) = std::env::var("GMAIL_PROJECT_ID") {
        project_id = val;
    } else {
        missing_fields.push("GMAIL_PROJECT_ID".to_string());
    }
    if let Ok(val) = std::env::var("GMAIL_SUBSCRIPTION_ID") {
        subscription_id = val;
    } else {
        missing_fields.push("GMAIL_SUBSCRIPTION_ID".to_string());
    }
    if let Ok(val) = std::env::var("GMAIL_LABEL_FILTERS") {
        label_filters = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Ok(val) = std::env::var("GMAIL_ALLOWED_SENDERS") {
        allowed_senders = val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    // Fold in DB-backed ThinClaw channel settings when env vars are absent.
    if let Ok(agent) = ironclaw.agent().await {
        if let Some(store) = agent.store() {
            if std::env::var("GMAIL_ENABLED").is_err() {
                if let Ok(Some(value)) = store
                    .get_setting("local_user", "channels.gmail_enabled")
                    .await
                {
                    enabled = value.as_bool().unwrap_or(enabled);
                }
            }
            if project_id.is_empty() {
                if let Ok(Some(value)) = store
                    .get_setting("local_user", "channels.gmail_project_id")
                    .await
                {
                    project_id = value.as_str().unwrap_or_default().to_string();
                    missing_fields.retain(|field| field != "GMAIL_PROJECT_ID");
                }
            }
            if subscription_id.is_empty() {
                if let Ok(Some(value)) = store
                    .get_setting("local_user", "channels.gmail_subscription_id")
                    .await
                {
                    subscription_id = value.as_str().unwrap_or_default().to_string();
                    missing_fields.retain(|field| field != "GMAIL_SUBSCRIPTION_ID");
                }
            }
            if allowed_senders.is_empty() {
                if let Ok(Some(value)) = store
                    .get_setting("local_user", "channels.gmail_allowed_senders")
                    .await
                {
                    if let Some(raw) = value.as_str() {
                        allowed_senders = raw
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                }
            }
            if label_filters.is_empty() {
                if let Ok(Some(value)) = store
                    .get_setting("local_user", "channels.gmail_label_filters")
                    .await
                {
                    if let Some(raw) = value.as_str() {
                        label_filters = raw
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                }
            }
            if let Ok(Some(value)) = store.get_setting("local_user", "gmail_refresh_token").await {
                if let Some(token) = value.as_str().filter(|token| !token.trim().is_empty()) {
                    secret_store.set("gmail_refresh_token", Some(token))?;
                    store
                        .delete_setting("local_user", "gmail_refresh_token")
                        .await
                        .map_err(|error| {
                            format!("legacy Gmail credential cleanup failed: {error}")
                        })?;
                }
            }
        }
    }

    let oauth_configured =
        secret_store.has("gmail_oauth_token") || secret_store.has("gmail_refresh_token");

    let configured = !project_id.is_empty() && !subscription_id.is_empty();
    let status = if !enabled {
        "disabled".to_string()
    } else if !configured {
        format!("missing credentials: {}", missing_fields.join(", "))
    } else if oauth_configured {
        format!("ready ({})", subscription_id)
    } else {
        "configured but OAuth not completed".to_string()
    };

    Ok(GmailStatusResponse {
        enabled,
        configured,
        status,
        project_id,
        subscription_id,
        label_filters,
        allowed_senders,
        missing_fields,
        oauth_configured,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_channel_status_entries_map_gateway_setup_status() {
        let entries = remote_channel_status_entries(&serde_json::json!({
            "channel_setup": {
                "gmail": {
                    "enabled": true,
                    "configured": false,
                    "missing_fields": ["gmail_client_secret"],
                    "needs_oauth": true
                },
                "nostr": {
                    "enabled": true,
                    "configured": true,
                    "connected_relay_count": 0
                },
                "matrix": {
                    "enabled": false,
                    "configured": false
                }
            }
        }));

        let gmail = entries.iter().find(|entry| entry.id == "gmail").unwrap();
        assert_eq!(gmail.state, "Error");
        assert_eq!(
            gmail.last_error.as_deref(),
            Some("missing fields: gmail_client_secret")
        );

        let nostr = entries.iter().find(|entry| entry.id == "nostr").unwrap();
        assert_eq!(nostr.state, "Degraded");

        let matrix = entries.iter().find(|entry| entry.id == "matrix").unwrap();
        assert_eq!(matrix.state, "Disconnected");
    }

    #[test]
    fn remote_route_matrix_documents_p3_surfaces() {
        let matrix = include_str!("../../../../../documentation/remote-gateway-route-matrix.md");

        for expected in [
            "/api/providers/route/simulate",
            "/api/jobs/*",
            "/api/autonomy/*",
            "/api/experiments/*",
            "/api/learning/*",
            "unavailable:",
        ] {
            assert!(
                matrix.contains(expected),
                "remote route matrix should mention {expected}"
            );
        }
    }
}
