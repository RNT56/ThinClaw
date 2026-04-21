//! TUI rendering methods.
//!
//! Contains all ratatui rendering logic for the TUI chat interface:
//! header, chat area with messages, input field, and status bar.

use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::branding::art::{best_wordmark_block, hero_block};

use super::{ChatMessage, TuiApp};

impl TuiApp {
    // ── Rendering ────────────────────────────────────────────────────

    pub(super) fn render(&mut self, frame: &mut Frame) {
        let logo = best_wordmark_block(frame.area().width.saturating_sub(4) as usize);
        let header_height = if let Some(logo) = &logo {
            logo.height() as u16 + 2
        } else {
            1
        };
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(header_height), Constraint::Min(5)])
            .split(frame.area());

        self.render_header(frame, outer[0], logo);

        let hero = hero_block(&self.skin);
        let hero_width = hero
            .as_ref()
            .map(|art| (art.width() as u16).saturating_add(4).clamp(18, 28))
            .unwrap_or(0);
        let show_hero = hero.is_some() && outer[1].width >= hero_width.saturating_add(48);

        let body_chunks = if show_hero {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(hero_width), Constraint::Min(36)])
                .split(outer[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Min(1)])
                .split(outer[1])
        };

        let main_area = if show_hero {
            body_chunks[1]
        } else {
            body_chunks[0]
        };
        if show_hero {
            self.render_hero(frame, body_chunks[0]);
        }

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),    // Chat area
                Constraint::Length(3), // Input
                Constraint::Length(1), // Status bar
            ])
            .split(main_area);

        self.render_chat(frame, main_chunks[0]);
        self.render_input(frame, main_chunks[1]);
        self.render_status(frame, main_chunks[2]);
    }

    fn render_header(
        &self,
        frame: &mut Frame,
        area: Rect,
        logo: Option<crate::branding::art::ArtBlock>,
    ) {
        let status = if self.active_stream.is_some() {
            "live"
        } else {
            "idle"
        };

        let Some(logo) = logo else {
            let header = Line::from(vec![
                Span::styled(" ThinClaw ", self.skin.title_style()),
                Span::styled(format!(" {}", self.skin.name), self.skin.accent_style()),
                Span::styled("│", self.skin.border_soft_style()),
                Span::styled(format!(" model {}", self.model), self.skin.body_style()),
                Span::styled("│", self.skin.border_soft_style()),
                Span::styled(
                    format!(" agent {}", self.agent_id),
                    self.skin.accent_soft_style(),
                ),
                Span::styled("│", self.skin.border_soft_style()),
                Span::styled(format!(" {}", status), self.skin.muted_style()),
                Span::styled(" ", self.skin.muted_style()),
                Span::styled(&self.status_text, self.skin.muted_style()),
            ]);
            frame.render_widget(Paragraph::new(header), area);
            return;
        };

        let mut lines = logo.to_ratatui_lines(&self.skin);
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", self.skin.name), self.skin.title_style()),
            Span::styled("│", self.skin.border_soft_style()),
            Span::styled(format!(" model {}", self.model), self.skin.body_style()),
            Span::styled(" │ ", self.skin.border_soft_style()),
            Span::styled(
                format!("agent {}", self.agent_id),
                self.skin.accent_soft_style(),
            ),
            Span::styled(" │ ", self.skin.border_soft_style()),
            Span::styled(format!("{status}"), self.skin.muted_style()),
            Span::styled(" ", self.skin.muted_style()),
            Span::styled(&self.status_text, self.skin.muted_style()),
        ]));

        let header = Paragraph::new(Text::from(lines)).block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(self.skin.border_style()),
        );
        frame.render_widget(header, area);
    }

    fn render_hero(&self, frame: &mut Frame, area: Rect) {
        let Some(hero) = hero_block(&self.skin) else {
            return;
        };

        let mut lines = Vec::new();
        let inner_height = area.height.saturating_sub(2) as usize;
        let top_padding = inner_height.saturating_sub(hero.height()) / 2;
        for _ in 0..top_padding {
            lines.push(Line::from(""));
        }
        lines.extend(hero.to_ratatui_lines(&self.skin));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            self.skin
                .tagline()
                .unwrap_or("Skin-driven operator deck")
                .to_string(),
            self.skin.muted_style(),
        )));

        let panel = Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.skin.border_style())
                    .title(Span::styled(
                        format!(" {} sigil ", self.skin.name),
                        self.skin.accent_style(),
                    )),
            );
        frame.render_widget(panel, area);
    }

    fn render_chat(&mut self, frame: &mut Frame, area: Rect) {
        let line_count = self.count_chat_lines();
        self.total_chat_lines = line_count;

        let visible_height = area.height.saturating_sub(2);
        if self.scroll_offset == u16::MAX || self.total_chat_lines <= visible_height {
            self.scroll_offset = self.total_chat_lines.saturating_sub(visible_height);
        }

        let chat_text = self.build_chat_text();
        let chat = Paragraph::new(chat_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.skin.border_style())
                    .title(Span::styled(" Activity deck ", self.skin.muted_style())),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));

        frame.render_widget(chat, area);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let input = Paragraph::new(self.input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(self.skin.accent_style())
                .title(Span::styled(
                    format!(" Command bay ({}) ", self.skin.prompt_symbol()),
                    self.skin.title_style(),
                )),
        );
        frame.render_widget(input, area);

        #[allow(clippy::cast_possible_truncation)]
        frame.set_cursor_position((area.x + self.cursor_pos as u16 + 1, area.y + 1));
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let (indicator, indicator_style) = if self.active_stream.is_some() {
            ("●", self.skin.good_style())
        } else {
            ("○", self.skin.border_soft_style())
        };

        let status_line = Line::from(vec![
            Span::styled(format!(" {} ", indicator), indicator_style),
            Span::styled(&self.status_text, self.skin.muted_style()),
            Span::styled(" · ", self.skin.border_soft_style()),
            Span::styled(&self.model, self.skin.accent_soft_style()),
            Span::styled(" · ", self.skin.border_soft_style()),
            Span::styled(format!("agent {}", self.agent_id), self.skin.muted_style()),
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
                    count += 2;
                    count += text.lines().count() as u16;
                }
                ChatMessage::ToolCall { args, result, .. } => {
                    count += 2;
                    if !args.is_empty() {
                        count += 1;
                        count += args.lines().take(5).count() as u16;
                    }
                    if let Some(r) = result {
                        count += 1;
                        count += r.lines().take(10).count() as u16;
                    }
                }
                ChatMessage::System { text } => {
                    count += text.lines().count() as u16;
                }
            }
            count += 1;
        }
        if self.active_stream.is_some() {
            count += 5;
        }
        count
    }

    fn build_chat_text(&self) -> Text<'_> {
        let mut lines = Vec::new();

        for msg in &self.messages {
            match msg {
                ChatMessage::User { text } => {
                    lines.push(Line::from(vec![
                        Span::styled("╭ you ", self.skin.good_style()),
                        Span::styled("│ ", self.skin.border_soft_style()),
                        Span::styled(text, self.skin.body_style()),
                    ]));
                }
                ChatMessage::Assistant { text, model, .. } => {
                    let label = model.as_deref().unwrap_or("AI");
                    lines.push(Line::from(vec![
                        Span::styled(format!("╭ {label} "), self.skin.accent_style()),
                        Span::styled("│ ", self.skin.border_soft_style()),
                        Span::styled("response", self.skin.muted_style()),
                    ]));
                    for line in text.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", self.skin.border_soft_style()),
                            Span::styled(line, self.skin.body_style()),
                        ]));
                    }
                    lines.push(Line::from(vec![
                        Span::styled("╰", self.skin.border_soft_style()),
                        Span::styled(" next turn ready ", self.skin.muted_style()),
                    ]));
                }
                ChatMessage::ToolCall {
                    name,
                    args,
                    result,
                    is_error,
                } => {
                    let tool_label = self.skin.tool_label(name);
                    let header_style = if *is_error {
                        self.skin.bad_style()
                    } else {
                        self.skin.warn_style()
                    };
                    lines.push(Line::from(vec![
                        Span::styled("╭ ", header_style),
                        Span::styled(format!("tool {tool_label}"), header_style),
                    ]));
                    if !args.is_empty() {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", self.skin.border_soft_style()),
                            Span::styled("input", self.skin.muted_style()),
                        ]));
                        for arg_line in args.lines().take(5) {
                            lines.push(Line::from(vec![
                                Span::styled("│ ", self.skin.border_soft_style()),
                                Span::styled(arg_line, self.skin.body_style()),
                            ]));
                        }
                    }
                    if let Some(result) = result {
                        lines.push(Line::from(vec![
                            Span::styled("│ ", self.skin.border_soft_style()),
                            Span::styled("result", self.skin.muted_style()),
                        ]));
                        let result_style = if *is_error {
                            self.skin.bad_style()
                        } else {
                            self.skin.muted_style()
                        };
                        for line in result.lines().take(10) {
                            lines.push(Line::from(vec![
                                Span::styled("│ ", self.skin.border_soft_style()),
                                Span::styled(line, result_style),
                            ]));
                        }
                    }
                    lines.push(Line::from(vec![
                        Span::styled("╰", self.skin.border_soft_style()),
                        Span::styled(" tool complete ", self.skin.muted_style()),
                    ]));
                }
                ChatMessage::System { text } => {
                    for line in text.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("• ", self.skin.border_soft_style()),
                            Span::styled(line, self.skin.muted_style().italic()),
                        ]));
                    }
                }
            }
            lines.push(Line::from(""));
        }

        if let Some(stream) = &self.active_stream {
            let display = stream.display_text();
            if !display.is_empty() {
                let display_lines: Vec<String> = display.lines().map(ToOwned::to_owned).collect();
                lines.push(Line::from(vec![
                    Span::styled("╭ ", self.skin.accent_style()),
                    Span::styled(format!("stream {}", self.model), self.skin.accent_style()),
                ]));
                for line in display_lines {
                    lines.push(Line::from(vec![
                        Span::styled("│ ", self.skin.border_soft_style()),
                        Span::styled(line, self.skin.body_style()),
                    ]));
                }
                lines.push(Line::from(vec![
                    Span::styled("╰", self.skin.border_soft_style()),
                    Span::styled(" still working ", self.skin.muted_style()),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("╭ ", self.skin.accent_style()),
                    Span::styled("thinking", self.skin.accent_style()),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("│ ", self.skin.border_soft_style()),
                    Span::styled("holding the line...", self.skin.muted_style()),
                ]));
                lines.push(Line::from(vec![
                    Span::styled("╰", self.skin.border_soft_style()),
                    Span::styled(" stay with me ", self.skin.muted_style()),
                ]));
            }
        }

        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "No messages yet.",
                self.skin.muted_style(),
            )));
        }

        Text::from(lines)
    }
}
