//! Shared ThinClaw skin definitions for terminal and WebUI surfaces.
//!
//! Skins define named personalities with semantic palette tokens, prompt
//! affordances, and optional Web-specific aura colors. Terminal clients and the
//! WebUI both load these manifests so ThinClaw keeps a single branding system.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use ratatui::style::{Color, Style};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct SkinToml {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tagline: Option<String>,
    accent: String,
    border: String,
    body: String,
    muted: String,
    good: String,
    warn: String,
    bad: String,
    header: String,
    #[serde(default = "default_prompt_symbol")]
    prompt_symbol: String,
    #[serde(default)]
    ascii_art: Vec<String>,
    #[serde(default)]
    tool_emojis: HashMap<String, String>,
    #[serde(default)]
    web: Option<SkinWebToml>,
}

#[derive(Debug, Clone, Deserialize)]
struct SkinWebToml {
    aura_primary: String,
    aura_secondary: String,
    #[serde(default)]
    chrome_style: Option<WebChromeStyle>,
    #[serde(default)]
    surface_pattern: Option<WebSurfacePattern>,
    #[serde(default)]
    message_shape: Option<WebMessageShape>,
    #[serde(default)]
    elevation: Option<WebElevation>,
}

#[derive(Debug, Clone)]
pub struct SkinWebColors {
    pub aura_primary: Color,
    pub aura_secondary: Color,
    pub chrome_style: WebChromeStyle,
    pub surface_pattern: WebSurfacePattern,
    pub message_shape: WebMessageShape,
    pub elevation: WebElevation,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebChromeStyle {
    Avionics,
    Observatory,
    Archive,
    Marble,
    Oracle,
    Aerial,
}

impl WebChromeStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Avionics => "avionics",
            Self::Observatory => "observatory",
            Self::Archive => "archive",
            Self::Marble => "marble",
            Self::Oracle => "oracle",
            Self::Aerial => "aerial",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebSurfacePattern {
    Grid,
    Stars,
    Paper,
    Marble,
    Haze,
    Cloud,
}

impl WebSurfacePattern {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grid => "grid",
            Self::Stars => "stars",
            Self::Paper => "paper",
            Self::Marble => "marble",
            Self::Haze => "haze",
            Self::Cloud => "cloud",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebMessageShape {
    Angular,
    Rounded,
    Sculpted,
    Soft,
    Cut,
}

impl WebMessageShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Angular => "angular",
            Self::Rounded => "rounded",
            Self::Sculpted => "sculpted",
            Self::Soft => "soft",
            Self::Cut => "cut",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WebElevation {
    Low,
    Medium,
    High,
}

impl WebElevation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Runtime skin used by the TUI, REPL, setup shell, and WebUI.
#[derive(Debug, Clone)]
pub struct CliSkin {
    pub name: String,
    pub tagline: Option<String>,
    pub accent: Color,
    pub accent_soft: Color,
    pub border: Color,
    pub border_soft: Color,
    pub body: Color,
    pub muted: Color,
    pub good: Color,
    pub warn: Color,
    pub bad: Color,
    pub header: Color,
    pub prompt_symbol: String,
    pub ascii_art: Vec<String>,
    pub tool_emojis: HashMap<String, String>,
    pub web: SkinWebColors,
}

const BUILTIN_SKINS: &[&str] = &[
    "cockpit", "midnight", "solar", "athena", "delphi", "olympus",
];

impl CliSkin {
    pub fn load(name: &str) -> Self {
        let requested = name.trim();
        let chosen = if requested.is_empty() {
            "cockpit"
        } else {
            requested
        };

        if let Some(path_skin) = load_user_skin(chosen) {
            return path_skin;
        }

        load_builtin_skin(chosen)
            .unwrap_or_else(|| load_builtin_skin("cockpit").expect("builtin skin"))
    }

    pub fn builtin_names() -> impl Iterator<Item = &'static str> {
        BUILTIN_SKINS.iter().copied()
    }

    pub fn available_names() -> Vec<String> {
        let mut names = BTreeSet::new();
        names.extend(BUILTIN_SKINS.iter().copied().map(str::to_string));

        if let Some(dir) = skin_dir()
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && !stem.trim().is_empty()
                {
                    names.insert(stem.to_string());
                }
            }
        }

