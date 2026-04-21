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
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use super::{SetupError, SetupWizard, StepStatus, WizardPhaseId, WizardPlan, WizardStepId};
use crate::branding::art::onboarding_brand_block;
use crate::setup::prompts::{TuiPromptMessage, TuiPromptMessageTone, take_tui_prompt_messages};
use crate::terminal_branding::resolve_cli_skin_name;
use crate::tui::skin::CliSkin;

struct CardSpec {
    eyebrow: String,
    title: String,
    subtitle: Option<String>,
    body_lines: Vec<String>,
    activity: Vec<TuiPromptMessage>,
    show_art: bool,
    status: Option<StepStatus>,
    primary_action: &'static str,
}

#[derive(Clone, Copy)]
struct CardLayout {
    card_area: Rect,
    content_area: Rect,
    footer_area: Rect,
    total_content_height: u16,
    max_scroll: u16,
}

pub(super) struct OnboardingTuiShell {
    plan: WizardPlan,
    active_phase: Option<WizardPhaseId>,
    active_step_id: Option<WizardStepId>,
    current_step: Option<(usize, usize)>,
    skin: CliSkin,
}

impl OnboardingTuiShell {
    pub(super) fn new(plan: WizardPlan) -> Self {
        Self {
            plan,
            active_phase: None,
            active_step_id: None,
            current_step: None,
            skin: CliSkin::load(&resolve_cli_skin_name()),
        }
    }

    pub(super) fn show_completion(&mut self, wizard: &SetupWizard) -> Result<(), SetupError> {
        self.refresh_skin(wizard);
        self.active_phase = Some(WizardPhaseId::Finish);
        self.active_step_id = Some(WizardStepId::Summary);
        self.current_step = Some((self.plan.total_steps(), self.plan.total_steps()));

        let continues = wizard.should_continue_to_runtime();

        self.present_view(CardSpec {
            eyebrow: "Launch summary".to_string(),
            title: "Onboarding complete".to_string(),
            subtitle: Some(wizard.runtime_handoff_summary()),
            body_lines: self.completion_body(wizard),
            activity: take_tui_prompt_messages(),
            show_art: true,
            status: Some(if wizard.followups.is_empty() {
                StepStatus::Completed
            } else {
                StepStatus::NeedsAttention
            }),
            primary_action: if continues {
                "Continue to startup"
            } else {
                "Finish setup"
            },
        })
    }

    fn completion_body(&self, wizard: &SetupWizard) -> Vec<String> {
        let mut lines = vec![
            if wizard.should_continue_to_runtime() {
                "Configuration is saved and the bootstrap handoff is ready.".to_string()
            } else {
                "Configuration is saved and ThinClaw is ready for a manual launch later."
                    .to_string()
            },
            format!("Runtime: {}", self.runtime_summary(wizard)),
            format!("AI stack: {}", self.provider_summary(wizard)),
            format!("Channels: {}", self.channel_summary(wizard)),
        ];

        if let Some(timezone) = wizard.settings.user_timezone.as_deref() {
            lines.push(format!("Timezone: {timezone}"));
        }

        lines.push("What next:".to_string());
        lines.extend(wizard.what_next_commands());
        lines.push(if wizard.should_continue_to_runtime() {
            "Press Enter to continue into startup.".to_string()
        } else {
            "Press Enter to finish onboarding.".to_string()
        });

        if wizard.followups.is_empty() {
            lines.push("Follow-ups: none queued.".to_string());
        } else {
            lines.push(format!(
                "Follow-ups: {} item(s) queued for later review.",
                wizard.followups.len()
            ));
        }

        if let Some(item) = wizard
            .validation_items()
            .into_iter()
            .find(|item| !matches!(item.level, super::ValidationLevel::Info))
        {
            lines.push(format!("Needs review: {} — {}", item.title, item.detail));
        }
        lines
    }

