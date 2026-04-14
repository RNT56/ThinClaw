//! WebChat theme and shared skin configuration.
//!
//! Controls the visual theme of the web control UI and resolves which shared
//! ThinClaw skin the WebUI should use.

use serde::{Deserialize, Serialize};

use crate::branding::skin::{CliSkin, color_to_hex, mix_color};

/// WebChat theme preference.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum WebChatTheme {
    /// Light theme (white backgrounds, dark text).
    Light,
    /// Dark theme (dark backgrounds, light text).
    Dark,
    /// Follow system preference (prefers-color-scheme).
    #[default]
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
    /// Configured assistant name for chat transcript surfaces.
    pub agent_name: String,
    /// Optional explicit Web UI skin override.
    pub skin: Option<String>,
    /// Default CLI skin fallback for the Web UI.
    pub cli_skin: String,
    /// Custom accent color override for the web UI (hex, legacy).
    pub accent_color: Option<String>,
    /// Whether to show the branding badge.
    pub show_branding: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebChatBootstrap {
    pub theme: String,
    pub agent_name: String,
    pub show_branding: bool,
    pub available_skins: Vec<String>,
    pub resolved_skin: ResolvedWebSkin,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedWebSkin {
    pub name: String,
    pub tagline: Option<String>,
    pub prompt_symbol: String,
    pub tool_emojis: std::collections::HashMap<String, String>,
    pub chrome_style: String,
    pub surface_pattern: String,
    pub message_shape: String,
    pub elevation: String,
    pub css_vars: WebSkinCssVars,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSkinCssVars {
    pub accent: String,
    pub accent_soft: String,
    pub header: String,
    pub border_strong: String,
    pub border_soft: String,
    pub good: String,
    pub warn: String,
    pub bad: String,
    pub aura_primary: String,
    pub aura_secondary: String,
    pub sidebar_bg: String,
    pub thread_active_bg: String,
    pub assistant_bg: String,
    pub assistant_edge: String,
    pub user_edge: String,
    pub composer_bg: String,
    pub composer_edge: String,
    pub code_bg: String,
    pub turn_bg: String,
    pub turn_border: String,
    pub message_shadow: String,
    pub turn_shadow: String,
    pub chat_user_bg: String,
    pub chat_user_fg: String,
    pub chat_assistant_border: String,
    pub chat_system_bg: String,
    pub badge_bg: String,
    pub badge_fg: String,
    pub feature_glow: String,
}

impl Default for WebChatConfig {
    fn default() -> Self {
        Self {
            theme: WebChatTheme::System,
            agent_name: "thinclaw".to_string(),
            skin: None,
            cli_skin: "cockpit".to_string(),
            accent_color: None,
            show_branding: true,
        }
    }
}

impl WebChatConfig {
    /// Build from persisted settings values.
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let theme = match settings.webchat_theme.to_lowercase().as_str() {
            "light" => WebChatTheme::Light,
            "dark" => WebChatTheme::Dark,
            _ => WebChatTheme::System,
        };

        Self {
            theme,
            agent_name: settings.agent.name.clone(),
            skin: settings.webchat_skin.clone(),
            cli_skin: settings.agent.cli_skin.clone(),
            accent_color: settings.webchat_accent_color.clone(),
            show_branding: settings.webchat_show_branding,
        }
    }

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

        let skin = std::env::var("WEBCHAT_SKIN")
            .ok()
            .filter(|v| !v.trim().is_empty());
        let agent_name = std::env::var("AGENT_NAME")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "thinclaw".to_string());
        let cli_skin = std::env::var("AGENT_CLI_SKIN")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| "cockpit".to_string());
        let accent_color = std::env::var("WEBCHAT_ACCENT_COLOR").ok();
        let show_branding = std::env::var("WEBCHAT_SHOW_BRANDING")
            .map(|v| v != "0" && !v.eq_ignore_ascii_case("false"))
            .unwrap_or(true);

        Self {
            theme,
            agent_name,
            skin,
            cli_skin,
            accent_color,
            show_branding,
        }
    }

    pub fn resolved_skin_name(&self) -> &str {
        self.skin.as_deref().unwrap_or(&self.cli_skin)
    }

    pub fn resolved_skin(&self) -> CliSkin {
        CliSkin::load(self.resolved_skin_name())
    }

    pub fn bootstrap_payload(&self) -> WebChatBootstrap {
        let skin = self.resolved_skin();
        WebChatBootstrap {
            theme: self.theme.to_string(),
            agent_name: self.agent_name.clone(),
            show_branding: self.show_branding,
            available_skins: CliSkin::available_names(),
            resolved_skin: ResolvedWebSkin {
                name: skin.name.clone(),
                tagline: skin.tagline.clone(),
                prompt_symbol: skin.prompt_symbol.clone(),
                tool_emojis: skin.tool_emojis.clone(),
                chrome_style: skin.web_chrome_style().to_string(),
                surface_pattern: skin.web_surface_pattern().to_string(),
                message_shape: skin.web_message_shape().to_string(),
                elevation: skin.web_elevation().to_string(),
                css_vars: self.css_vars_for_skin(&skin, !matches!(self.theme, WebChatTheme::Light)),
            },
        }
    }

    pub fn runtime_css(&self) -> String {
        let skin = self.resolved_skin();
        let theme = self.theme.to_string();
        let dark_vars = self.css_vars_for_skin(&skin, true);
        let light_vars = self.css_vars_for_skin(&skin, false);
        let mut lines = vec![
            format!(
                ":root {{ {} }}",
                css_var_declarations(theme.as_str(), &dark_vars)
            ),
            format!(
                "html[data-webchat-theme=\"dark\"] {{ {} }}",
                css_var_declarations(theme.as_str(), &dark_vars)
            ),
            format!(
                "html[data-webchat-theme=\"light\"] {{ {} }}",
                css_var_declarations(theme.as_str(), &light_vars)
            ),
            format!(
                "@media (prefers-color-scheme: dark) {{ html[data-webchat-theme=\"system\"] {{ {} }} }}",
                css_var_declarations(theme.as_str(), &dark_vars)
            ),
            format!(
                "@media (prefers-color-scheme: light) {{ html[data-webchat-theme=\"system\"] {{ {} }} }}",
                css_var_declarations(theme.as_str(), &light_vars)
            ),
        ];

        if let Some(accent) = self.legacy_accent_override() {
            let override_vars = accent_override_declarations(&accent);
            lines.extend([
                format!(":root {{ {override_vars} }}"),
                format!("html[data-webchat-theme=\"dark\"] {{ {override_vars} }}"),
                format!("html[data-webchat-theme=\"light\"] {{ {override_vars} }}"),
                format!(
                    "@media (prefers-color-scheme: dark) {{ html[data-webchat-theme=\"system\"] {{ {override_vars} }} }}"
                ),
                format!(
                    "@media (prefers-color-scheme: light) {{ html[data-webchat-theme=\"system\"] {{ {override_vars} }} }}"
                ),
            ]);
        }

        lines.join("\n")
    }

    pub fn legacy_accent_override(&self) -> Option<String> {
        self.accent_color
            .as_deref()
            .filter(|value| is_safe_hex_color(value))
            .map(ToOwned::to_owned)
    }

    fn css_vars_for_skin(&self, skin: &CliSkin, theme_is_dark: bool) -> WebSkinCssVars {
        let theme_surface = if theme_is_dark {
            ratatui::style::Color::Rgb(9, 9, 11)
        } else {
            ratatui::style::Color::Rgb(245, 247, 251)
        };
        let accent = color_to_hex(skin.accent);
        let accent_soft = rgba_hex(&color_to_hex(skin.accent_soft), 0.22);
        let header = color_to_hex(skin.header);
        let border_strong = rgba_color(skin.border, if theme_is_dark { 0.30 } else { 0.22 });
        let border_soft = rgba_color(skin.border_soft, if theme_is_dark { 0.18 } else { 0.14 });
        let good = color_to_hex(skin.good);
        let warn = color_to_hex(skin.warn);
        let bad = color_to_hex(skin.bad);
        let aura_primary = rgba_color(
            skin.web_aura_primary(),
            if theme_is_dark { 0.20 } else { 0.14 },
        );
        let aura_secondary = rgba_color(
            skin.web_aura_secondary(),
            if theme_is_dark { 0.16 } else { 0.10 },
        );
        let sidebar_bg = color_to_hex(mix_color(
            skin.web_aura_secondary(),
            theme_surface,
            if theme_is_dark { 0.84 } else { 0.92 },
        ));
        let thread_active_bg = rgba_color(skin.accent, if theme_is_dark { 0.14 } else { 0.10 });
        let assistant_bg = color_to_hex(mix_color(
            skin.body,
            theme_surface,
            if theme_is_dark { 0.88 } else { 0.80 },
        ));
        let assistant_edge = rgba_color(skin.header, if theme_is_dark { 0.24 } else { 0.18 });
        let user_edge = rgba_color(skin.accent, if theme_is_dark { 0.44 } else { 0.28 });
        let composer_bg = color_to_hex(mix_color(
            skin.header,
            theme_surface,
            if theme_is_dark { 0.92 } else { 0.96 },
        ));
        let composer_edge = rgba_color(skin.border, if theme_is_dark { 0.24 } else { 0.18 });
        let code_bg = color_to_hex(mix_color(
            skin.border,
            theme_surface,
            if theme_is_dark { 0.22 } else { 0.12 },
        ));
        let turn_bg = rgba_color(
            mix_color(
                skin.web_aura_secondary(),
                theme_surface,
                if theme_is_dark { 0.66 } else { 0.78 },
            ),
            if theme_is_dark { 0.34 } else { 0.70 },
        );
        let turn_border = rgba_color(skin.border, if theme_is_dark { 0.24 } else { 0.14 });
        let message_shadow = message_shadow_value(skin.web_elevation(), theme_is_dark);
        let turn_shadow = turn_shadow_value(skin.web_elevation(), theme_is_dark);
        let chat_user_bg_color = if theme_is_dark {
            mix_color(skin.accent, theme_surface, 0.08)
        } else {
            mix_color(skin.accent, ratatui::style::Color::White, 0.18)
        };
        let chat_user_bg = color_to_hex(chat_user_bg_color);
        let chat_user_fg = best_text_color(chat_user_bg_color);
        let chat_assistant_border =
            rgba_color(skin.border, if theme_is_dark { 0.26 } else { 0.18 });
        let chat_system_bg = rgba_color(
            mix_color(skin.header, theme_surface, 0.65),
            if theme_is_dark { 0.34 } else { 0.60 },
        );
        let badge_bg_color = mix_color(
            skin.header,
            theme_surface,
            if theme_is_dark { 0.72 } else { 0.84 },
        );
        let badge_bg = color_to_hex(badge_bg_color);
        let badge_fg = best_text_color(badge_bg_color);
        let feature_glow = format!(
            "radial-gradient(circle at top right, {} 0%, transparent 36%)",
            rgba_color(
                skin.web_aura_primary(),
                if theme_is_dark { 0.20 } else { 0.16 }
            )
        );

        WebSkinCssVars {
            accent,
            accent_soft,
            header,
            border_strong,
            border_soft,
            good,
            warn,
            bad,
            aura_primary,
            aura_secondary,
            sidebar_bg,
            thread_active_bg,
            assistant_bg,
            assistant_edge,
            user_edge,
            composer_bg,
            composer_edge,
            code_bg,
            turn_bg,
            turn_border,
            message_shadow,
            turn_shadow,
            chat_user_bg,
            chat_user_fg,
            chat_assistant_border,
            chat_system_bg,
            badge_bg,
            badge_fg,
            feature_glow,
        }
    }
}

