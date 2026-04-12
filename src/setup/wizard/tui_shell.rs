use std::{io, time::Duration};

use crossterm::{
    ExecutableCommand, cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use super::{
    ReadinessSummary, SetupError, SetupWizard, StepDescriptor, StepStatus, ValidationLevel,
    WizardPhaseId, WizardPlan,
};
use crate::setup::prompts::tui_prompt_session_active;

pub(super) struct OnboardingTuiShell {
    plan: WizardPlan,
    active_phase: Option<WizardPhaseId>,
}

impl OnboardingTuiShell {
    pub(super) fn new(plan: WizardPlan) -> Self {
        Self {
            plan,
            active_phase: None,
        }
    }

    pub(super) fn show_intro(&mut self, wizard: &SetupWizard) -> Result<(), SetupError> {
        self.present_view(
            wizard,
            Some((
                "Cockpit Overview",
                "Progress is saved continuously while you configure.",
            )),
            &[
                "Start with a setup profile, then work through each phase in order.",
                "The right panel keeps a live launch-readiness view and highlights anything that still needs care.",
                "Press Enter to begin, or Esc to leave onboarding for now.",
            ],
        )
    }

    pub(super) fn show_step(
        &mut self,
        wizard: &SetupWizard,
        descriptor: &StepDescriptor,
        current: usize,
        total: usize,
    ) -> Result<(), SetupError> {
        self.active_phase = Some(descriptor.phase_id);
        let title = format!("Step {current}/{total}: {}", descriptor.title);
        let header = (title.as_str(), descriptor.why_this_matters);
        let mut body = vec![descriptor.description];
        if let Some(recommended) = descriptor.recommended {
            body.push(recommended);
        }
        body.push("Press Enter to open this step.");
        self.present_view(wizard, Some(header), &body)
    }

    pub(super) fn show_step_result(
        &mut self,
        wizard: &SetupWizard,
        descriptor: &StepDescriptor,
        status: StepStatus,
    ) -> Result<(), SetupError> {
        self.active_phase = Some(descriptor.phase_id);
        let detail = match status {
            StepStatus::Completed => "Completed and recorded.",
            StepStatus::Skipped => "Skipped for this run.",
            StepStatus::NeedsAttention => "Completed with follow-up work queued.",
            StepStatus::InProgress => "Still in progress.",
            StepStatus::Pending => "Still pending.",
        };
        self.present_view(
            wizard,
            Some((descriptor.title, detail)),
            &["Press Enter to continue to the next step."],
        )
    }

    pub(super) fn show_completion(&mut self, wizard: &SetupWizard) -> Result<(), SetupError> {
        self.present_view(
            wizard,
            Some((
                "Launch Summary",
                "ThinClaw will now continue into its normal bootstrap path.",
            )),
            &[
                "Your readiness notes and follow-ups are saved in settings.",
                "Press Enter to complete onboarding and continue startup.",
            ],
        )
    }

    fn present_view(
        &mut self,
        wizard: &SetupWizard,
        header: Option<(&str, &str)>,
        body_lines: &[&str],
    ) -> Result<(), SetupError> {
        let shared_session = tui_prompt_session_active();
        if !shared_session {
            enable_raw_mode().map_err(SetupError::Io)?;
            io::stdout()
                .execute(EnterAlternateScreen)
                .map_err(SetupError::Io)?;
        }
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).map_err(SetupError::Io)?;
        terminal.hide_cursor().map_err(SetupError::Io)?;

        let result = self.event_loop(&mut terminal, wizard, header, body_lines);

        if !shared_session {
            disable_raw_mode().map_err(SetupError::Io)?;
            io::stdout()
                .execute(LeaveAlternateScreen)
                .map_err(SetupError::Io)?;
            io::stdout().execute(cursor::Show).map_err(SetupError::Io)?;
            terminal.show_cursor().map_err(SetupError::Io)?;
        }

        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        wizard: &SetupWizard,
        header: Option<(&str, &str)>,
        body_lines: &[&str],
    ) -> Result<(), SetupError> {
        loop {
            terminal
                .draw(|frame| self.render(frame, wizard, header, body_lines))
                .map_err(SetupError::Io)?;

            if event::poll(Duration::from_millis(250)).map_err(SetupError::Io)?
                && let Event::Key(key) = event::read().map_err(SetupError::Io)?
            {
                match (key.modifiers, key.code) {
                    (_, KeyCode::Enter) => return Ok(()),
                    (_, KeyCode::Esc) => return Err(SetupError::Cancelled),
                    (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                        return Err(SetupError::Cancelled);
                    }
                    _ => {}
                }
            }
        }
    }

    fn render(
        &self,
        frame: &mut Frame,
        wizard: &SetupWizard,
        header: Option<(&str, &str)>,
        body_lines: &[&str],
    ) {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(4),
                Constraint::Min(12),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(frame.area());

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(28),
                Constraint::Percentage(42),
                Constraint::Percentage(30),
            ])
            .split(outer[1]);

        frame.render_widget(Clear, frame.area());
        self.render_header(frame, outer[0], header);
        self.render_phases(frame, columns[0], wizard);
        self.render_focus(frame, columns[1], body_lines);
        self.render_summary(frame, columns[2], wizard);
        self.render_footer(frame, outer[2]);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect, header: Option<(&str, &str)>) {
        let (title, subtitle) = header.unwrap_or(("ThinClaw Humanist Cockpit", ""));
        let mut text = vec![Line::from(vec![
            Span::styled("ThinClaw ", Style::default().fg(Color::Cyan).bold()),
            Span::styled(title, Style::default().fg(Color::White).bold()),
        ])];
        if !subtitle.is_empty() {
            text.push(Line::from(Span::styled(
                subtitle,
                Style::default().fg(Color::DarkGray),
            )));
        }
        text.push(Line::from(vec![
            Span::styled(
                "Enter",
                Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
            ),
            Span::raw(" continue "),
            Span::styled(
                "Esc",
                Style::default().fg(Color::Black).bg(Color::Yellow).bold(),
            ),
            Span::raw(" leave "),
            Span::styled(
                "Ctrl+C",
                Style::default().fg(Color::Black).bg(Color::Red).bold(),
            ),
            Span::raw(" abort"),
        ]));
        frame.render_widget(
            Paragraph::new(text).alignment(Alignment::Left).block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
            area,
        );
    }

    fn render_phases(&self, frame: &mut Frame, area: Rect, wizard: &SetupWizard) {
        let items: Vec<ListItem> = self
            .plan
            .phases
            .iter()
            .map(|phase| {
                let complete = phase.step_ids.iter().all(|step| {
                    matches!(wizard.step_statuses.get(step), Some(StepStatus::Completed))
                });
                let is_active = self.active_phase == Some(phase.id);
                let (symbol, style, badge, badge_color) = if complete {
                    (
                        "✓",
                        Style::default().fg(Color::Green).bold(),
                        "ready",
                        Color::Green,
                    )
                } else if is_active {
                    (
                        "▶",
                        Style::default().fg(Color::Cyan).bold(),
                        "live",
                        Color::Cyan,
                    )
                } else {
                    (
                        "•",
                        Style::default().fg(Color::DarkGray),
                        "queued",
                        Color::DarkGray,
                    )
                };
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(format!("{symbol} "), style),
                        Span::styled(phase.title, style),
                        Span::raw("  "),
                        Span::styled(
                            format!(" {badge} "),
                            Style::default().fg(Color::Black).bg(badge_color).bold(),
                        ),
                    ]),
                    Line::from(Span::styled(
                        phase.description,
                        Style::default().fg(Color::Gray),
                    )),
                ])
            })
            .collect();

        frame.render_widget(
            List::new(items).block(
                Block::default()
                    .title(" Flight Plan ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
            area,
        );
    }

    fn render_focus(&self, frame: &mut Frame, area: Rect, body_lines: &[&str]) {
        let lines: Vec<Line> = body_lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    Line::from(vec![
                        Span::styled("◆ ", Style::default().fg(Color::Cyan).bold()),
                        Span::styled(*line, Style::default().fg(Color::White).bold()),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled("  ", Style::default()),
                        Span::styled(*line, Style::default().fg(Color::Gray)),
                    ])
                }
            })
            .collect();

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(
                Block::default()
                    .title(" Mission Focus ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            ),
            area,
        );
    }

    fn render_summary(&self, frame: &mut Frame, area: Rect, wizard: &SetupWizard) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(7), Constraint::Min(8)])
            .split(area);

        self.render_readiness(frame, chunks[0], wizard.readiness_summary());

        let validations = wizard.validation_items();
        let items: Vec<ListItem> = validations
            .iter()
            .map(|item| {
                let (color, level_label, symbol) = match item.level {
                    ValidationLevel::Info => (Color::Cyan, "info", "i"),
                    ValidationLevel::Warning => (Color::Yellow, "warn", "!"),
                    ValidationLevel::Error => (Color::Red, "error", "x"),
                };
                ListItem::new(vec![
                    Line::from(Span::styled(&item.title, Style::default().fg(color).bold())),
                    Line::from(vec![
                        Span::styled(
                            format!(" {symbol} "),
                            Style::default().fg(Color::Black).bg(color).bold(),
                        ),
                        Span::raw(" "),
                        Span::styled(level_label, Style::default().fg(color).bold()),
                    ]),
                    Line::from(Span::styled(&item.detail, Style::default().fg(Color::Gray))),
                ])
            })
            .collect();

        frame.render_widget(
            List::new(items).block(
                Block::default()
                    .title(" Watchlist ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
            chunks[1],
        );
    }

    fn render_readiness(&self, frame: &mut Frame, area: Rect, readiness: ReadinessSummary) {
        let accent = if readiness.needs_attention > 0 || readiness.followups > 0 {
            Color::Yellow
        } else {
            Color::Green
        };
        let text = vec![
            Line::from(Span::styled(
                readiness.headline,
                Style::default().fg(accent).bold(),
            )),
            Line::from(vec![
                Span::styled(
                    " ready ",
                    Style::default().fg(Color::Black).bg(Color::Green).bold(),
                ),
                Span::raw(" "),
                Span::styled(
                    readiness.ready_now.to_string(),
                    Style::default().fg(Color::Green).bold(),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " attention ",
                    Style::default().fg(Color::Black).bg(Color::Yellow).bold(),
                ),
                Span::raw(" "),
                Span::styled(
                    readiness.needs_attention.to_string(),
                    Style::default().fg(Color::Yellow).bold(),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    " follow-ups ",
                    Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
                ),
                Span::raw(" "),
                Span::styled(
                    readiness.followups.to_string(),
                    Style::default().fg(Color::Cyan).bold(),
                ),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(text).wrap(Wrap { trim: false }).block(
                Block::default()
                    .title(" Launch Readiness ")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
            area,
        );
    }

    fn render_footer(&self, frame: &mut Frame, area: Rect) {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    " Enter ",
                    Style::default().fg(Color::Black).bg(Color::Cyan).bold(),
                ),
                Span::raw(" continue "),
                Span::styled(
                    " Esc ",
                    Style::default().fg(Color::Black).bg(Color::Yellow).bold(),
                ),
                Span::raw(" leave "),
                Span::styled(
                    " Ctrl+C ",
                    Style::default().fg(Color::Black).bg(Color::Red).bold(),
                ),
                Span::raw(" force quit"),
            ]))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
            area,
        );
    }
}
