use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use nostr_sdk::{ToBech32, prelude::Keys};

use crate::channels::web::identity_helpers::GatewayRequestIdentity;
use crate::channels::web::server::GatewayState;

const NOSTR_SECRET_NAME: &str = "nostr_private_key";
const NOSTR_TOOL_NAME: &str = "nostr_actions";

#[derive(serde::Deserialize)]
pub(crate) struct NostrPrivateKeyRequest {
    #[serde(default)]
    private_key: Option<String>,
}

pub(crate) async fn reconcile_nostr_runtime(
    state: &GatewayState,
    user_id: &str,
) -> Result<(), String> {
    let settings = if let Some(store) = state.store.as_ref() {
        let map = store
            .get_all_settings(user_id)
            .await
            .map_err(|err| format!("failed to load settings: {err}"))?;
        crate::settings::Settings::from_db_map(&map)
    } else {
        crate::settings::Settings::default()
    };

    let nostr_config =
        crate::config::ChannelsConfig::resolve_nostr(&settings).map_err(|err| err.to_string())?;

    let mut next_runtime = None;
    let mut next_channel = None;
    if let Some(config) = nostr_config {
        let channel = crate::channels::NostrChannel::new(config).map_err(|err| err.to_string())?;
        next_runtime = Some(channel.runtime());
        next_channel = Some(channel);
    }

    if let Some(channel_manager) = state.channel_manager.as_ref() {
        let has_nostr = channel_manager
            .channel_names()
            .await
            .into_iter()
            .any(|name| name == "nostr");

        match next_channel {
            Some(channel) => {
                if has_nostr {
                    channel_manager
                        .hot_remove("nostr")
                        .await
                        .map_err(|err| format!("failed to replace active Nostr channel: {err}"))?;
                }
                channel_manager
                    .hot_add(Box::new(channel))
                    .await
                    .map_err(|err| format!("failed to activate Nostr channel: {err}"))?;
            }
            None if has_nostr => {
                channel_manager
                    .hot_remove("nostr")
                    .await
                    .map_err(|err| format!("failed to deactivate Nostr channel: {err}"))?;
            }
            None => {}
        }
    }

    if let Some(tool_registry) = state.tool_registry.as_ref() {
        let _ = tool_registry.unregister(NOSTR_TOOL_NAME).await;
        if let Some(runtime) = next_runtime {
            tool_registry
                .register(Arc::new(crate::tools::builtin::NostrActionsTool::new(
                    runtime,
                )))
                .await;
        }
    }

    Ok(())
}

pub(crate) async fn nostr_save_key_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
    Json(body): Json<NostrPrivateKeyRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let private_key = body.private_key.as_deref().unwrap_or("").trim().to_string();
    if private_key.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    let keys = Keys::parse(&private_key).map_err(|err| {
        tracing::warn!("Rejected invalid Nostr private key from WebUI: {}", err);
        StatusCode::BAD_REQUEST
    })?;

    let _ = secrets
        .delete(&request_identity.principal_id, NOSTR_SECRET_NAME)
        .await;

    let params = crate::secrets::CreateSecretParams::new(NOSTR_SECRET_NAME, private_key)
        .with_provider("nostr");
    secrets
        .create(&request_identity.principal_id, params)
        .await
        .map_err(|err| {
            tracing::error!("Failed to save Nostr private key: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let refreshed =
        crate::config::refresh_secrets(secrets.as_ref(), &request_identity.principal_id).await;
    tracing::info!(
        refreshed,
        "Nostr private key saved via WebUI and secrets overlay refreshed"
    );

    let public_key_hex = keys.public_key().to_hex();
    let public_key_npub = keys
        .public_key()
        .to_bech32()
        .unwrap_or_else(|_| public_key_hex.clone());

    if let Err(err) = reconcile_nostr_runtime(state.as_ref(), &request_identity.principal_id).await
    {
        tracing::warn!("Nostr runtime reconcile failed after secret save: {}", err);
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "partial_failure",
                "message": format!(
                    "The Nostr private key was saved, but the live Nostr runtime could not be reconciled: {}",
                    err
                ),
                "public_key_hex": public_key_hex,
                "public_key_npub": public_key_npub,
            })),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Nostr private key saved",
            "public_key_hex": public_key_hex,
            "public_key_npub": public_key_npub,
        })),
    ))
}

pub(crate) async fn nostr_delete_key_handler(
    State(state): State<Arc<GatewayState>>,
    request_identity: GatewayRequestIdentity,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let secrets = state
        .secrets_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    secrets
        .delete(&request_identity.principal_id, NOSTR_SECRET_NAME)
        .await
        .map_err(|err| {
            tracing::error!("Failed to delete Nostr private key: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let refreshed =
        crate::config::refresh_secrets(secrets.as_ref(), &request_identity.principal_id).await;
    tracing::info!(
        refreshed,
        "Nostr private key removed via WebUI and secrets overlay refreshed"
    );

    if let Err(err) = reconcile_nostr_runtime(state.as_ref(), &request_identity.principal_id).await
    {
        tracing::warn!(
            "Nostr runtime reconcile failed after secret delete: {}",
            err
        );
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "status": "partial_failure",
                "message": format!(
                    "The Nostr private key was removed, but the live Nostr runtime could not be reconciled: {}",
                    err
                ),
            })),
        ));
    }

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "message": "Nostr private key removed",
        })),
    ))
}