    fn refresh_skin(&mut self, wizard: &SetupWizard) {
        let skin_name = if wizard.settings.agent.cli_skin.trim().is_empty() {
            resolve_cli_skin_name()
        } else {
            wizard.settings.agent.cli_skin.clone()
        };
        self.skin = CliSkin::load(&skin_name);
    }

    fn runtime_summary(&self, wizard: &SetupWizard) -> String {
        match wizard.settings.database_backend.as_deref() {
            Some("libsql") => {
                if let Some(path) = wizard.settings.libsql_path.as_deref() {
                    format!("libSQL ({path})")
                } else {
                    "libSQL (default path)".to_string()
                }
            }
            Some(other) => other.to_string(),
            None if wizard.settings.database_url.is_some() => "PostgreSQL".to_string(),
            None => "still needs review".to_string(),
        }
    }

    fn provider_summary(&self, wizard: &SetupWizard) -> String {
        let provider = wizard
            .settings
            .llm_backend
            .as_deref()
            .map(|value| match value {
                "anthropic" => "Anthropic",
                "openai" => "OpenAI",
                "ollama" => "Ollama",
                "openai_compatible" => "OpenAI-compatible",
                other => other,
            })
            .unwrap_or("unconfigured");
        let model = wizard
            .settings
            .selected_model
            .as_deref()
            .unwrap_or("model not selected");
        format!("{provider} · {model}")
    }

    fn channel_summary(&self, wizard: &SetupWizard) -> String {
        let mut channels = vec!["terminal".to_string()];
        let configured = wizard.configured_channel_names();
        if configured.is_empty() {
            return "terminal only".to_string();
        }
        channels.extend(configured);
        channels.join(", ")
    }

    fn present_view(&mut self, spec: CardSpec) -> Result<(), SetupError> {
        enable_raw_mode().map_err(SetupError::Io)?;
        io::stdout()
            .execute(EnterAlternateScreen)
            .map_err(SetupError::Io)?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend).map_err(SetupError::Io)?;
        terminal.hide_cursor().map_err(SetupError::Io)?;

        let result = self.event_loop(&mut terminal, &spec);

        disable_raw_mode().map_err(SetupError::Io)?;
        io::stdout()
            .execute(LeaveAlternateScreen)
            .map_err(SetupError::Io)?;
        io::stdout().execute(cursor::Show).map_err(SetupError::Io)?;
        terminal.show_cursor().map_err(SetupError::Io)?;

