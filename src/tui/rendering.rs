//! TUI rendering methods.
//!
//! Contains all ratatui rendering logic for the TUI chat interface:
//! header, chat area with messages, input field, and status bar.

use ratatui::prelude::*;
use ratatui::widgets::*;

use super::{ChatMessage, TuiApp};

const COLOR_ACCENT: Color = Color::Rgb(106, 166, 255);
const COLOR_ACCENT_SOFT: Color = Color::Rgb(162, 199, 255);
const COLOR_BORDER: Color = Color::Rgb(82, 96, 125);
const COLOR_BORDER_SOFT: Color = Color::Rgb(54, 66, 88);
const COLOR_BODY: Color = Color::Rgb(236, 241, 248);
const COLOR_MUTED: Color = Color::Rgb(145, 156, 176);
const COLOR_GOOD: Color = Color::Rgb(120, 217, 173);
const COLOR_WARN: Color = Color::Rgb(244, 196, 104);
const COLOR_BAD: Color = Color::Rgb(255, 128, 128);

fn cockpit_border() -> Style {
    Style::default().fg(COLOR_BORDER)
}

fn cockpit_border_soft() -> Style {
    Style::default().fg(COLOR_BORDER_SOFT)
}

fn cockpit_title() -> Style {
    Style::default().fg(COLOR_BODY).bg(COLOR_ACCENT).bold()
}

fn cockpit_accent() -> Style {
    Style::default().fg(COLOR_ACCENT_SOFT).bold()
}

fn cockpit_body() -> Style {
    Style::default().fg(COLOR_BODY)
}

fn cockpit_muted() -> Style {
    Style::default().fg(COLOR_MUTED)
}

impl TuiApp {
    // ── Rendering ────────────────────────────────────────────────────