fn css_var_declarations(theme: &str, vars: &WebSkinCssVars) -> String {
    format!(
        "--webchat-theme: {}; --accent: {}; --accent-hover: {}; --accent-soft: {}; --header: {}; --border-strong: {}; --border-soft: {}; --good: {}; --warn: {}; --bad: {}; --aura-primary: {}; --aura-secondary: {}; --sidebar-bg: {}; --thread-active-bg: {}; --assistant-bg: {}; --assistant-edge: {}; --user-edge: {}; --composer-bg: {}; --composer-edge: {}; --code-bg: {}; --turn-bg: {}; --turn-border: {}; --message-shadow: {}; --turn-shadow: {}; --chat-user-bg: {}; --chat-user-fg: {}; --chat-assistant-border: {}; --chat-system-bg: {}; --badge-bg: {}; --badge-fg: {}; --feature-glow: {};",
        theme,
        vars.accent,
        vars.accent,
        vars.accent_soft,
        vars.header,
        vars.border_strong,
        vars.border_soft,
        vars.good,
        vars.warn,
        vars.bad,
        vars.aura_primary,
        vars.aura_secondary,
        vars.sidebar_bg,
        vars.thread_active_bg,
        vars.assistant_bg,
        vars.assistant_edge,
        vars.user_edge,
        vars.composer_bg,
        vars.composer_edge,
        vars.code_bg,
        vars.turn_bg,
        vars.turn_border,
        vars.message_shadow,
        vars.turn_shadow,
        vars.chat_user_bg,
        vars.chat_user_fg,
        vars.chat_assistant_border,
        vars.chat_system_bg,
        vars.badge_bg,
        vars.badge_fg,
        vars.feature_glow,
    )
}

