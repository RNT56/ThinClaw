use crate::openclaw::OpenClawManager;
use tauri::State;
use tokio_tungstenite::connect_async;
use tungstenite::client::IntoClientRequest;

/// Switch active gateway to a specific profile
#[tauri::command]
#[specta::specta]
pub async fn openclaw_switch_to_profile(
    state: State<'_, OpenClawManager>,
    _sidecar: State<'_, crate::sidecar::SidecarManager>,
    profile_id: String,
) -> Result<(), String> {
    let mut cfg = if let Some(c) = state.get_config().await {
        c
    } else {
        state.init_config().await?
    };

    let profile = cfg
        .profiles
        .iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| "Profile not found".to_string())?
        .clone();

    // Preserve the profile connection details into the gateway settings
    if profile.mode == "local" {
        cfg.gateway_mode = "local".to_string();
        cfg.remote_url = None;
        cfg.remote_token = None;
    } else {
        cfg.gateway_mode = "remote".to_string();
        cfg.remote_url = Some(profile.url);
        cfg.remote_token = profile.token;
    }

    cfg.save_identity().map_err(|e| e.to_string())?;
    *state.config.write().await = Some(cfg);

    // IronClaw is in-process — no gateway restart needed
    Ok(())
}

/// Test connection to a potential gateway
#[tauri::command]
#[specta::specta]
pub async fn openclaw_test_connection(url: String, token: Option<String>) -> Result<bool, String> {
    // Basic validation
    if url.trim().is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    let mut request = url
        .into_client_request()
        .map_err(|e| format!("Invalid URL: {}", e))?;

    if let Some(tok) = token {
        if !tok.trim().is_empty() {
            let headers = request.headers_mut();
            headers.insert(
                "Authorization",
                format!("Bearer {}", tok)
                    .parse()
                    .map_err(|_| "Invalid token format")?,
            );
        }
    }

    // Set a short timeout
    let attempt =
        tokio::time::timeout(tokio::time::Duration::from_secs(5), connect_async(request)).await;

    match attempt {
        Ok(Ok((_ws_stream, _response))) => Ok(true),
        Ok(Err(e)) => Err(format!("Connection failed: {}", e)),
        Err(_) => Err("Connection timed out".to_string()),
    }
}