        names.into_iter().collect()
    }

    pub fn prompt_symbol(&self) -> &str {
        &self.prompt_symbol
    }

    pub fn tagline(&self) -> Option<&str> {
        self.tagline.as_deref()
    }

    pub fn ascii_art(&self) -> &[String] {
        &self.ascii_art
    }

    pub fn tool_label(&self, tool_name: &str) -> String {
        match self.tool_emojis.get(tool_name) {
            Some(emoji) if !emoji.trim().is_empty() => format!("{emoji} {tool_name}"),
            _ => tool_name.to_string(),
        }
    }

    pub fn tool_emoji(&self, tool_name: &str) -> Option<&str> {
        self.tool_emojis
            .get(tool_name)
            .map(String::as_str)
            .filter(|emoji| !emoji.trim().is_empty())
    }

    pub fn title_style(&self) -> Style {
        Style::default().fg(self.body).bg(self.header).bold()
    }

    pub fn accent_style(&self) -> Style {
        Style::default().fg(self.accent).bold()
    }

    pub fn accent_soft_style(&self) -> Style {
        Style::default().fg(self.accent_soft).bold()
    }

    pub fn border_style(&self) -> Style {
        Style::default().fg(self.border)
    }

    pub fn border_soft_style(&self) -> Style {
        Style::default().fg(self.border_soft)
    }

    pub fn body_style(&self) -> Style {
        Style::default().fg(self.body)
    }

    pub fn muted_style(&self) -> Style {
        Style::default().fg(self.muted)
    }

    pub fn good_style(&self) -> Style {
        Style::default().fg(self.good)
    }

    pub fn warn_style(&self) -> Style {
        Style::default().fg(self.warn)
    }

    pub fn bad_style(&self) -> Style {
        Style::default().fg(self.bad)
    }

    pub fn web_aura_primary(&self) -> Color {
        self.web.aura_primary
    }

    pub fn web_aura_secondary(&self) -> Color {
        self.web.aura_secondary
    }

    pub fn web_chrome_style(&self) -> &'static str {
        self.web.chrome_style.as_str()
    }

    pub fn web_surface_pattern(&self) -> &'static str {
        self.web.surface_pattern.as_str()
    }

    pub fn web_message_shape(&self) -> &'static str {
        self.web.message_shape.as_str()
    }

    pub fn web_elevation(&self) -> &'static str {
        self.web.elevation.as_str()
    }

    pub fn to_termimad_skin(&self) -> termimad::MadSkin {
        let mut skin = termimad::MadSkin::default();
        skin.set_headers_fg(to_term_color(self.header));
        skin.bold.set_fg(to_term_color(self.body));
        skin.italic.set_fg(to_term_color(self.accent_soft));
        skin.inline_code.set_fg(to_term_color(self.good));
        skin.code_block.set_fg(to_term_color(self.good));
        skin.code_block.left_margin = 2;
        skin
    }

    pub fn ansi_fg(&self, color: Color) -> String {
        format!(
            "\x1b[38;2;{};{};{}m",
            rgb(color).0,
            rgb(color).1,
            rgb(color).2
        )
    }

    pub fn ansi_bg(&self, color: Color) -> String {
        format!(
            "\x1b[48;2;{};{};{}m",
            rgb(color).0,
            rgb(color).1,
            rgb(color).2
        )
    }

    pub fn ansi_reset(&self) -> &'static str {
        "\x1b[0m"
    }
}

impl Default for CliSkin {
    fn default() -> Self {
        Self::load("cockpit")
    }
}

pub fn color_to_hex(color: Color) -> String {
    let (r, g, b) = rgb(color);
    format!("#{r:02X}{g:02X}{b:02X}")
}

pub fn mix_color(a: Color, b: Color, ratio: f32) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let lerp = |x: u8, y: u8| -> u8 {
        let blended = (x as f32 * (1.0 - ratio)) + (y as f32 * ratio);
        blended.round().clamp(0.0, 255.0) as u8
    };
    Color::Rgb(lerp(ar, br), lerp(ag, bg), lerp(ab, bb))
}

fn default_prompt_symbol() -> String {
    "›".to_string()
}

fn skin_dir() -> Option<PathBuf> {
    Some(crate::platform::resolve_data_dir("skins"))
}

fn load_user_skin(name: &str) -> Option<CliSkin> {
    let path = skin_dir()?.join(format!("{name}.toml"));
    let data = std::fs::read_to_string(&path).ok()?;
    match parse_skin_toml(&data, name) {
        Ok(skin) => Some(skin),
        Err(err) => {
            tracing::warn!(
                path = %path.display(),
                error = %err,
                "Failed to parse user CLI skin; falling back to builtin"
            );
            None
        }
    }
}