fn accent_override_declarations(accent: &str) -> String {
    let hover = rgba_hex(accent, 0.88);
    format!(
        "--webchat-accent: {}; --accent: {}; --accent-hover: {}; --accent-soft: {};",
        accent,
        accent,
        hover,
        rgba_hex(accent, 0.24),
    )
}

fn best_text_color(background: ratatui::style::Color) -> String {
    let (r, g, b) = match background {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        other => {
            let hex = color_to_hex(other);
            let r = u8::from_str_radix(&hex[1..3], 16).unwrap_or(255);
            let g = u8::from_str_radix(&hex[3..5], 16).unwrap_or(255);
            let b = u8::from_str_radix(&hex[5..7], 16).unwrap_or(255);
            (r, g, b)
        }
    };
    let luminance = (0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b)) / 255.0;
    if luminance > 0.56 {
        "#09090B".to_string()
    } else {
        "#FAFAFA".to_string()
    }
}

fn rgba_hex(hex: &str, alpha: f32) -> String {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return format!("rgba(52, 211, 153, {:.3})", alpha.clamp(0.0, 1.0));
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(52);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(211);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(153);
    format!("rgba({r}, {g}, {b}, {:.3})", alpha.clamp(0.0, 1.0))
}

fn rgba_color(color: ratatui::style::Color, alpha: f32) -> String {
    let hex = color_to_hex(color);
    rgba_hex(&hex, alpha)
}

