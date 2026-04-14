use std::sync::Arc;

use axum::{extract::State, http::header, response::IntoResponse};

use super::server::GatewayState;

pub(crate) async fn index_handler(State(state): State<Arc<GatewayState>>) -> impl IntoResponse {
    let webchat = load_webchat_config(state.as_ref()).await;
    let html = render_index_html(&webchat);
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        html,
    )
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

pub(crate) fn render_index_html(webchat: &crate::config::WebChatConfig) -> String {
    let theme = match webchat.theme {
        crate::config::WebChatTheme::Light => "light",
        crate::config::WebChatTheme::Dark => "dark",
        crate::config::WebChatTheme::System => "system",
    };
    let branding = if webchat.show_branding {
        "true"
    } else {
        "false"
    };
    let mut html = include_str!("static/index.html").replace(
        "<html lang=\"en\">",
        &format!(
            "<html lang=\"en\" data-webchat-theme=\"{theme}\" data-show-branding=\"{branding}\">"
        ),
    );

    let runtime_css = webchat.runtime_css();
    if !runtime_css.is_empty() {
        html = html.replace(
            "</head>",
            &format!("  <style id=\"webchat-runtime-theme\">{runtime_css}</style>\n</head>"),
        );
    }

    let payload = webchat.bootstrap_payload();
    let payload_json = escape_json_for_html(
        &serde_json::to_string(&payload).expect("webchat bootstrap payload serializes"),
    );
    html.replace(
        "<script src=\"/app.js\"></script>",
        &format!(
            "<script id=\"webchat-bootstrap\" type=\"application/json\">{payload_json}</script>\n  <script src=\"/app.js\"></script>"
        ),
    )
}

fn escape_json_for_html(value: &str) -> String {
    value.replace("</", "<\\/")
}

pub(crate) async fn css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/style.css"),
    )
}

pub(crate) async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/app.js"),
    )
}

pub(crate) async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("static/favicon.ico").as_slice(),
    )
}

pub(crate) async fn apple_touch_icon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("static/apple-touch-icon.png").as_slice(),
    )
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
}
