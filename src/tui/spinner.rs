//! Skin-driven animated spinner with kawaii face frames.
//!
//! Provides a [`KawaiiSpinner`] that cycles through emoji/braille frames
//! and renders as `ratatui` [`Span`]s. The TUI event loop advances the
//! spinner on every Nth tick (typically 300 ms).

use ratatui::prelude::*;

use crate::branding::skin::CliSkin;

/// Default kawaii face frames.
const DEFAULT_KAWAII_FRAMES: &[&str] = &[
    "(◕‿◕)",
    "(◕ᴗ◕)",
    "(◔‿◔)",
    "(◕‿◕)✧",
    "(◕‿◕)⋆",
    "(◕ᴗ◕)˚",
    "(◔‿◔)*",
    "(◕‿◕)·",
];

/// Braille dot spinner frames (compact, terminal-safe).
const DEFAULT_BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Simple dot spinner frames.
const DEFAULT_DOT_FRAMES: &[&str] = &["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"];

/// Arrow spinner frames.
const DEFAULT_ARROW_FRAMES: &[&str] = &["←", "↖", "↑", "↗", "→", "↘", "↓", "↙"];

/// Spinner preset style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpinnerStyle {
    Kawaii,
    Dots,
    Braille,
    Arrows,
}

impl SpinnerStyle {
    pub fn from_str_lossy(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "kawaii" => Self::Kawaii,
            "dots" => Self::Dots,
            "braille" => Self::Braille,
            "arrows" => Self::Arrows,
            _ => Self::Braille,
        }
    }

    pub fn default_frames(self) -> &'static [&'static str] {
        match self {
            Self::Kawaii => DEFAULT_KAWAII_FRAMES,
            Self::Dots => DEFAULT_DOT_FRAMES,
            Self::Braille => DEFAULT_BRAILLE_FRAMES,
            Self::Arrows => DEFAULT_ARROW_FRAMES,
        }
    }
}

impl Default for SpinnerStyle {
    fn default() -> Self {
        Self::Braille
    }
}

/// Skin-driven animated spinner.
///
/// The TUI event loop calls [`tick()`] periodically to advance the animation.
/// The current frame is rendered via [`to_spans()`] or [`to_line()`].
pub struct KawaiiSpinner {
    /// Resolved frame strings (from skin custom frames or preset defaults).
    frames: Vec<String>,
    /// Current tick counter.
    tick: usize,
    /// Optional label shown after the spinner frame.
    label: String,
}

impl KawaiiSpinner {
    /// Create a spinner from the active skin's configuration.
    pub fn from_skin(skin: &CliSkin, label: impl Into<String>) -> Self {
        let frames = skin.resolved_spinner_frames();
        Self {
            frames: frames.iter().map(|s| s.to_string()).collect(),
            tick: 0,
            label: label.into(),
        }
    }

    /// Create a spinner from explicit frames.
    pub fn with_frames(frames: &[&str], label: impl Into<String>) -> Self {
        Self {
            frames: frames.iter().map(|s| s.to_string()).collect(),
            tick: 0,
            label: label.into(),
        }
    }

    /// Advance the spinner by one frame.
    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    /// Get the current frame string.
    pub fn current_frame(&self) -> &str {
        if self.frames.is_empty() {
            "⠋"
        } else {
            &self.frames[self.tick % self.frames.len()]
        }
    }

    /// Update the label shown alongside the spinner.
    pub fn set_label(&mut self, label: impl Into<String>) {
        self.label = label.into();
    }

    /// Render as a list of styled `Span`s.
    pub fn to_spans(&self, skin: &CliSkin) -> Vec<Span<'static>> {
        let frame = self.current_frame().to_string();
        let mut spans = vec![Span::styled(frame, skin.accent_style())];
        if !self.label.is_empty() {
            spans.push(Span::styled(format!(" {}", self.label), skin.muted_style()));
        }
        spans
    }

    /// Render as a single `Line`.
    pub fn to_line(&self, skin: &CliSkin) -> Line<'static> {
        Line::from(self.to_spans(skin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spinner_style_from_str_lossy() {
        assert_eq!(SpinnerStyle::from_str_lossy("kawaii"), SpinnerStyle::Kawaii);
        assert_eq!(
            SpinnerStyle::from_str_lossy("Braille"),
            SpinnerStyle::Braille
        );
        assert_eq!(
            SpinnerStyle::from_str_lossy("unknown"),
            SpinnerStyle::Braille
        );
    }

    #[test]
    fn spinner_cycles_frames() {
        let mut spinner = KawaiiSpinner::with_frames(&["a", "b", "c"], "test");
        assert_eq!(spinner.current_frame(), "a");
        spinner.tick();
        assert_eq!(spinner.current_frame(), "b");
        spinner.tick();
        assert_eq!(spinner.current_frame(), "c");
        spinner.tick();
        assert_eq!(spinner.current_frame(), "a");
    }

    #[test]
    fn spinner_handles_empty_frames() {
        let spinner = KawaiiSpinner {
            frames: vec![],
            tick: 0,
            label: String::new(),
        };
        assert_eq!(spinner.current_frame(), "⠋");
    }

    #[test]
    fn spinner_to_spans_includes_label() {
        let skin = CliSkin::default();
        let spinner = KawaiiSpinner::with_frames(&["⠋"], "thinking");
        let spans = spinner.to_spans(&skin);
        assert_eq!(spans.len(), 2);
    }

    #[test]
    fn spinner_to_spans_no_label() {
        let skin = CliSkin::default();
        let spinner = KawaiiSpinner::with_frames(&["⠋"], "");
        let spans = spinner.to_spans(&skin);
        assert_eq!(spans.len(), 1);
    }
}