fn message_shadow_value(level: &str, theme_is_dark: bool) -> String {
    match (level, theme_is_dark) {
        ("low", true) => {
            "0 12px 22px rgba(0, 0, 0, 0.18), 0 1px 0 rgba(255, 255, 255, 0.03)".to_string()
        }
        ("low", false) => {
            "0 12px 26px rgba(15, 23, 42, 0.08), 0 1px 0 rgba(255, 255, 255, 0.72)".to_string()
        }
        ("high", true) => {
            "0 22px 40px rgba(0, 0, 0, 0.28), 0 1px 0 rgba(255, 255, 255, 0.04)".to_string()
        }
        ("high", false) => {
            "0 20px 42px rgba(15, 23, 42, 0.14), 0 1px 0 rgba(255, 255, 255, 0.78)".to_string()
        }
        (_, true) => {
            "0 16px 30px rgba(0, 0, 0, 0.22), 0 1px 0 rgba(255, 255, 255, 0.03)".to_string()
        }
        _ => "0 16px 34px rgba(15, 23, 42, 0.10), 0 1px 0 rgba(255, 255, 255, 0.76)".to_string(),
    }
}

fn turn_shadow_value(level: &str, theme_is_dark: bool) -> String {
    match (level, theme_is_dark) {
        ("low", true) => "0 6px 18px rgba(0, 0, 0, 0.12)".to_string(),
        ("low", false) => "0 6px 18px rgba(15, 23, 42, 0.05)".to_string(),
        ("high", true) => "0 14px 32px rgba(0, 0, 0, 0.18)".to_string(),
        ("high", false) => "0 14px 32px rgba(15, 23, 42, 0.08)".to_string(),
        (_, true) => "0 10px 24px rgba(0, 0, 0, 0.14)".to_string(),
        _ => "0 10px 24px rgba(15, 23, 42, 0.06)".to_string(),
    }
}