fn load_builtin_skin(name: &str) -> Option<CliSkin> {
    let raw = match name {
        "cockpit" => Some(include_str!("../../assets/skins/cockpit.toml")),
        "midnight" => Some(include_str!("../../assets/skins/midnight.toml")),
        "solar" => Some(include_str!("../../assets/skins/solar.toml")),
        "athena" => Some(include_str!("../../assets/skins/athena.toml")),
        "delphi" => Some(include_str!("../../assets/skins/delphi.toml")),
        "olympus" => Some(include_str!("../../assets/skins/olympus.toml")),
        _ => None,
    }?;

    Some(parse_skin_toml(raw, name).expect("builtin skin TOML is valid"))
}

fn parse_skin_toml(data: &str, fallback_name: &str) -> Result<CliSkin, String> {
    let raw: SkinToml = toml::from_str(data).map_err(|e| e.to_string())?;
    let name = raw.name.unwrap_or_else(|| fallback_name.to_string());
    let accent = parse_color(&raw.accent)?;
    let border = parse_color(&raw.border)?;
    let body = parse_color(&raw.body)?;
    let muted = parse_color(&raw.muted)?;
    let good = parse_color(&raw.good)?;
    let warn = parse_color(&raw.warn)?;
    let bad = parse_color(&raw.bad)?;
    let header = parse_color(&raw.header)?;
    let web = parse_web_colors(raw.web.as_ref(), accent, border, header, body)?;

    Ok(CliSkin {
        name,
        tagline: raw.tagline,
        accent,
        accent_soft: mix_color(accent, body, 0.40),
        border,
        border_soft: mix_color(border, body, 0.50),
        body,
        muted,
        good,
        warn,
        bad,
        header,
        prompt_symbol: raw.prompt_symbol,
        ascii_art: raw.ascii_art,
        tool_emojis: raw.tool_emojis,
        web,
    })
}

fn parse_web_colors(
    raw: Option<&SkinWebToml>,
    accent: Color,
    border: Color,
    header: Color,
    body: Color,
) -> Result<SkinWebColors, String> {
    if let Some(raw) = raw {
        return Ok(SkinWebColors {
            aura_primary: parse_color(&raw.aura_primary)?,
            aura_secondary: parse_color(&raw.aura_secondary)?,
            chrome_style: raw.chrome_style.unwrap_or(WebChromeStyle::Avionics),
            surface_pattern: raw.surface_pattern.unwrap_or(WebSurfacePattern::Grid),
            message_shape: raw.message_shape.unwrap_or(WebMessageShape::Rounded),
            elevation: raw.elevation.unwrap_or(WebElevation::Medium),
        });
    }

    Ok(SkinWebColors {
        aura_primary: mix_color(accent, header, 0.32),
        aura_secondary: mix_color(border, body, 0.18),
        chrome_style: WebChromeStyle::Avionics,
        surface_pattern: WebSurfacePattern::Grid,
        message_shape: WebMessageShape::Rounded,
        elevation: WebElevation::Medium,
    })
}

fn parse_color(value: &str) -> Result<Color, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("empty color".to_string());
    }
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 {
        return Err(format!("invalid color '{value}': expected #RRGGBB"));
    }
    let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())?;
    Ok(Color::Rgb(r, g, b))
}

fn rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(idx) => (idx, idx, idx),
        Color::Reset => (255, 255, 255),
        Color::Black => (0, 0, 0),
        Color::Red => (204, 0, 0),
        Color::Green => (0, 153, 0),
        Color::Yellow => (204, 153, 0),
        Color::Blue => (0, 102, 204),
        Color::Magenta => (153, 0, 153),
        Color::Cyan => (0, 153, 153),
        Color::Gray => (153, 153, 153),
        Color::DarkGray => (96, 96, 96),
        Color::LightRed => (255, 102, 102),
        Color::LightGreen => (102, 255, 102),
        Color::LightYellow => (255, 204, 102),
        Color::LightBlue => (102, 178, 255),
        Color::LightMagenta => (204, 102, 204),
        Color::LightCyan => (102, 255, 255),
        Color::White => (255, 255, 255),
    }
}

