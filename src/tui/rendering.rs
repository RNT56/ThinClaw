//! TUI rendering methods.
//!
//! Contains all ratatui rendering logic for the TUI chat interface:
//! header, chat area with messages, input field, and status bar.

use ratatui::prelude::*;
use ratatui::widgets::*;

use super::{ChatMessage, TuiApp};

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
        let header = Line::from(vec![
            Span::styled(
                " ThinClaw ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(80, 80, 200))
                    .bold(),
            ),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(&self.model, Style::default().fg(Color::Cyan)),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("Agent: {}", self.agent_id),
                Style::default().fg(Color::Yellow),
            ),
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
                    .borders(Borders::LEFT | Borders::RIGHT)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));

        frame.render_widget(chat, area);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let input = Paragraph::new(self.input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(100, 100, 200)))
                .title(Span::styled(
                    " Message (/help for commands) ",
                    Style::default().fg(Color::Rgb(150, 150, 255)),
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
        let streaming_indicator = if self.active_stream.is_some() {
            Span::styled(" ● ", Style::default().fg(Color::Green))
        } else {
            Span::styled(" ○ ", Style::default().fg(Color::DarkGray))
        };

        let status_line = Line::from(vec![
            streaming_indicator,
            Span::styled(&self.status_text, Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(" │ {} ", self.model),
                Style::default().fg(Color::DarkGray),
            ),
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
                    count += 1; // label
                    count += text.lines().count() as u16;
                }
                ChatMessage::ToolCall { args, result, .. } => {
                    count += 2; // top + bottom border
                    if !args.is_empty() {
                        count += args.lines().take(5).count() as u16;
                    }
                    if let Some(r) = result {
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
                        Span::styled("You: ", Style::default().fg(Color::Green).bold()),
                        Span::raw(text),
                    ]));
                }
                ChatMessage::Assistant { text, model, .. } => {
                    let label = model.as_deref().unwrap_or("AI");
                    lines.push(Line::from(vec![Span::styled(
                        format!("{label}: "),
                        Style::default().fg(Color::Cyan).bold(),
                    )]));
                    // Add content lines
                    for line in text.lines() {
                        lines.push(Line::from(Span::styled(
                            format!("  {line}"),
                            Style::default().fg(Color::White),
                        )));
                    }
                }
                ChatMessage::ToolCall {
                    name,
                    args,
                    result,
                    is_error,
                } => {
                    lines.push(Line::from(Span::styled(
                        format!("  ┌─ Tool: {name} ─"),
                        Style::default().fg(Color::DarkGray),
                    )));
                    if !args.is_empty() {
                        for arg_line in args.lines().take(5) {
                            lines.push(Line::from(Span::styled(
                                format!("  │ {arg_line}"),
                                Style::default().fg(Color::DarkGray),
                            )));
                        }
                    }
                    if let Some(result) = result {
                        let color = if *is_error {
                            Color::Red
                        } else {
                            Color::DarkGray
                        };
                        for line in result.lines().take(10) {
                            lines.push(Line::from(Span::styled(
                                format!("  │ {line}"),
                                Style::default().fg(color),
                            )));
                        }
                    }
                    lines.push(Line::from(Span::styled(
                        "  └──────────────",
                        Style::default().fg(Color::DarkGray),
                    )));
                }
                ChatMessage::System { text } => {
                    for line in text.lines() {
                        lines.push(Line::from(Span::styled(
                            line,
                            Style::default().fg(Color::Yellow).italic(),
                        )));
                    }
                }
            }
            lines.push(Line::from("")); // Spacing
        }

        // Active streaming
        if let Some(stream) = &self.active_stream {
            let display = stream.display_text();
            if !display.is_empty() {
                lines.push(Line::from(vec![Span::styled(
                    format!("{}: ", self.model),
                    Style::default().fg(Color::Cyan).bold(),
                )]));
                for line in display.lines() {
                    lines.push(Line::from(Span::styled(
                        format!("  {line}"),
                        Style::default().fg(Color::White),
                    )));
                }
                lines.push(Line::from(Span::styled(
                    "  ▊",
                    Style::default().fg(Color::Cyan),
                )));
            } else {
                // Show thinking indicator
                lines.push(Line::from(Span::styled(
                    "  ⠋ Thinking...",
                    Style::default().fg(Color::Cyan),
                )));
            }
        }

        Text::from(lines)
    }
}
