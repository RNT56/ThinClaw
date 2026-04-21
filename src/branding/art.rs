//! Shared branding art primitives for CLI and TUI surfaces.
//!
//! The ThinClaw wordmark is generated from a bitmap font so every surface can
//! render the same logo with palette-derived highlight, face, and shadow
//! layers. Skin-specific hero glyphs are stored as plain Unicode text and
//! tinted with a vertical gradient derived from the active skin.

use std::borrow::Cow;

use ratatui::prelude::*;
use unicode_width::UnicodeWidthStr;

use crate::branding::skin::{CliSkin, mix_color};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtTone {
    Highlight,
    Face,
    Shadow,
    HeroTop,
    HeroMid,
    HeroBase,
    Body,
    Muted,
}

#[derive(Debug, Clone)]
pub struct ArtSpan {
    pub text: Cow<'static, str>,
    pub tone: Option<ArtTone>,
    pub bold: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ArtLine {
    spans: Vec<ArtSpan>,
}

#[derive(Debug, Clone, Default)]
pub struct ArtBlock {
    lines: Vec<ArtLine>,
}

impl ArtSpan {
    pub fn plain(text: impl Into<Cow<'static, str>>) -> Self {
        Self {
            text: text.into(),
            tone: None,
            bold: false,
        }
    }

    pub fn toned(text: impl Into<Cow<'static, str>>, tone: ArtTone, bold: bool) -> Self {
        Self {
            text: text.into(),
            tone: Some(tone),
            bold,
        }
    }
}

impl ArtLine {
    pub fn from_spans(spans: Vec<ArtSpan>) -> Self {
        Self { spans }
    }

    pub fn blank(width: usize) -> Self {
        Self::from_spans(vec![ArtSpan::plain(" ".repeat(width))])
    }

    pub fn plain_text(&self) -> String {
        self.spans
            .iter()
            .map(|span| span.text.as_ref())
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn width(&self) -> usize {
        UnicodeWidthStr::width(self.plain_text().as_str())
    }

    pub fn to_ansi(&self, skin: &CliSkin) -> String {
        let mut out = String::new();
        let reset = skin.ansi_reset();
        for span in &self.spans {
            match span.tone {
                Some(tone) => {
                    if span.bold {
                        out.push_str("\x1b[1m");
                    }
                    out.push_str(&skin.ansi_fg(color_for_tone(skin, tone)));
                    out.push_str(span.text.as_ref());
                    out.push_str(reset);
                }
                None => out.push_str(span.text.as_ref()),
            }
        }
        out
    }

    pub fn to_ratatui(&self, skin: &CliSkin) -> Line<'static> {
        let spans = self
            .spans
            .iter()
            .map(|span| match span.tone {
                Some(tone) => Span::styled(
                    span.text.clone().into_owned(),
                    style_for_tone(skin, tone, span.bold),
                ),
                None => Span::raw(span.text.clone().into_owned()),
            })
            .collect::<Vec<_>>();
        Line::from(spans)
    }
}

impl ArtBlock {
    pub fn new(lines: Vec<ArtLine>) -> Self {
        Self { lines }
    }

