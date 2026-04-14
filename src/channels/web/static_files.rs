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

    let inline_css = render_webchat_inline_css(webchat);
    if !inline_css.is_empty() {
        html = html.replace(
            "</head>",
            &format!("  <style id=\"webchat-runtime-theme\">{inline_css}</style>\n</head>"),
        );
    }

    html
}

pub(crate) fn render_webchat_inline_css(webchat: &crate::config::WebChatConfig) -> String {
    let Some(accent) = webchat
        .accent_color
        .as_deref()
        .filter(|value| is_safe_hex_color(value))
    else {
        return String::new();
    };

    format!(":root {{ --accent: {accent}; --accent-hover: {accent}; }}")
}

pub(crate) fn is_safe_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 4 | 7 | 9)
        && bytes.first() == Some(&b'#')
        && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
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
