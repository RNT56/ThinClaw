use std::sync::Arc;

use axum::{extract::State, response::IntoResponse};

use super::server::GatewayState;

pub(crate) async fn index_handler(State(state): State<Arc<GatewayState>>) -> impl IntoResponse {
    let webchat = load_webchat_config(state.as_ref()).await;
    thinclaw_gateway::web::static_files::render_index_response(&gateway_webchat_config(&webchat))
}

pub(crate) async fn load_webchat_config(state: &GatewayState) -> crate::config::WebChatConfig {
    if let Some(store) = state.store.as_ref()
        && let Ok(map) = store.get_all_settings(&state.user_id).await
    {
        let settings = crate::settings::Settings::from_db_map(&map);
        return crate::config::WebChatConfig::from_settings(&settings);
    }

    crate::config::WebChatConfig::from_env()
}

#[cfg(test)]
pub(crate) fn render_index_html(webchat: &crate::config::WebChatConfig) -> String {
    thinclaw_gateway::web::static_files::render_index_html(&gateway_webchat_config(webchat))
}

fn gateway_webchat_config(
    webchat: &crate::config::WebChatConfig,
) -> thinclaw_gateway::web::static_files::WebChatRuntimeConfig {
    let theme = match webchat.theme {
        crate::config::WebChatTheme::Light => {
            thinclaw_gateway::web::static_files::WebChatTheme::Light
        }
        crate::config::WebChatTheme::Dark => {
            thinclaw_gateway::web::static_files::WebChatTheme::Dark
        }
        crate::config::WebChatTheme::System => {
            thinclaw_gateway::web::static_files::WebChatTheme::System
        }
    };
    thinclaw_gateway::web::static_files::WebChatRuntimeConfig {
        theme,
        show_branding: webchat.show_branding,
        runtime_css: webchat.runtime_css(),
        bootstrap_payload: serde_json::to_value(webchat.bootstrap_payload())
            .unwrap_or(serde_json::Value::Null),
    }
}

#[cfg(test)]
fn escape_json_for_html(value: &str) -> String {
    value.replace("</", "<\\/")
}

pub(crate) async fn css_handler() -> impl IntoResponse {
    thinclaw_gateway::web::static_files::css_handler().await
}

pub(crate) async fn js_handler() -> impl IntoResponse {
    thinclaw_gateway::web::static_files::js_handler().await
}

pub(crate) async fn favicon_handler() -> impl IntoResponse {
    thinclaw_gateway::web::static_files::favicon_handler().await
}

pub(crate) async fn apple_touch_icon_handler() -> impl IntoResponse {
    thinclaw_gateway::web::static_files::apple_touch_icon_handler().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_index_html_includes_bootstrap_payload() {
        let html = render_index_html(&crate::config::WebChatConfig::default());
        assert!(html.contains("webchat-bootstrap"));
        assert!(html.contains("availableSkins"));
        assert!(html.contains("resolvedSkin"));
    }

    #[test]
    fn escape_json_for_html_escapes_script_closers() {
        let escaped = escape_json_for_html("{\"x\":\"</script>\"}");
        assert!(escaped.contains("<\\/script>"));
    }

    #[test]
    fn cost_dashboard_script_displays_provider_and_token_capture_provenance() {
        let js = include_str!("static/app.js");
        assert!(js.contains("provider-usage requests"));
        assert!(js.contains("provider cost"));
        assert!(js.contains("priced locally"));
        assert!(js.contains("token-capture requests"));
        assert!(js.contains("token ids"));
        assert!(js.contains("logprobs"));
    }
}
