//! WebChat theme configuration.
//!
//! Controls the visual theme of the web control UI (gateway dashboard).
//! Reads `WEBCHAT_THEME` env var or falls back to system preference detection.

use serde::{Deserialize, Serialize};

/// WebChat theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WebChatTheme {
    /// Light theme (white backgrounds, dark text).
    Light,
    /// Dark theme (dark backgrounds, light text).
    Dark,
    /// Follow system preference (prefers-color-scheme).
    System,
}

impl WebChatTheme {
    /// CSS class name to apply on the `<html>` or `<body>` element.
    pub fn css_class(&self) -> &str {
        match self {
            Self::Light => "theme-light",
            Self::Dark => "theme-dark",
            Self::System => "theme-system",
        }
    }

    /// CSS media query value for the theme.
    pub fn media_query(&self) -> Option<&str> {
        match self {
            Self::Dark => Some("(prefers-color-scheme: dark)"),
            Self::Light => Some("(prefers-color-scheme: light)"),
            Self::System => None,
        }
    }

    /// Whether the theme is a dark variant.
    pub fn is_dark(&self) -> bool {
        matches!(self, Self::Dark)
    }
}

impl Default for WebChatTheme {
    fn default() -> Self {
        Self::System
    }
}

impl std::fmt::Display for WebChatTheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Light => write!(f, "light"),
            Self::Dark => write!(f, "dark"),
            Self::System => write!(f, "system"),
        }
    }
}

/// WebChat configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebChatConfig {
    /// Theme preference.
    pub theme: WebChatTheme,
    /// Custom accent color for the web UI (hex, e.g. "#22c55e").
    pub accent_color: Option<String>,
    /// Whether to show the "Powered by IronClaw" badge.
    pub show_branding: bool,
}

impl Default for WebChatConfig {
    fn default() -> Self {
        Self {
            theme: WebChatTheme::System,
            accent_color: None,
            show_branding: true,
        }
    }
}

impl WebChatConfig {
    /// Load from environment variables.
    pub fn from_env() -> Self {
        let theme = std::env::var("WEBCHAT_THEME")
            .ok()
            .and_then(|v| match v.to_lowercase().as_str() {
                "light" => Some(WebChatTheme::Light),
                "dark" => Some(WebChatTheme::Dark),
                "system" | "auto" => Some(WebChatTheme::System),
                _ => None,
            })
            .unwrap_or_default();

        let accent_color = std::env::var("WEBCHAT_ACCENT_COLOR").ok();
        let show_branding = std::env::var("WEBCHAT_SHOW_BRANDING")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);

        Self {
            theme,
            accent_color,
            show_branding,
        }
    }

    /// Generate the CSS variables for the theme.
    pub fn css_variables(&self) -> String {
        let mut vars = format!(":root {{ --webchat-theme: {}; ", self.theme);
        if let Some(ref color) = self.accent_color {
            vars.push_str(&format!("--webchat-accent: {}; ", color));
        }
        vars.push('}');
        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_default() {
        assert_eq!(WebChatTheme::default(), WebChatTheme::System);
    }

    #[test]
    fn test_theme_css_class() {
        assert_eq!(WebChatTheme::Light.css_class(), "theme-light");
        assert_eq!(WebChatTheme::Dark.css_class(), "theme-dark");
        assert_eq!(WebChatTheme::System.css_class(), "theme-system");
    }

    #[test]
    fn test_theme_display() {
        assert_eq!(WebChatTheme::Light.to_string(), "light");
        assert_eq!(WebChatTheme::Dark.to_string(), "dark");
        assert_eq!(WebChatTheme::System.to_string(), "system");
    }

    #[test]
    fn test_theme_serializable() {
        let json = serde_json::to_string(&WebChatTheme::Dark).unwrap();
        assert_eq!(json, "\"dark\"");
        let deser: WebChatTheme = serde_json::from_str("\"light\"").unwrap();
        assert_eq!(deser, WebChatTheme::Light);
    }

    #[test]
    fn test_config_default() {
        let config = WebChatConfig::default();
        assert_eq!(config.theme, WebChatTheme::System);
        assert!(config.show_branding);
        assert!(config.accent_color.is_none());
    }

    #[test]
    fn test_config_serializable() {
        let config = WebChatConfig {
            theme: WebChatTheme::Dark,
            accent_color: Some("#22c55e".into()),
            show_branding: false,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"theme\":\"dark\""));
        assert!(json.contains("\"accent_color\":\"#22c55e\""));
        assert!(json.contains("\"show_branding\":false"));
    }

    #[test]
    fn test_css_variables() {
        let config = WebChatConfig {
            theme: WebChatTheme::Dark,
            accent_color: Some("#ff0000".into()),
            show_branding: true,
        };
        let css = config.css_variables();
        assert!(css.contains("--webchat-theme: dark"));
        assert!(css.contains("--webchat-accent: #ff0000"));
    }

    #[test]
    fn test_css_variables_no_accent() {
        let config = WebChatConfig::default();
        let css = config.css_variables();
        assert!(css.contains("--webchat-theme: system"));
        assert!(!css.contains("--webchat-accent"));
    }

    #[test]
    fn test_is_dark() {
        assert!(WebChatTheme::Dark.is_dark());
        assert!(!WebChatTheme::Light.is_dark());
        assert!(!WebChatTheme::System.is_dark());
    }

    #[test]
    fn test_media_query() {
        assert!(WebChatTheme::Dark.media_query().is_some());
        assert!(WebChatTheme::Light.media_query().is_some());
        assert!(WebChatTheme::System.media_query().is_none());
    }
}