fn to_term_color(color: Color) -> termimad::crossterm::style::Color {
    match color {
        Color::Rgb(r, g, b) => termimad::crossterm::style::Color::Rgb { r, g, b },
        Color::Black => termimad::crossterm::style::Color::Black,
        Color::Red => termimad::crossterm::style::Color::DarkRed,
        Color::Green => termimad::crossterm::style::Color::DarkGreen,
        Color::Yellow => termimad::crossterm::style::Color::DarkYellow,
        Color::Blue => termimad::crossterm::style::Color::DarkBlue,
        Color::Magenta => termimad::crossterm::style::Color::DarkMagenta,
        Color::Cyan => termimad::crossterm::style::Color::DarkCyan,
        Color::Gray => termimad::crossterm::style::Color::Grey,
        Color::DarkGray => termimad::crossterm::style::Color::DarkGrey,
        Color::LightRed => termimad::crossterm::style::Color::Red,
        Color::LightGreen => termimad::crossterm::style::Color::Green,
        Color::LightYellow => termimad::crossterm::style::Color::Yellow,
        Color::LightBlue => termimad::crossterm::style::Color::Blue,
        Color::LightMagenta => termimad::crossterm::style::Color::Magenta,
        Color::LightCyan => termimad::crossterm::style::Color::Cyan,
        Color::White => termimad::crossterm::style::Color::White,
        Color::Reset => termimad::crossterm::style::Color::Reset,
        Color::Indexed(idx) => termimad::crossterm::style::Color::AnsiValue(idx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_builtin_skin() {
        let skin = CliSkin::load("cockpit");
        assert_eq!(skin.name, "cockpit");
        assert_eq!(skin.prompt_symbol(), "›");
    }

    #[test]
    fn available_names_include_builtins() {
        let names = CliSkin::available_names();
        assert!(names.contains(&"cockpit".to_string()));
        assert!(names.contains(&"midnight".to_string()));
        assert!(names.contains(&"solar".to_string()));
        assert!(names.contains(&"athena".to_string()));
        assert!(names.contains(&"delphi".to_string()));
        assert!(names.contains(&"olympus".to_string()));
    }

    #[test]
    fn builtin_names_are_stable() {
        let names: Vec<_> = CliSkin::builtin_names().collect();
        assert_eq!(
            names,
            vec![
                "cockpit", "midnight", "solar", "athena", "delphi", "olympus"
            ]
        );
    }

    #[test]
    fn builtin_skin_includes_ascii_art() {
        let skin = CliSkin::load("athena");
        assert!(!skin.ascii_art().is_empty());
        assert!(skin.tagline().is_some());
    }

    #[test]
    fn skin_toml_derives_web_colors_when_missing() {
        let toml = r##"
name = "derived"
accent = "#6AA6FF"
border = "#A7B4C6"
body = "#D9E2F2"
muted = "#7A8694"
good = "#5CCAA7"
warn = "#FFD166"
bad = "#FF6B6B"
header = "#C9E1FF"
prompt_symbol = ">"
ascii_art = ["DERIVED"]
"##;
        let parsed: SkinToml = toml::from_str(toml).expect("parse test skin");
        let web = parse_web_colors(
            parsed.web.as_ref(),
            parse_color(&parsed.accent).expect("accent"),
            parse_color(&parsed.border).expect("border"),
            parse_color(&parsed.header).expect("header"),
            parse_color(&parsed.body).expect("body"),
        )
        .expect("derive web colors");
        assert_eq!(
            color_to_hex(web.aura_primary),
            color_to_hex(mix_color(
                parse_color(&parsed.accent).expect("accent"),
                parse_color(&parsed.header).expect("header"),
                0.32,
            ))
        );
        assert_eq!(
            color_to_hex(web.aura_secondary),
            color_to_hex(mix_color(
                parse_color(&parsed.border).expect("border"),
                parse_color(&parsed.body).expect("body"),
                0.18,
            ))
        );
        assert_eq!(web.chrome_style.as_str(), "avionics");
        assert_eq!(web.surface_pattern.as_str(), "grid");
        assert_eq!(web.message_shape.as_str(), "rounded");
        assert_eq!(web.elevation.as_str(), "medium");
    }

    #[test]
    fn parse_skin_toml_accepts_explicit_web_colors() {
        let skin = parse_skin_toml(
            r##"
name = "test"
accent = "#111111"
border = "#222222"
body = "#333333"
muted = "#444444"
good = "#555555"
warn = "#666666"
bad = "#777777"
header = "#888888"
[web]
aura_primary = "#123456"
aura_secondary = "#654321"
chrome_style = "oracle"
surface_pattern = "haze"
message_shape = "cut"
elevation = "high"
    "##,
            "test",
        )
        .expect("skin parses");
        assert_eq!(color_to_hex(skin.web_aura_primary()), "#123456");
        assert_eq!(color_to_hex(skin.web_aura_secondary()), "#654321");
        assert_eq!(skin.web_chrome_style(), "oracle");
        assert_eq!(skin.web_surface_pattern(), "haze");
        assert_eq!(skin.web_message_shape(), "cut");
        assert_eq!(skin.web_elevation(), "high");
    }
}
