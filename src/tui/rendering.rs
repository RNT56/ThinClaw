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
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(5)])
            .split(frame.area());

        self.render_header(frame, outer[0], None);
        let main_area = outer[1];

        // Dynamic input height: grows with content, clamped to 3..8 lines
        let input_line_count = self.textarea.lines().len() as u16;
        let input_height = (input_line_count + 2).clamp(3, 8);

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),               // Chat area
                Constraint::Length(input_height), // Input (dynamic)
                Constraint::Length(1),            // Status bar
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
            frame.render_widget(
                Paragraph::new(header).alignment(self.skin.header_alignment),
                area,
            );
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
            Span::styled(status.to_string(), self.skin.muted_style()),
            Span::styled(" ", self.skin.muted_style()),
            Span::styled(&self.status_text, self.skin.muted_style()),
        ]));

        let header = Paragraph::new(Text::from(lines))
            .alignment(self.skin.header_alignment)
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(self.skin.border_style())
                    .border_type(self.skin.tui_border_type()),
            );
        frame.render_widget(header, area);
    }

    fn render_chat(&mut self, frame: &mut Frame, area: Rect) {
        let wrap = Wrap { trim: false };
        let chat_text = sanitize_terminal_text(self.build_chat_text());
        let line_count = wrapped_text_rows(&chat_text, area.width.saturating_sub(2));
        self.total_chat_lines = line_count;

        let visible_height = area.height.saturating_sub(2);
        if self.scroll_offset == u16::MAX || self.total_chat_lines <= visible_height {
            self.scroll_offset = self.total_chat_lines.saturating_sub(visible_height);
        }

        let chat = Paragraph::new(chat_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(self.skin.border_style())
                    .border_type(self.skin.tui_border_type())
                    .title(Span::styled(" Conversation ", self.skin.muted_style())),
            )
            .wrap(wrap)
            .scroll((self.scroll_offset, 0));

        frame.render_widget(chat, area);
    }

    fn render_input(&mut self, frame: &mut Frame, area: Rect) {
        let (border_style, title) = if let Some(prompt) = self.pending_approvals.front() {
            let age = prompt.received_at.elapsed().as_secs();
            let detail = format!(
                "{} — {} • args {}B • {}s • {}",
                prompt.tool_name,
                prompt.description,
                prompt.redacted_parameters.len(),
                age,
                prompt.request_id
            );
            let detail = detail.chars().take(72).collect::<String>();
            (
                self.skin.warn_style(),
                format!(
                    " Approval: {detail} — yes/no/always ({}) ",
                    self.skin.prompt_symbol()
                ),
            )
        } else {
            (
                self.skin.accent_style(),
                format!(" Message ({}) ", self.skin.prompt_symbol()),
            )
        };
        self.textarea.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .border_type(self.skin.tui_border_type())
                .title(Span::styled(title, self.skin.title_style())),
        );
        frame.render_widget(&self.textarea, area);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let indicator_spans = if self.active_stream.is_some() {
            self.spinner.to_spans(&self.skin)
        } else if !self.pending_approvals.is_empty() {
            vec![Span::styled("⚠ ", self.skin.warn_style().bold())]
        } else {
            vec![Span::styled("○ ", self.skin.border_soft_style())]
        };

        let mut status_spans = Vec::new();
        status_spans.push(Span::raw(" "));
        status_spans.extend(indicator_spans);
        status_spans.push(Span::styled(" ", Style::default()));
        status_spans.push(Span::styled(&self.status_text, self.skin.muted_style()));
        status_spans.push(Span::styled(" · ", self.skin.border_soft_style()));
        status_spans.push(Span::styled(&self.model, self.skin.accent_soft_style()));
        status_spans.push(Span::styled(" · ", self.skin.border_soft_style()));
        status_spans.push(Span::styled(
            format!("agent {}", self.agent_id),
            self.skin.muted_style(),
        ));

        // Show idle time when not streaming
        if self.active_stream.is_none() {
            let idle_secs = self.last_activity.elapsed().as_secs();
            if idle_secs >= 60 {
                let mins = idle_secs / 60;
                status_spans.push(Span::styled(" · ", self.skin.border_soft_style()));
                status_spans.push(Span::styled(
                    format!("{mins}m idle"),
                    self.skin.muted_style(),
                ));
            }
        }

        // When gradient is enabled, color each character of the status line
        // with a left-to-right accent→border gradient for a premium feel.
        if self.skin.status_gradient && area.width > 0 {
            // Build a gradient-tinted version of the status content
            let plain: String = status_spans.iter().map(|s| s.content.as_ref()).collect();
            let mut gradient_spans = Vec::new();
            for (i, ch) in plain.chars().enumerate() {
                let ratio = i as f32 / plain.len().max(1) as f32;
                let color = self
                    .skin
                    .gradient_at(self.skin.accent, self.skin.border, ratio);
                gradient_spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
            }
            frame.render_widget(Paragraph::new(Line::from(gradient_spans)), area);
        } else {
            frame.render_widget(Paragraph::new(Line::from(status_spans)), area);
        }
    }

    fn markdown_spans(&self, line: &str) -> Vec<Span<'static>> {
        let trimmed = line.trim_start();
        if let Some(content) = trimmed.strip_prefix("### ") {
            return vec![Span::styled(
                content.to_string(),
                self.skin.accent_soft_style().bold(),
            )];
        }
        if let Some(content) = trimmed
            .strip_prefix("## ")
            .or_else(|| trimmed.strip_prefix("# "))
        {
            return vec![Span::styled(
                content.to_string(),
                self.skin.accent_style().bold(),
            )];
        }
        if let Some(content) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            return vec![
                Span::styled("• ".to_string(), self.skin.accent_style()),
                Span::styled(content.to_string(), self.skin.body_style()),
            ];
        }
        if let Some(content) = trimmed.strip_prefix("> ") {
            return vec![Span::styled(
                content.to_string(),
                self.skin.muted_style().italic(),
            )];
        }
        if trimmed.starts_with('`') && trimmed.ends_with('`') && trimmed.len() >= 2 {
            return vec![Span::styled(
                trimmed[1..trimmed.len() - 1].to_string(),
                self.skin.warn_style(),
            )];
        }
        vec![Span::styled(line.to_string(), self.skin.body_style())]
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
                        let mut spans = vec![Span::styled(
                            "│ ".to_string(),
                            self.skin.border_soft_style(),
                        )];
                        spans.extend(self.markdown_spans(line));
                        lines.push(Line::from(spans));
                    }
                    lines.push(Line::from(vec![
                        Span::styled("╰", self.skin.border_soft_style()),
                        Span::styled(" next turn ready ", self.skin.muted_style()),
                    ]));
                }
                ChatMessage::ToolCall {
                    invocation_id,
                    name,
                    args,
                    result,
                    is_error,
                    completed,
                    duration_ms,
                    artifact_count,
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
                        Span::styled(
                            format!(" · {}", invocation_id.as_str()),
                            self.skin.muted_style(),
                        ),
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
                        Span::styled(
                            format!(
                                " {}{}{} ",
                                if *completed { "complete" } else { "running" },
                                duration_ms
                                    .map(|duration| format!(" · {duration}ms"))
                                    .unwrap_or_default(),
                                if *artifact_count > 0 {
                                    format!(" · {artifact_count} artifact(s)")
                                } else {
                                    String::new()
                                }
                            ),
                            self.skin.muted_style(),
                        ),
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
                ChatMessage::Info { text } => {
                    for line in text.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("✓ ", self.skin.good_style()),
                            Span::styled(line, self.skin.good_style()),
                        ]));
                    }
                }
                ChatMessage::Warning { text } => {
                    for line in text.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("⚠ ", self.skin.warn_style().bold()),
                            Span::styled(line, self.skin.warn_style()),
                        ]));
                    }
                }
                ChatMessage::Error { text } => {
                    for line in text.lines() {
                        lines.push(Line::from(vec![
                            Span::styled("✖ ", self.skin.bad_style().bold()),
                            Span::styled(line, self.skin.bad_style()),
                        ]));
                    }
                }
                ChatMessage::AgentNote { content, note_type } => {
                    let (header_label, header_style) = match note_type.as_str() {
                        "warning" => ("agent warning", self.skin.warn_style()),
                        "question" => ("agent question", self.skin.accent_style()),
                        "interim_result" => ("agent interim result", self.skin.good_style()),
                        _ => ("agent note", self.skin.accent_soft_style()),
                    };
                    lines.push(Line::from(vec![
                        Span::styled("┌─ ", header_style),
                        Span::styled(header_label, header_style),
                        Span::styled(" ─", header_style),
                    ]));
                    for line in content.lines().take(20) {
                        let mut spans = vec![Span::styled(
                            "│ ".to_string(),
                            self.skin.border_soft_style(),
                        )];
                        spans.extend(self.markdown_spans(line));
                        lines.push(Line::from(spans));
                    }
                    lines.push(Line::from(Span::styled(
                        "└────────────────────────────────",
                        self.skin.muted_style(),
                    )));
                }
                ChatMessage::SubagentCard {
                    agent_id,
                    name,
                    detail,
                    success,
                } => {
                    let header_style = match success {
                        Some(true) => self.skin.good_style(),
                        Some(false) => self.skin.bad_style(),
                        None => self.skin.accent_style(),
                    };
                    let marker = match success {
                        Some(true) => "✓",
                        Some(false) => "✖",
                        None => "▶",
                    };
                    lines.push(Line::from(vec![
                        Span::styled("┌─ ", header_style),
                        Span::styled(format!("{marker} sub-agent: {name}"), header_style),
                        Span::styled(format!(" · {agent_id}"), self.skin.muted_style()),
                    ]));
                    lines.push(Line::from(vec![
                        Span::styled("│ ", self.skin.border_soft_style()),
                        Span::styled(detail, self.skin.muted_style()),
                    ]));
                    lines.push(Line::from(Span::styled(
                        "└────────────────────────────────",
                        self.skin.muted_style(),
                    )));
                }
            }
            lines.push(Line::from(""));
        }

        if let Some(stream) = &self.active_stream {
            let display = stream.display_text();
            if !display.is_empty() {
                let display_lines: Vec<String> = display.lines().map(ToOwned::to_owned).collect();
                let mut header_spans = vec![Span::styled("╭ ", self.skin.accent_style())];
                header_spans.extend(self.spinner.to_spans(&self.skin));
                header_spans.push(Span::styled(
                    format!(" stream {}", self.model),
                    self.skin.accent_style(),
                ));
                lines.push(Line::from(header_spans));
                for line in display_lines {
                    let mut spans = vec![Span::styled(
                        "│ ".to_string(),
                        self.skin.border_soft_style(),
                    )];
                    spans.extend(self.markdown_spans(&line));
                    lines.push(Line::from(spans));
                }
                lines.push(Line::from(vec![
                    Span::styled("╰", self.skin.border_soft_style()),
                    Span::styled(" still working ", self.skin.muted_style()),
                ]));
            } else {
                let mut header_spans = vec![Span::styled("╭ ", self.skin.accent_style())];
                header_spans.extend(self.spinner.to_spans(&self.skin));
                lines.push(Line::from(header_spans));
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

/// Strip every terminal control byte from dynamic transcript content while
/// preserving ordinary newlines/tabs as harmless layout characters. In
/// particular this removes ESC, BEL, C1 controls, and therefore ANSI/OSC/DCS
/// introducers before crossterm ever sees them.
pub(super) fn sanitize_terminal_text(text: Text<'_>) -> Text<'static> {
    Text::from(
        text.lines
            .into_iter()
            .map(|line| {
                Line::from(
                    line.spans
                        .into_iter()
                        .map(|span| {
                            let safe = span
                                .content
                                .chars()
                                .filter(|character| {
                                    *character == '\n'
                                        || *character == '\t'
                                        || (!character.is_control()
                                            && !matches!(*character as u32, 0x80..=0x9f))
                                })
                                .collect::<String>();
                            Span::styled(safe, span.style)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>(),
    )
}

fn wrapped_text_rows(text: &Text<'_>, width: u16) -> u16 {
    let width = usize::from(width.max(1));
    text.lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum::<usize>()
        .min(u16::MAX as usize) as u16
}