    pub fn lines(&self) -> &[ArtLine] {
        &self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn width(&self) -> usize {
        self.lines.iter().map(ArtLine::width).max().unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        self.lines.len()
    }

    pub fn plain_lines(&self) -> Vec<String> {
        self.lines.iter().map(ArtLine::plain_text).collect()
    }

    pub fn to_ansi_lines(&self, skin: &CliSkin) -> Vec<String> {
        self.lines.iter().map(|line| line.to_ansi(skin)).collect()
    }

    pub fn to_ratatui_lines(&self, skin: &CliSkin) -> Vec<Line<'static>> {
        self.lines
            .iter()
            .map(|line| line.to_ratatui(skin))
            .collect()
    }
}

pub fn wordmark_block() -> ArtBlock {
    let mut cursor_x = 0usize;
    let depth = 2usize;
    let spacing = 1usize;
    let glyphs = WORDMARK.chars().map(letter_mask).collect::<Vec<_>>();
    let height = glyphs.first().map_or(0, |glyph| glyph.len());
    let width = glyphs
        .iter()
        .map(|glyph| glyph.first().map_or(0, |row| row.len()))
        .sum::<usize>()
        + spacing.saturating_mul(glyphs.len().saturating_sub(1))
        + depth;

    let mut canvas = vec![vec![0u8; width]; height + depth];
    for glyph in glyphs {
        let glyph_height = glyph.len();
        let glyph_width = glyph.first().map_or(0, |row| row.len());
        for y in 0..glyph_height {
            for (x, ch) in glyph[y].chars().enumerate() {
                if ch == ' ' {
                    continue;
                }
                paint(&mut canvas, cursor_x + x + 2, y + 2, 1);
                paint(&mut canvas, cursor_x + x + 1, y + 1, 2);
                paint(&mut canvas, cursor_x + x, y, 3);
            }
        }
        cursor_x += glyph_width + spacing;
    }

    ArtBlock::new(
        canvas
            .into_iter()
            .map(|row| row_to_art_line(&row))
            .collect::<Vec<_>>(),
    )
}

pub fn compact_wordmark_block() -> ArtBlock {
    let lines = vec![
        ArtLine::from_spans(vec![ArtSpan::toned(
            "╔══════════╗".to_string(),
            ArtTone::Highlight,
            true,
        )]),
        ArtLine::from_spans(vec![ArtSpan::toned(
            "║ THINCLAW ║".to_string(),
            ArtTone::Face,
            true,
        )]),
        ArtLine::from_spans(vec![ArtSpan::toned(
            "╚══════════╝".to_string(),
            ArtTone::Shadow,
            true,
        )]),
        ArtLine::from_spans(vec![ArtSpan::toned(
            "operator deck".to_string(),
            ArtTone::Muted,
            false,
        )]),
    ];
    ArtBlock::new(lines)
}

pub fn best_wordmark_block(max_width: usize) -> Option<ArtBlock> {
    let full = wordmark_block();
    if full.width() <= max_width {
        return Some(full);
    }

    let compact = compact_wordmark_block();
    if compact.width() <= max_width {
        Some(compact)
    } else {
        None
    }
}

pub fn hero_block(skin: &CliSkin) -> Option<ArtBlock> {
    if skin.hero_art().is_empty() {
        return None;
    }

    let total = skin.hero_art().len();
    let mut lines = Vec::with_capacity(total);
    for (idx, line) in skin.hero_art().iter().enumerate() {
        let ratio = if total <= 1 {
            0.0
        } else {
            idx as f32 / (total - 1) as f32
        };
        let tone = if ratio < 0.28 {
            ArtTone::HeroTop
        } else if ratio < 0.64 {
            ArtTone::HeroMid
        } else {
            ArtTone::HeroBase
        };
        lines.push(ArtLine::from_spans(vec![ArtSpan::toned(
            line.clone(),
            tone,
            true,
        )]));
    }

    Some(ArtBlock::new(lines))
}

pub fn onboarding_brand_block(skin: &CliSkin, max_width: usize) -> Option<ArtBlock> {
    let logo = wordmark_block();
    let compact_logo = compact_wordmark_block();
    let hero = hero_block(skin);

    if let Some(hero) = hero {
        let combined = compose_horizontal(&logo, &hero, 4);
        if combined.width() <= max_width {
            return Some(combined);
        }

        let stacked = stack_blocks(&logo, &hero, 1);
        if stacked.width() <= max_width {
            return Some(stacked);
        }

        let compact_combined = compose_horizontal(&compact_logo, &hero, 3);
        if compact_combined.width() <= max_width {
            return Some(compact_combined);
        }

        let compact_stacked = stack_blocks(&compact_logo, &hero, 1);
        if compact_stacked.width() <= max_width {
            return Some(compact_stacked);
        }
    }

    if logo.width() <= max_width {
        Some(logo)
    } else if compact_logo.width() <= max_width {
        Some(compact_logo)
    } else if let Some(hero) = hero_block(skin) {
        if hero.width() <= max_width {
            Some(hero)
        } else {
            None
        }
    } else {
        None
    }
}

pub fn wordmark_plain_lines() -> Vec<String> {
    wordmark_block().plain_lines()
}

pub fn wordmark_fits(width: usize) -> bool {
    best_wordmark_block(width).is_some()
}

pub fn hero_fits(skin: &CliSkin, width: usize) -> bool {
    hero_block(skin).is_some_and(|art| art.width() <= width)
}

pub fn text_display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

pub fn lines_display_width(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| UnicodeWidthStr::width(line.as_str()))
        .max()
        .unwrap_or(0)
}

