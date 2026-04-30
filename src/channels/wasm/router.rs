//! Compatibility adapter for WASM channel webhook routing.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};

pub use thinclaw_channels::wasm::router::{
    RegisteredEndpoint, RegisteredWebhookAuth, RouterState, WasmChannelRouter,
};

#[derive(Clone)]
struct OAuthRouterState {
    extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
}

async fn oauth_callback_handler(
    State(state): State<OAuthRouterState>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let code = params.get("code").cloned().unwrap_or_default();
    let oauth_state = params.get("state").cloned().unwrap_or_default();

    if code.is_empty() {
        let error = params
            .get("error")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        return (
            StatusCode::BAD_REQUEST,
            axum::response::Html(format!(
                "<!DOCTYPE html><html><body style=\"font-family: sans-serif; \
                 display: flex; justify-content: center; align-items: center; \
                 height: 100vh; margin: 0; background: #191919; color: white;\">\
                 <div style=\"text-align: center;\">\
                 <h1>Authorization Failed</h1>\
                 <p>Error: {}</p>\
                 </div></body></html>",
                error
            )),
        );
    }

    if !oauth_state.is_empty() {
        if let Some(ref ext_mgr) = state.extension_manager {
            let is_valid = ext_mgr.validate_pending_auth_nonce(&oauth_state).await;
            if !is_valid {
                tracing::warn!("OAuth callback: invalid or expired state nonce");
                return (
                    StatusCode::BAD_REQUEST,
                    axum::response::Html(crate::cli::oauth_defaults::landing_html(
                        "ThinClaw", false,
                    )),
                );
            }

            match ext_mgr.complete_oauth_callback(&oauth_state, &code).await {
                Ok((extension_name, _thread_id, _auth_result, activate_result)) => {
                    tracing::info!(
                        extension = %extension_name,
                        tools = activate_result.tools_loaded.len(),
                        "OAuth callback completed and tool activated"
                    );
                    return (
                        StatusCode::OK,
                        axum::response::Html(crate::cli::oauth_defaults::landing_html(
                            &extension_name,
                            true,
                        )),
                    );
                }
                Err(error) => {
                    tracing::warn!(error = %error, "OAuth callback completion failed");
                    return (
                        StatusCode::BAD_REQUEST,
                        axum::response::Html(crate::cli::oauth_defaults::landing_html(
                            "ThinClaw", false,
                        )),
                    );
                }
            }
        }
    }

    (
        StatusCode::BAD_REQUEST,
        axum::response::Html(crate::cli::oauth_defaults::landing_html("ThinClaw", false)),
    )
}

pub fn create_wasm_channel_router(
    router: Arc<WasmChannelRouter>,
    extension_manager: Option<Arc<crate::extensions::ExtensionManager>>,
) -> Router {
    thinclaw_channels::wasm::router::create_wasm_channel_router(router).merge(
        Router::new()
            .route("/oauth/callback", get(oauth_callback_handler))
            .with_state(OAuthRouterState { extension_manager }),
    )
}
