//! Shared terminal branding helpers for boot, REPL, setup, and CLI subcommands.

use ratatui::style::Color;

use crate::settings::Settings;
use crate::tui::skin::CliSkin;

pub fn resolve_cli_skin_name() -> String {
    let settings = Settings::load();
    std::env::var("AGENT_CLI_SKIN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| settings.agent.cli_skin.clone())
}

#[derive(Debug, Clone)]
pub struct TerminalBranding {
    pub skin: CliSkin,
}

impl TerminalBranding {
    pub fn current() -> Self {
        Self::from_skin(CliSkin::load(&resolve_cli_skin_name()))
    }

    pub fn from_skin(skin: CliSkin) -> Self {
        Self { skin }
    }

    pub fn reset(&self) -> &'static str {
        self.skin.ansi_reset()
    }

    pub fn body(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.body, false)
    }

    pub fn body_bold(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.body, true)
    }

    pub fn accent(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.accent, true)
    }

    pub fn accent_soft(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.accent_soft, false)
    }

    pub fn muted(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.muted, false)
    }

    pub fn good(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.good, true)
    }

    pub fn warn(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.warn, true)
    }

    pub fn bad(&self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), self.skin.bad, true)
    }

    pub fn separator(&self, width: usize) -> String {
        self.muted("─".repeat(width.max(12)))
    }

    pub fn key_value(&self, key: &str, value: impl std::fmt::Display) -> String {
        format!(
            "  {}  {}",
            self.muted(format!("{key:<12}")),
            self.body(value.to_string())
        )
    }

    pub fn banner_lines(&self, title: &str, subtitle: Option<&str>) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(String::new());
        for line in self.skin.ascii_art() {
            lines.push(format!("  {}", self.accent(line)));
        }
        lines.push(format!("  {}", self.body_bold(title)));
        if let Some(text) = subtitle.or(self.skin.tagline()) {
            lines.push(format!("  {}", self.muted(text)));
        }
        lines.push(format!(
            "  {}",
            self.accent_soft(format!(
                "skin {}  prompt {}",
                self.skin.name,
                self.skin.prompt_symbol()
            ))
        ));
        lines.push(String::new());
        lines
    }

    pub fn print_banner(&self, title: &str, subtitle: Option<&str>) {
        for line in self.banner_lines(title, subtitle) {
            println!("{line}");
        }
    }

    fn paint(&self, text: &str, color: Color, bold: bool) -> String {
        let prefix = if bold { "\x1b[1m" } else { "" };
        format!(
            "{}{}{}{}",
            prefix,
            self.skin.ansi_fg(color),
            text,
            self.reset()
        )
    }
}