        result
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
        spec: &CardSpec,
    ) -> Result<(), SetupError> {
        let mut scroll_offset = 0u16;

        loop {
            let viewport = Rect::from(terminal.size().map_err(SetupError::Io)?);
            let card_width = viewport.width.saturating_sub(8).clamp(60, 100);
            let content = self.build_card_text(spec, card_width.saturating_sub(2));
            let layout = self.card_layout(viewport, card_width, &content);
            scroll_offset = scroll_offset.min(layout.max_scroll);

            terminal
                .draw(|frame| self.render(frame, spec, &content, layout, scroll_offset))
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
                    (_, KeyCode::Up) | (_, KeyCode::Char('k')) => {
                        scroll_offset = scroll_offset.saturating_sub(1);
                    }
                    (_, KeyCode::Down) | (_, KeyCode::Char('j')) => {
                        scroll_offset = scroll_offset.saturating_add(1).min(layout.max_scroll);
                    }
                    (_, KeyCode::PageUp) => {
                        scroll_offset = scroll_offset.saturating_sub(layout.content_area.height);
                    }
                    (_, KeyCode::PageDown) => {
                        scroll_offset = scroll_offset
                            .saturating_add(layout.content_area.height)
                            .min(layout.max_scroll);
                    }
                    (_, KeyCode::Home) => scroll_offset = 0,
                    (_, KeyCode::End) => scroll_offset = layout.max_scroll,
                    _ => {}
                }
            }
        }
    }

    fn render(
        &self,
        frame: &mut Frame,
        spec: &CardSpec,
        content: &Text<'static>,
        layout: CardLayout,
        scroll_offset: u16,
    ) {
        frame.render_widget(Clear, frame.area());
        let border_style = self.border_style(spec);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(
                format!(" {} ", spec.eyebrow),
                self.skin.accent_style(),
            ));
        frame.render_widget(block, layout.card_area);
        frame.render_widget(
            Paragraph::new(content.clone())
                .wrap(Wrap { trim: false })
                .scroll((scroll_offset, 0))
                .alignment(Alignment::Left),
            layout.content_area,
        );
        frame.render_widget(
            Paragraph::new(self.footer_text(
                spec.primary_action,
                scroll_offset,
                layout.max_scroll,
                layout.content_area.height,
                layout.total_content_height,
            ))
            .alignment(Alignment::Left),
            layout.footer_area,
        );
    }

    fn border_style(&self, spec: &CardSpec) -> Style {
        match spec.status {
            Some(StepStatus::Completed) => self.skin.accent_style(),
            Some(StepStatus::NeedsAttention) => self.skin.warn_style(),
            Some(StepStatus::Skipped) => self.skin.border_style(),
            _ => self.skin.border_style(),
        }
    }

    fn build_card_text(&self, spec: &CardSpec, width: u16) -> Text<'static> {
        let mut lines = Vec::new();
        let content_width = width.saturating_sub(2) as usize;

        lines.push(Line::from(Span::styled(
            format!("ThinClaw · {} · onboarding", self.skin.name),
            self.skin.accent_soft_style(),
        )));

        if spec.show_art && ascii_art_fits(&self.skin, content_width) {
            lines.push(Line::from(""));
            if let Some(art) = onboarding_brand_block(&self.skin, content_width) {
                lines.extend(art.to_ratatui_lines(&self.skin));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            spec.title.clone(),
            self.skin.body_style().bold(),
        )));
        if let Some(subtitle) = spec.subtitle.as_deref() {
            lines.push(Line::from(Span::styled(
                subtitle.to_string(),
                self.skin.muted_style(),
            )));
        }

        lines.push(Line::from(""));
        lines.extend(self.summary_lines());
        lines.push(self.progress_bar_line(content_width));
        lines.push(Line::from(""));

        for body_line in &spec.body_lines {
            if body_line.starts_with("Recommended:") {
                lines.push(Line::from(vec![
                    Span::styled("› ", self.skin.accent_style()),
                    Span::styled(body_line.clone(), self.skin.accent_style()),
                ]));
            } else if body_line.starts_with("Press ") {
                lines.push(Line::from(vec![
                    Span::styled("→ ", self.skin.muted_style()),
                    Span::styled(body_line.clone(), self.skin.muted_style()),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("• ", self.skin.border_soft_style()),
                    Span::styled(body_line.clone(), self.skin.body_style()),
                ]));
            }
        }

        let recent_activity: Vec<TuiPromptMessage> = spec
            .activity
            .iter()
            .filter(|message| !message.text.trim().is_empty())
            .cloned()
            .collect();
        if !recent_activity.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Recent activity",
                self.skin.body_style().bold(),
            )));
            for message in recent_activity.iter().rev().take(4).rev() {
                lines.push(self.activity_line(message));
            }
        }

        Text::from(lines)
    }

    fn summary_lines(&self) -> Vec<Line<'static>> {
        vec![
            self.meta_line("Phase", &self.phase_progress_label()),
            self.meta_line("Step", &self.step_progress_label()),
            self.meta_line("Skin", &self.skin.name),
        ]
    }

    fn progress_bar_line(&self, content_width: usize) -> Line<'static> {
        let bar_width = content_width.clamp(18, 30);
        let ratio = self.progress_ratio();
        let filled = ((bar_width as f32) * ratio).round() as usize;
        let empty = bar_width.saturating_sub(filled);
        let percent = (ratio * 100.0).round() as usize;

        Line::from(vec![
            Span::styled("Progress ", self.skin.muted_style()),
            Span::styled("█".repeat(filled), self.skin.accent_style()),
            Span::styled("░".repeat(empty), self.skin.border_soft_style()),
            Span::raw(" "),
            Span::styled(format!("{percent:>3}%"), self.skin.muted_style()),
        ])
    }

    fn activity_line(&self, message: &TuiPromptMessage) -> Line<'static> {
        let (marker, style) = match message.tone {
            TuiPromptMessageTone::Accent => ("› ", self.skin.accent_style()),
            TuiPromptMessageTone::Info => ("• ", self.skin.body_style()),
            TuiPromptMessageTone::Warning => ("! ", self.skin.warn_style()),
            TuiPromptMessageTone::Error => ("× ", self.skin.bad_style()),
        };
        Line::from(vec![
            Span::styled(marker, style),
            Span::styled(message.text.clone(), style),
        ])
    }

    fn footer_text(
        &self,
        primary_action: &str,
        scroll_offset: u16,
        max_scroll: u16,
        visible_height: u16,
        total_content_height: u16,
    ) -> Text<'static> {
        let mut lines = vec![Line::from(vec![
            keycap("Enter", self.skin.title_style()),
            Span::styled(format!(" {primary_action} "), self.skin.body_style()),
            keycap("Esc", self.skin.title_style()),
            Span::styled(" leave ", self.skin.body_style()),
            keycap("Ctrl+C", self.skin.title_style()),
            Span::styled(" abort", self.skin.body_style()),
        ])];

        if max_scroll > 0 {
            let visible_from = scroll_offset.saturating_add(1).min(total_content_height);
            let visible_to = scroll_offset
                .saturating_add(visible_height)
                .min(total_content_height);
            lines.push(Line::from(vec![
                Span::styled("Use ", self.skin.muted_style()),
                keycap("↑/↓", self.skin.title_style()),
                Span::styled(
                    format!(
                        " to scroll · showing {visible_from}-{visible_to} of {total_content_height}"
                    ),
                    self.skin.muted_style(),
                ),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                "Progress saves automatically.",
                self.skin.muted_style(),
            )));
        }

        Text::from(lines)
    }

    fn meta_line(&self, label: &str, value: &str) -> Line<'static> {
        Line::from(vec![
            Span::styled(format!("{label}: "), self.skin.accent_soft_style()),
            Span::raw(" "),
            Span::styled(value.to_string(), self.skin.body_style()),
        ])
    }

    fn phase_progress_label(&self) -> String {
        let total_phases = self.plan.phases.len();
        match self
            .active_phase
            .and_then(|phase_id| self.plan.phase(phase_id))
        {
            Some(phase) => {
                let index = self
                    .plan
                    .phase_index(phase.id)
                    .map(|idx| idx + 1)
                    .unwrap_or(1);
                format!(
                    "{index}/{total_phases} · {} · {} steps",
                    phase.id.title(),
                    phase.step_ids.len()
                )
            }
            None => format!("{total_phases} phases"),
        }
    }

    fn step_progress_label(&self) -> String {
        match (
            self.current_step,
            self.active_phase
                .and_then(|phase_id| self.plan.phase(phase_id)),
            self.active_step_id,
        ) {
            (Some((current, total)), Some(phase), Some(step_id)) => {
                let phase_index = phase
                    .step_ids
                    .iter()
                    .position(|candidate| *candidate == step_id)
                    .map(|idx| idx + 1)
                    .unwrap_or(1);
                format!(
                    "{current}/{total} overall · {phase_index}/{} in phase",
                    phase.step_ids.len()
                )
            }
            _ => "Overview".to_string(),
        }
    }

    fn progress_ratio(&self) -> f32 {
        self.current_step
            .map(|(current, total)| current as f32 / total.max(1) as f32)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
    }

    fn card_layout(&self, area: Rect, card_width: u16, content: &Text<'static>) -> CardLayout {
        let card_width = card_width.max(24).min(area.width);
        let inner_width = card_width.saturating_sub(2).max(1);
        let total_content_height = text_visual_height(content, inner_width as usize);
        let footer_height = 2u16;
        let max_card_height = area.height.saturating_sub(2).max(14);
        let desired_card_height = total_content_height
            .saturating_add(footer_height)
            .saturating_add(2)
            .clamp(14, max_card_height);
        let card_area = centered_rect(area, card_width, desired_card_height);
        let inner_area = Rect::new(
            card_area.x.saturating_add(1),
            card_area.y.saturating_add(1),
            card_area.width.saturating_sub(2),
            card_area.height.saturating_sub(2),
        );
        let footer_height = footer_height
            .min(inner_area.height.saturating_sub(1))
            .max(1);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(footer_height)])
            .split(inner_area);
        let content_area = sections[0];
        let footer_area = sections[1];
        let max_scroll = total_content_height.saturating_sub(content_area.height);

        CardLayout {
            card_area,
            content_area,
            footer_area,
            total_content_height,
            max_scroll,
        }
    }
}