pub fn is_safe_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 4 | 7 | 9)
        && bytes.first() == Some(&b'#')
        && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
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
        assert_eq!(config.agent_name, "thinclaw");
        assert!(config.skin.is_none());
        assert!(config.accent_color.is_none());
        assert_eq!(config.cli_skin, "cockpit");
    }

    #[test]
    fn test_resolved_skin_name_prefers_webchat_override() {
        let config = WebChatConfig {
            theme: WebChatTheme::Dark,
            agent_name: "thinclaw".into(),
            skin: Some("athena".into()),
            cli_skin: "midnight".into(),
            accent_color: None,
            show_branding: true,
        };
        assert_eq!(config.resolved_skin_name(), "athena");
    }

    #[test]
    fn test_resolved_skin_name_falls_back_to_cli_skin() {
        let config = WebChatConfig {
            theme: WebChatTheme::Dark,
            agent_name: "thinclaw".into(),
            skin: None,
            cli_skin: "midnight".into(),
            accent_color: None,
            show_branding: true,
        };
        assert_eq!(config.resolved_skin_name(), "midnight");
    }

    #[test]
    fn test_bootstrap_payload_contains_expected_shape() {
        let config = WebChatConfig::default();
        let payload = config.bootstrap_payload();
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("agentName"));
        assert!(json.contains("availableSkins"));
        assert!(json.contains("resolvedSkin"));
        assert!(json.contains("cssVars"));
        assert!(json.contains("promptSymbol"));
        assert!(json.contains("chromeStyle"));
        assert!(json.contains("surfacePattern"));
        assert!(json.contains("messageShape"));
        assert!(json.contains("elevation"));
    }

    #[test]
    fn test_runtime_css_contains_contract_vars() {
        let config = WebChatConfig {
            theme: WebChatTheme::Dark,
            agent_name: "thinclaw".into(),
            skin: Some("athena".into()),
            cli_skin: "cockpit".into(),
            accent_color: None,
            show_branding: true,
        };
        let css = config.runtime_css();
        assert!(css.contains("--accent:"));
        assert!(css.contains("--aura-primary:"));
        assert!(css.contains("--assistant-bg:"));
        assert!(css.contains("--composer-bg:"));
        assert!(css.contains("--turn-bg:"));
        assert!(css.contains("--chat-user-bg:"));
        assert!(css.contains("--feature-glow:"));
    }

    #[test]
    fn test_runtime_css_tracks_system_light_and_dark_media_queries() {
        let config = WebChatConfig {
            theme: WebChatTheme::System,
            agent_name: "thinclaw".into(),
            skin: Some("athena".into()),
            cli_skin: "cockpit".into(),
            accent_color: None,
            show_branding: true,
        };
        let css = config.runtime_css();
        assert!(css.contains("@media (prefers-color-scheme: dark)"));
        assert!(css.contains("@media (prefers-color-scheme: light)"));
        assert!(css.contains("html[data-webchat-theme=\"system\"]"));
    }

    #[test]
    fn test_legacy_accent_override_only_updates_accent_vars() {
        let config = WebChatConfig {
            theme: WebChatTheme::Dark,
            agent_name: "thinclaw".into(),
            skin: Some("athena".into()),
            cli_skin: "cockpit".into(),
            accent_color: Some("#ff0000".into()),
            show_branding: true,
        };
        let css = config.runtime_css();
        assert!(css.contains("--webchat-accent: #ff0000"));
        assert!(css.contains("--header:"));
        assert!(css.contains("--chat-user-bg:"));
        assert!(css.contains("html[data-webchat-theme=\"light\"] { --webchat-accent: #ff0000"));
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