pub fn color_for_tone(skin: &CliSkin, tone: ArtTone) -> Color {
    let white = Color::Rgb(255, 255, 255);
    let black = Color::Rgb(18, 18, 18);
    let highlight_anchor = stronger_contrast(skin.accent, skin.body, white);
    let shadow_anchor = darker_of(skin.border, black);
    let hero_anchor = stronger_contrast(skin.header, skin.body, white);

    match tone {
        ArtTone::Highlight => mix_color(skin.accent, highlight_anchor, 0.32),
        ArtTone::Face => mix_color(skin.accent, skin.header, 0.10),
        ArtTone::Shadow => mix_color(skin.accent, shadow_anchor, 0.56),
        ArtTone::HeroTop => mix_color(skin.header, hero_anchor, 0.28),
        ArtTone::HeroMid => mix_color(skin.accent_soft, skin.header, 0.24),
        ArtTone::HeroBase => mix_color(skin.border, shadow_anchor, 0.38),
        ArtTone::Body => skin.body,
        ArtTone::Muted => skin.muted,
    }
}

pub fn style_for_tone(skin: &CliSkin, tone: ArtTone, bold: bool) -> Style {
    let mut style = Style::default().fg(color_for_tone(skin, tone));
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn paint(canvas: &mut [Vec<u8>], x: usize, y: usize, tone: u8) {
    if let Some(row) = canvas.get_mut(y)
        && let Some(cell) = row.get_mut(x)
        && tone > *cell
    {
        *cell = tone;
    }
}

fn row_to_art_line(row: &[u8]) -> ArtLine {
    let mut spans = Vec::new();
    let mut start = 0usize;
    while start < row.len() {
        let tone = row[start];
        let mut end = start + 1;
        while end < row.len() && row[end] == tone {
            end += 1;
        }
        let text = if tone == 0 {
            " ".repeat(end - start)
        } else {
            "█".repeat(end - start)
        };
        spans.push(match tone {
            0 => ArtSpan::plain(text),
            1 => ArtSpan::toned(text, ArtTone::Shadow, true),
            2 => ArtSpan::toned(text, ArtTone::Face, true),
            _ => ArtSpan::toned(text, ArtTone::Highlight, true),
        });
        start = end;
    }
    ArtLine::from_spans(spans)
}

fn compose_horizontal(left: &ArtBlock, right: &ArtBlock, gap: usize) -> ArtBlock {
    let height = left.height().max(right.height());
    let left_pad_top = (height.saturating_sub(left.height())) / 2;
    let right_pad_top = (height.saturating_sub(right.height())) / 2;
    let left_width = left.width();
    let right_width = right.width();
    let mut lines = Vec::with_capacity(height);

    for idx in 0..height {
        let mut spans = Vec::new();
        let left_idx = idx.checked_sub(left_pad_top);
        if let Some(line_idx) = left_idx.filter(|line_idx| *line_idx < left.height()) {
            spans.extend(left.lines[line_idx].spans.clone());
        } else {
            spans.push(ArtSpan::plain(" ".repeat(left_width)));
        }

        spans.push(ArtSpan::plain(" ".repeat(gap)));

        let right_idx = idx.checked_sub(right_pad_top);
        if let Some(line_idx) = right_idx.filter(|line_idx| *line_idx < right.height()) {
            spans.extend(right.lines[line_idx].spans.clone());
        } else {
            spans.push(ArtSpan::plain(" ".repeat(right_width)));
        }

        lines.push(ArtLine::from_spans(spans));
    }

    ArtBlock::new(lines)
}

fn stack_blocks(top: &ArtBlock, bottom: &ArtBlock, gap_lines: usize) -> ArtBlock {
    let width = top.width().max(bottom.width());
    let mut lines = Vec::with_capacity(top.height() + gap_lines + bottom.height());

    for line in &top.lines {
        lines.push(pad_line_to_width(line, width));
    }
    for _ in 0..gap_lines {
        lines.push(ArtLine::blank(width));
    }
    for line in &bottom.lines {
        lines.push(pad_line_to_width(line, width));
    }

    ArtBlock::new(lines)
}

fn pad_line_to_width(line: &ArtLine, width: usize) -> ArtLine {
    let current = line.width();
    if current >= width {
        return line.clone();
    }

    let mut spans = line.spans.clone();
    spans.push(ArtSpan::plain(" ".repeat(width - current)));
    ArtLine::from_spans(spans)
}

fn stronger_contrast(base: Color, one: Color, two: Color) -> Color {
    if (luminance(base) - luminance(one)).abs() >= (luminance(base) - luminance(two)).abs() {
        one
    } else {
        two
    }
}

fn darker_of(one: Color, two: Color) -> Color {
    if luminance(one) <= luminance(two) {
        one
    } else {
        two
    }
}

fn luminance(color: Color) -> f32 {
    let (r, g, b) = match color {
        Color::Rgb(r, g, b) => (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0),
        Color::Black => (0.0, 0.0, 0.0),
        Color::White => (1.0, 1.0, 1.0),
        Color::Gray => (0.5, 0.5, 0.5),
        Color::DarkGray => (0.35, 0.35, 0.35),
        Color::Red => (0.8, 0.0, 0.0),
        Color::Green => (0.0, 0.6, 0.0),
        Color::Yellow => (0.8, 0.6, 0.0),
        Color::Blue => (0.0, 0.4, 0.8),
        Color::Magenta => (0.6, 0.0, 0.6),
        Color::Cyan => (0.0, 0.6, 0.6),
        Color::LightRed => (1.0, 0.4, 0.4),
        Color::LightGreen => (0.4, 1.0, 0.4),
        Color::LightYellow => (1.0, 0.82, 0.4),
        Color::LightBlue => (0.4, 0.7, 1.0),
        Color::LightMagenta => (0.85, 0.45, 0.85),
        Color::LightCyan => (0.45, 0.95, 0.95),
        Color::Indexed(idx) => {
            let v = idx as f32 / 255.0;
            (v, v, v)
        }
        Color::Reset => (1.0, 1.0, 1.0),
    };
    (0.2126 * r) + (0.7152 * g) + (0.0722 * b)
}

const WORDMARK: &str = "THINCLAW";

fn letter_mask(ch: char) -> &'static [&'static str] {
    match ch {
        'T' => &["██████", "  ██  ", "  ██  ", "  ██  ", "  ██  ", "  ██  "],
        'H' => &["██  ██", "██  ██", "██████", "██  ██", "██  ██", "██  ██"],
        'I' => &["██████", "  ██  ", "  ██  ", "  ██  ", "  ██  ", "██████"],
        'N' => &[
            "██   ██",
            "███  ██",
            "████ ██",
            "██ ████",
            "██  ███",
            "██   ██",
        ],
        'C' => &[
            " ██████",
            "██     ",
            "██     ",
            "██     ",
            "██     ",
            " ██████",
        ],
        'L' => &["██    ", "██    ", "██    ", "██    ", "██    ", "██████"],
        'A' => &[" ████ ", "██  ██", "██  ██", "██████", "██  ██", "██  ██"],
        'W' => &[
            "██   ██",
            "██   ██",
            "██ █ ██",
            "██ █ ██",
            "███████",
            "██   ██",
        ],
        _ => &[""],
    }
}