fn keycap(label: &str, style: Style) -> Span<'static> {
    Span::styled(format!(" {label} "), style)
}

fn centered_rect(area: Rect, desired_width: u16, desired_height: u16) -> Rect {
    let width = desired_width.max(24).min(area.width);
    let height = desired_height.max(8).min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width, height)
}

fn ascii_art_fits(skin: &CliSkin, width: usize) -> bool {
    onboarding_brand_block(skin, width).is_some()
}

fn estimate_wrapped_height(text: &str, width: usize) -> u16 {
    if width == 0 {
        return 1;
    }

    UnicodeWidthStr::width(text).max(1).div_ceil(width) as u16
}

fn text_visual_height(text: &Text<'_>, width: usize) -> u16 {
    text.lines
        .iter()
        .map(|line| {
            let plain = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            estimate_wrapped_height(&plain, width)
        })
        .sum::<u16>()
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{OnboardingFollowupCategory, OnboardingFollowupStatus};
    use crate::setup::wizard::FollowupDraft;

    #[test]
    fn test_completion_body_prioritizes_what_next_before_followups() {
        let mut wizard = SetupWizard::new();
        wizard.followups.push(FollowupDraft {
            id: "runtime-check".to_string(),
            title: "Verify runtime".to_string(),
            category: OnboardingFollowupCategory::Runtime,
            status: OnboardingFollowupStatus::Pending,
            instructions: "Double-check runtime handoff.".to_string(),
            action_hint: None,
        });

        let shell = OnboardingTuiShell::new(wizard.build_plan());
        let lines = shell.completion_body(&wizard);
        let what_next_index = lines
            .iter()
            .position(|line| line == "What next:")
            .expect("what next section should be present");
        let followups_index = lines
            .iter()
            .position(|line| line.starts_with("Follow-ups:"))
            .expect("follow-up summary should be present");

        assert!(what_next_index < followups_index);
    }

    #[test]
    fn test_card_layout_reports_scroll_when_content_overflows() {
        let wizard = SetupWizard::new();
        let shell = OnboardingTuiShell::new(wizard.build_plan());
        let spec = CardSpec {
            eyebrow: "Launch summary".to_string(),
            title: "Onboarding complete".to_string(),
            subtitle: Some("ThinClaw will now continue into `thinclaw tui`.".to_string()),
            body_lines: (0..40)
                .map(|index| format!("Runtime note {index}: keep this content visible."))
                .collect(),
            activity: Vec::new(),
            show_art: false,
            status: Some(StepStatus::Completed),
            primary_action: "Continue to startup",
        };

        let content = shell.build_card_text(&spec, 70);
        let layout = shell.card_layout(Rect::new(0, 0, 80, 18), 72, &content);

        assert!(layout.max_scroll > 0);
        assert!(layout.total_content_height > layout.content_area.height);
    }
}