    pub(super) fn render(&mut self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Min(5),    // Chat area
                Constraint::Length(3), // Input
                Constraint::Length(1), // Status bar
            ])
            .split(frame.area());

        self.render_header(frame, chunks[0]);
        self.render_chat(frame, chunks[1]);
        self.render_input(frame, chunks[2]);
        self.render_status(frame, chunks[3]);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let status = if self.active_stream.is_some() {
            "live"
        } else {
            "idle"
        };

        let header = Line::from(vec![
            Span::styled(" ThinClaw ", cockpit_title()),
            Span::styled(" cockpit ", cockpit_accent()),
            Span::styled("│", cockpit_border_soft()),
            Span::styled(format!(" model {}", self.model), cockpit_body()),
            Span::styled("│", cockpit_border_soft()),
            Span::styled(format!(" agent {}", self.agent_id), cockpit_accent()),
            Span::styled("│", cockpit_border_soft()),
            Span::styled(format!(" {}", status), cockpit_muted()),
            Span::styled(" ", cockpit_muted()),
            Span::styled(&self.status_text, cockpit_muted()),
        ]);

        frame.render_widget(Paragraph::new(header), area);
    }

    fn render_chat(&mut self, frame: &mut Frame, area: Rect) {
        // Count lines first (no borrow conflict)
        let line_count = self.count_chat_lines();
        self.total_chat_lines = line_count;

        // Clamp scroll to valid range
        let visible_height = area.height.saturating_sub(2); // borders
        if self.scroll_offset == u16::MAX || self.total_chat_lines <= visible_height {
            // Auto-scroll to bottom
            self.scroll_offset = self.total_chat_lines.saturating_sub(visible_height);
        }

        let chat_text = self.build_chat_text();
        let chat = Paragraph::new(chat_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(cockpit_border())
                    .title(Span::styled(" Activity deck ", cockpit_muted())),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));

        frame.render_widget(chat, area);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let input = Paragraph::new(self.input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(cockpit_accent())
                .title(Span::styled(
                    " Command bay (/help for controls) ",
                    cockpit_title(),
                )),
        );
        frame.render_widget(input, area);

        // Position cursor
        // cursor_pos is a char index — for monospace terminals this maps 1:1
        // to display columns for ASCII. CJK wide chars would need unicode-width,
        // but this is a reasonable approximation for typical input.
        #[allow(clippy::cast_possible_truncation)]
        frame.set_cursor_position((area.x + self.cursor_pos as u16 + 1, area.y + 1));
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let (indicator, indicator_style) = if self.active_stream.is_some() {
            ("●", Style::default().fg(COLOR_GOOD))
        } else {
            ("○", Style::default().fg(COLOR_BORDER_SOFT))
        };

        let status_line = Line::from(vec![
            Span::styled(format!(" {} ", indicator), indicator_style),
            Span::styled(&self.status_text, cockpit_muted()),
            Span::styled(" · ", cockpit_border_soft()),
            Span::styled(&self.model, cockpit_accent()),
            Span::styled(" · ", cockpit_border_soft()),
            Span::styled(format!("agent {}", self.agent_id), cockpit_muted()),
        ]);
        frame.render_widget(Paragraph::new(status_line), area);
    }

    /// Count total lines for scroll calculation (no borrow of text data).
    fn count_chat_lines(&self) -> u16 {
        let mut count: u16 = 0;
        for msg in &self.messages {
            match msg {
                ChatMessage::User { .. } => count += 1,
                ChatMessage::Assistant { text, .. } => {
                    count += 2; // label + closing line
                    count += text.lines().count() as u16;
                }
                ChatMessage::ToolCall { args, result, .. } => {
                    count += 2; // label + closing line
                    if !args.is_empty() {
                        count += 1; // args label
                        count += args.lines().take(5).count() as u16;
                    }
                    if let Some(r) = result {
                        count += 1; // result label
                        count += r.lines().take(10).count() as u16;
                    }
                }
                ChatMessage::System { text } => {
                    count += text.lines().count() as u16;
                }
            }
            count += 1; // spacing
        }
        if self.active_stream.is_some() {
            count += 5; // approximate
        }
        count
    }

    fn build_chat_text(&self) -> Text<'_> {
        let mut lines = Vec::new();

        for msg in &self.messages {
            match msg {
                ChatMessage::User { text } => {
                    lines.push(Line::from(vec![
                        Span::styled("╭ you ", Style::default().fg(COLOR_GOOD).bold()),
                        Span::styled("│ ", cockpit_border_soft()),
                        Span::styled(text, cockpit_body()),
                    ]));
                }
                ChatMessage::Assistant { text, model, .. } => {
                    let label = model.as_deref().unwrap_or("AI");
                    lines.push(Line::from(vec![
                        Span::styled(format!("╭ {label} "), cockpit_accent()),
                        Span::styled("│ ", cockpit_border_soft()),
                        Span::styled("response", cockpit_muted()),
                    ]));
                    for line in text.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", cockpit_border_soft()),
                            Span::styled(line, cockpit_body()),
                        ]));
                    }
                    lines.push(Line::from(vec![
                        Span::styled("╰", cockpit_border_soft()),
                        Span::styled(" next turn ready ", cockpit_muted()),
                    ]));
                }
                ChatMessage::ToolCall {
                    name,
                    args,
                    result,
                    is_error,
                } => {
                    let header_style = if *is_error {
                        Style::default().fg(COLOR_BAD).bold()
                    } else {
                        Style::default().fg(COLOR_WARN).bold()
                    };
                    lines.push(Line::from(vec![
                        Span::styled("╭ ", header_style),
                        Span::styled(format!("tool {name}"), header_style),
                    ]));
                    if !args.is_empty() {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", cockpit_border_soft()),
                            Span::styled("input", cockpit_muted()),
                        ]));
                        for arg_line in args.lines().take(5) {
                            lines.push(Line::from(vec![
                                Span::styled("│ ", cockpit_border_soft()),
                                Span::styled(arg_line, cockpit_body()),
                            ]));
                        }
                    }
                    if let Some(result) = result {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", cockpit_border_soft()),
                            Span::styled("result", cockpit_muted()),
                        ]));
                        let color = if *is_error { COLOR_BAD } else { COLOR_MUTED };
                        for line in result.lines().take(10) {
                            lines.push(Line::from(vec![
                                Span::styled("│ ", cockpit_border_soft()),
                                Span::styled(line, Style::default().fg(color)),
                            ]));
                        }
                    }
                    lines.push(Line::from(vec![
                        Span::styled("╰", cockpit_border_soft()),
                        Span::styled(" tool complete ", cockpit_muted()),
                    ]));
                }
                ChatMessage::System { text } => {
                    for line in text.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("• ", cockpit_border_soft()),
                            Span::styled(line, cockpit_muted().italic()),
                        ]));
                    }
                }
            }
            lines.push(Line::from("")); // Spacing
        }

        // Active streaming
        if let Some(stream) = &self.active_stream {
            let display = stream.display_text();
            if !display.is_empty() {
                let display_lines: Vec<String> = display.lines().map(ToOwned::to_owned).collect();
                lines.push(Line::from(vec![
                    Span::styled("╭ ", cockpit_accent()),
                    Span::styled(format!("stream {}", self.model), cockpit_accent()),
                ]));
                for line in display_lines {
                    lines.push(Line::from(vec![
                        Span::styled("│ ", cockpit_border_soft()),
                        Span::styled(line, cockpit_body()),
                    ]));
                }
                lines.push(Line::from(vec![
                    Span::styled("╰", cockpit_border_soft()),
                    Span::styled(" still working ", cockpit_muted()),
                ]));
            } else {
                // Show thinking indicator
                lines.push(Line::from(vec![
                    Span::styled("╭ ", cockpit_accent()),
                    Span::styled("thinking", cockpit_accent()),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("│ ", cockpit_border_soft()),
                    Span::styled("holding the line...", cockpit_muted()),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("╰", cockpit_border_soft()),
                    Span::styled(" stay with me ", cockpit_muted()),
                ]));
            }
        }

        Text::from(lines)
    }
}
