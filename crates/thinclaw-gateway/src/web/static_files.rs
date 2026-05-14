use axum::{http::header, response::IntoResponse};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebChatTheme {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone)]
pub struct WebChatRuntimeConfig {
    pub theme: WebChatTheme,
    pub show_branding: bool,
    pub runtime_css: String,
    pub bootstrap_payload: serde_json::Value,
}

pub fn render_index_response(webchat: &WebChatRuntimeConfig) -> impl IntoResponse + use<> {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        render_index_html(webchat),
    )
}

pub fn render_index_html(webchat: &WebChatRuntimeConfig) -> String {
    let theme = match webchat.theme {
        WebChatTheme::Light => "light",
        WebChatTheme::Dark => "dark",
        WebChatTheme::System => "system",
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

    if !webchat.runtime_css.is_empty() {
        html = html.replace(
            "</head>",
            &format!(
                "  <style id=\"webchat-runtime-theme\">{}</style>\n</head>",
                webchat.runtime_css
            ),
        );
    }

    let payload_json = escape_json_for_html(
        &serde_json::to_string(&webchat.bootstrap_payload)
            .expect("webchat bootstrap payload serializes"),
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

pub async fn css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/style.css"),
    )
}

pub async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/app.js"),
    )
}

pub async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("static/favicon.ico").as_slice(),
    )
}

pub async fn apple_touch_icon_handler() -> impl IntoResponse {
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
        let html = render_index_html(&WebChatRuntimeConfig {
            theme: WebChatTheme::System,
            show_branding: true,
            runtime_css: String::new(),
            bootstrap_payload: serde_json::json!({
                "availableSkins": [],
                "resolvedSkin": null,
            }),
        });
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
