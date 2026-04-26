//! Interactive prompt utilities for the setup wizard.
//!
//! Provides terminal UI components for:
//! - Single selection menus
//! - Multi-select with toggles
//! - Password/secret input (hidden)
//! - Yes/no confirmations
//! - Styled headers and step indicators

use std::{
    cell::{Cell, RefCell},
    io::{self, Write},
};

use crossterm::{
    ExecutableCommand, cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    style::{Color as CrosstermColor, Print, ResetColor, SetForegroundColor},
    terminal::{self, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};
use secrecy::SecretString;
use unicode_width::UnicodeWidthStr;

use crate::branding::art::{best_wordmark_block, hero_block};
use crate::terminal_branding::TerminalBranding;
use crate::tui::skin::CliSkin;

/// Prompt rendering mode for setup interactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptUiMode {
    Cli,
    Tui,
}

impl PromptUiMode {
    const fn default_mode() -> Self {
        Self::Cli
    }
}

thread_local! {
    static PROMPT_UI_MODE: Cell<PromptUiMode> = const { Cell::new(PromptUiMode::default_mode()) };
    static PROMPT_TUI_MESSAGES: RefCell<Vec<TuiPromptMessage>> = const { RefCell::new(Vec::new()) };
    static PROMPT_TUI_CONTEXT: RefCell<Option<TuiPromptContext>> = const { RefCell::new(None) };
}

const MAX_TUI_MESSAGES: usize = 48;
const MAX_VISIBLE_TUI_MESSAGES: usize = 4;
const TUI_BACK_SIGNAL: &str = "__thinclaw_previous_step__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiPromptMessageTone {
    Accent,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct TuiPromptMessage {
    pub tone: TuiPromptMessageTone,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct TuiPromptContext {
    pub phase_title: Option<String>,
    pub phase_description: Option<String>,
    pub step_progress: Option<String>,
    pub description: Option<String>,
    pub why_this_matters: Option<String>,
    pub recommended: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct PromptOptionView {
    title: String,
    badge: Option<String>,
    detail: Vec<String>,
}

pub fn current_prompt_ui_mode() -> PromptUiMode {
    PROMPT_UI_MODE.with(|mode| mode.get())
}

pub struct PromptUiModeGuard {
    previous: PromptUiMode,
}

impl Drop for PromptUiModeGuard {
    fn drop(&mut self) {
        PROMPT_UI_MODE.with(|mode| mode.set(self.previous));
    }
}

pub fn push_prompt_ui_mode(mode: PromptUiMode) -> PromptUiModeGuard {
    let previous = current_prompt_ui_mode();
    PROMPT_UI_MODE.with(|current| current.set(mode));
    PromptUiModeGuard { previous }
}

fn push_tui_prompt_message(tone: TuiPromptMessageTone, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    PROMPT_TUI_MESSAGES.with(|messages| {
        let mut messages = messages.borrow_mut();
        messages.push(TuiPromptMessage {
            tone,
            text: trimmed.to_string(),
        });
        if messages.len() > MAX_TUI_MESSAGES {
            let overflow = messages.len() - MAX_TUI_MESSAGES;
            messages.drain(0..overflow);
        }
    });
}

pub fn clear_tui_prompt_messages() {
    PROMPT_TUI_MESSAGES.with(|messages| messages.borrow_mut().clear());
}

pub fn set_tui_prompt_context(context: TuiPromptContext) {
    PROMPT_TUI_CONTEXT.with(|slot| {
        *slot.borrow_mut() = Some(context);
    });
}

pub fn clear_tui_prompt_context() {
    PROMPT_TUI_CONTEXT.with(|slot| {
        *slot.borrow_mut() = None;
    });
}

pub fn take_tui_prompt_messages() -> Vec<TuiPromptMessage> {
    PROMPT_TUI_MESSAGES.with(|messages| messages.take())
}

pub fn is_back_navigation(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted && error.to_string() == TUI_BACK_SIGNAL
}

fn back_navigation_error() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, TUI_BACK_SIGNAL)
}

fn recent_tui_prompt_messages(limit: usize) -> Vec<TuiPromptMessage> {
    PROMPT_TUI_MESSAGES.with(|messages| {
        let messages = messages.borrow();
        let start = messages.len().saturating_sub(limit);
        messages[start..].to_vec()
    })
}

fn current_tui_prompt_context() -> Option<TuiPromptContext> {
    PROMPT_TUI_CONTEXT.with(|slot| slot.borrow().clone())
}

fn estimate_wrapped_lines(text: &str, width: usize) -> u16 {
    if width == 0 {
        return 1;
    }

    let text_width = UnicodeWidthStr::width(text).max(1);
    text_width.div_ceil(width) as u16
}

fn estimate_message_height(messages: &[TuiPromptMessage], width: usize) -> u16 {
    messages
        .iter()
        .map(|message| estimate_wrapped_lines(&message.text, width.saturating_sub(3)).max(1))
        .sum()
}

fn render_prompt_messages(messages: &[TuiPromptMessage], skin: &CliSkin) -> Vec<Line<'static>> {
    messages
        .iter()
        .map(|message| {
            let (marker, style) = match message.tone {
                TuiPromptMessageTone::Accent => ("› ", skin.accent_style()),
                TuiPromptMessageTone::Info => ("• ", skin.body_style()),
                TuiPromptMessageTone::Warning => ("! ", skin.warn_style()),
                TuiPromptMessageTone::Error => ("× ", skin.bad_style()),
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(message.text.clone(), style),
            ])
        })
        .collect()
}

fn prompt_option_view(raw: &str) -> PromptOptionView {
    let mut lines: Vec<String> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if lines.is_empty() {
        return PromptOptionView::default();
    }

    let mut badge = None;
    for line in &mut lines {
        if line.contains("[current]") {
            *line = line.replace("[current]", "").trim().to_string();
            badge = Some("current".to_string());
        }
    }

    let first = lines.remove(0);
    let (title, inline_detail) = split_prompt_option_line(&first);
    let mut detail = Vec::new();
    if !inline_detail.is_empty() {
        detail.push(inline_detail);
    }
    if !lines.is_empty() {
        detail.push(lines.join(" "));
    }

    PromptOptionView {
        title: if title.is_empty() { first } else { title },
        badge,
        detail,
    }
}

fn split_prompt_option_line(line: &str) -> (String, String) {
    for separator in [" — ", " – ", " - "] {
        if let Some((title, detail)) = line.split_once(separator) {
            return (title.trim().to_string(), detail.trim().to_string());
        }
    }
    (line.trim().to_string(), String::new())
}

fn render_prompt_detail_block(
    skin: &CliSkin,
    context: Option<&TuiPromptContext>,
    option: Option<&PromptOptionView>,
    recent_messages: &[TuiPromptMessage],
) -> Text<'static> {
    let mut lines = Vec::new();

    if let Some(context) = context {
        if let Some(progress) = context.step_progress.as_deref() {
            let mut meta = progress.to_string();
            if let Some(phase) = context.phase_title.as_deref() {
                meta.push_str(" · ");
                meta.push_str(phase);
            }
            lines.push(Line::from(Span::styled(meta, skin.accent_soft_style())));
        } else if let Some(phase) = context.phase_title.as_deref() {
            lines.push(Line::from(Span::styled(
                phase.to_string(),
                skin.accent_soft_style(),
            )));
        }

        if let Some(phase_description) = context.phase_description.as_deref() {
            lines.push(Line::from(Span::styled(
                phase_description.to_string(),
                skin.muted_style(),
            )));
        }

        if let Some(description) = context.description.as_deref() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "About this step",
                skin.body_style().bold(),
            )));
            lines.push(Line::from(Span::styled(
                description.to_string(),
                skin.body_style(),
            )));
        }

        if let Some(why) = context.why_this_matters.as_deref() {
            lines.push(Line::from(Span::styled(
                format!("Why: {why}"),
                skin.muted_style(),
            )));
        }

        if let Some(recommended) = context.recommended.as_deref() {
            lines.push(Line::from(Span::styled(
                format!("Recommended: {recommended}"),
                skin.accent_style(),
            )));
        }
    }

    if let Some(option) = option {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            "Active setting",
            skin.body_style().bold(),
        )));
        let mut option_spans = vec![Span::styled(option.title.clone(), skin.title_style())];
        if let Some(badge) = option.badge.as_deref() {
            option_spans.push(Span::styled(format!("  {badge}"), skin.accent_soft_style()));
        }
        lines.push(Line::from(option_spans));
        if option.detail.is_empty() {
            lines.push(Line::from(Span::styled(
                "This choice does not add extra notes beyond the label.",
                skin.muted_style(),
            )));
        } else {
            for detail_line in &option.detail {
                lines.push(Line::from(Span::styled(
                    detail_line.clone(),
                    skin.body_style(),
                )));
            }
        }
    }

    let live_messages: Vec<_> = recent_messages
        .iter()
        .filter(|message| !message.text.trim().is_empty())
        .cloned()
        .collect();
    if !live_messages.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            "Latest feedback",
            skin.body_style().bold(),
        )));
        lines.extend(render_prompt_messages(&live_messages, skin));
    }

    Text::from(lines)
}

fn estimate_prompt_detail_height(
    context: Option<&TuiPromptContext>,
    option: Option<&PromptOptionView>,
    recent_messages: &[TuiPromptMessage],
    width: usize,
) -> u16 {
    let mut height = 0u16;

    if let Some(context) = context {
        if let Some(progress) = context.step_progress.as_deref() {
            height = height.saturating_add(estimate_wrapped_lines(progress, width));
        } else if let Some(phase) = context.phase_title.as_deref() {
            height = height.saturating_add(estimate_wrapped_lines(phase, width));
        }
        if let Some(phase_description) = context.phase_description.as_deref() {
            height = height.saturating_add(estimate_wrapped_lines(phase_description, width));
        }

        if let Some(description) = context.description.as_deref() {
            height = height.saturating_add(2);
            height = height.saturating_add(estimate_wrapped_lines(description, width));
        }
        if let Some(why) = context.why_this_matters.as_deref() {
            height = height.saturating_add(estimate_wrapped_lines(&format!("Why: {why}"), width));
        }
        if let Some(recommended) = context.recommended.as_deref() {
            height = height.saturating_add(estimate_wrapped_lines(
                &format!("Recommended: {recommended}"),
                width,
            ));
        }
    }

    if let Some(option) = option {
        height = height.saturating_add(2);
        height = height.saturating_add(estimate_wrapped_lines(&option.title, width));
        for line in &option.detail {
            height = height.saturating_add(estimate_wrapped_lines(line, width));
        }
        if option.detail.is_empty() {
            height = height.saturating_add(1);
        }
    }

    if !recent_messages.is_empty() {
        height = height.saturating_add(2);
        height = height.saturating_add(estimate_message_height(recent_messages, width));
    }

    height.max(1)
}

fn detail_panel_width(available_width: u16) -> Option<u16> {
    (available_width >= 58).then_some(available_width.saturating_mul(48) / 100)
}

fn prompt_logo_lines(skin: &CliSkin, width: usize) -> Option<Vec<Line<'static>>> {
    if width == 0 {
        return None;
    }

    best_wordmark_block(width).map(|art| art.to_ratatui_lines(skin))
}

fn draw_prompt_logo(frame: &mut Frame, skin: &CliSkin) -> Rect {
    let logo_lines = prompt_logo_lines(skin, frame.area().width.saturating_sub(4) as usize);
    let logo_height = logo_lines
        .as_ref()
        .map(|lines| lines.len() as u16)
        .unwrap_or(0);

    if logo_height == 0 || frame.area().height <= logo_height.saturating_add(5) {
        return frame.area();
    }

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(logo_height.saturating_add(1)),
            Constraint::Min(1),
        ])
        .split(frame.area());

    if let Some(lines) = logo_lines {
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .alignment(Alignment::Center),
            split[0],
        );
    }

    split[1]
}

fn prompt_hero_panel_width(skin: &CliSkin, available_width: u16) -> Option<u16> {
    let hero = hero_block(skin)?;
    let panel_width = (hero.width() as u16).saturating_add(4).clamp(18, 24);
    (available_width >= panel_width.saturating_add(26)).then_some(panel_width)
}

fn render_prompt_hero(frame: &mut Frame, area: Rect, skin: &CliSkin) {
    let Some(hero) = hero_block(skin) else {
        return;
    };

    let mut lines = Vec::new();
    let inner_height = area.height.saturating_sub(2) as usize;
    let top_padding = inner_height.saturating_sub(hero.height()) / 2;
    for _ in 0..top_padding {
        lines.push(Line::from(""));
    }
    lines.extend(hero.to_ratatui_lines(skin));

    let widget = Paragraph::new(Text::from(lines))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(skin.border_style())
                .title(Span::styled(" sigil ", skin.accent_style())),
        );
    frame.render_widget(widget, area);
}

fn with_tui_terminal<F, T>(mut body: F) -> io::Result<T>
where
    F: FnMut(&mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<T>,
{
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    stdout.execute(cursor::Hide)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = body(&mut terminal);

    let _ = terminal::disable_raw_mode();
    let _ = io::stdout().execute(LeaveAlternateScreen);
    let _ = io::stdout().execute(cursor::Show);
    let _ = terminal.show_cursor();

    result
}

fn draw_prompt_list(
    frame: &mut Frame,
    title: &str,
    subtitle: Option<&str>,
    help: &str,
    options: &[String],
    cursor_idx: usize,
    selected: Option<&[bool]>,
) {
    let branding = TerminalBranding::current();
    let skin = &branding.skin;
    frame.render_widget(Clear, frame.area());
    let content_area = draw_prompt_logo(frame, skin);
    let context = current_tui_prompt_context();
    let recent_messages = recent_tui_prompt_messages(MAX_VISIBLE_TUI_MESSAGES);
    let option_views: Vec<PromptOptionView> = options
        .iter()
        .map(|option| prompt_option_view(option))
        .collect();
    let desired_width = content_area.width.saturating_sub(6).min(126);
    let body_height = (options.len() as u16).clamp(7, 13).max(10);
    let card_area = centered_prompt_rect(
        content_area,
        desired_width,
        body_height
            .saturating_add(7)
            .min(content_area.height.saturating_sub(2).max(12)),
    );
    let header_height = if subtitle.is_some() { 4 } else { 3 };
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(skin.border_style()),
        card_area,
    );
    let constraints = vec![
        Constraint::Length(header_height),
        Constraint::Min(8),
        Constraint::Length(2),
    ];
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .margin(1)
        .split(card_area);

    let mut header_lines = vec![
        Line::from(Span::styled(
            format!("ThinClaw · {} · onboarding", skin.name),
            skin.accent_soft_style(),
        )),
        Line::from(Span::styled(title, skin.body_style().bold())),
    ];
    if let Some(sub) = subtitle {
        vec![Line::from(Span::styled(sub, skin.muted_style()))]
            .into_iter()
            .for_each(|line| header_lines.push(line));
    }

    frame.render_widget(
        Paragraph::new(header_lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(skin.border_soft_style()),
            ),
        inner[0],
    );

    let content_row = inner[1];
    let (hero_area, main_area) =
        if let Some(hero_width) = prompt_hero_panel_width(skin, content_row.width) {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(hero_width), Constraint::Min(20)])
                .split(content_row);
            (Some(split[0]), split[1])
        } else {
            (None, content_row)
        };
    if let Some(hero_area) = hero_area {
        render_prompt_hero(frame, hero_area, skin);
    }

    let (options_area, detail_area) =
        if let Some(detail_width) = detail_panel_width(main_area.width) {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Min(22),
                    Constraint::Length(detail_width.clamp(28, main_area.width.saturating_sub(22))),
                ])
                .split(main_area);
            (split[0], Some(split[1]))
        } else {
            let split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(9)])
                .split(main_area);
            (split[0], Some(split[1]))
        };

    let options_inner_height = options_area.height.saturating_sub(2).max(1) as usize;
    let visible_rows = options_inner_height.max(1);
    let window_start = prompt_window_start(cursor_idx, options.len(), visible_rows);
    let window_end = (window_start + visible_rows).min(options.len());
    let items: Vec<ListItem> = options[window_start..window_end]
        .iter()
        .enumerate()
        .map(|(offset, _)| {
            let idx = window_start + offset;
            let is_cursor = idx == cursor_idx;
            let is_selected = selected.is_some_and(|s| s.get(idx).copied().unwrap_or(false));
            let option = &option_views[idx];
            let marker = if selected.is_some() {
                if is_selected { "[x]" } else { "[ ]" }
            } else if is_cursor {
                "›"
            } else {
                "·"
            };
            let marker_style = if is_cursor {
                skin.accent_style()
            } else if is_selected {
                skin.accent_soft_style()
            } else {
                skin.border_soft_style()
            };
            let text_style = if is_cursor {
                skin.body_style().bold()
            } else {
                skin.body_style()
            };
            let mut spans = vec![
                Span::styled(format!("{marker} "), marker_style),
                Span::styled(option.title.clone(), text_style),
            ];
            if let Some(badge) = option.badge.as_deref() {
                spans.push(Span::styled(format!("  {badge}"), skin.accent_soft_style()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    frame.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(skin.border_style())
                .title(Span::styled(" choices ", skin.muted_style())),
        ),
        options_area,
    );

    if let Some(detail_area) = detail_area {
        let active_option = option_views.get(cursor_idx);
        frame.render_widget(
            Paragraph::new(render_prompt_detail_block(
                skin,
                context.as_ref(),
                active_option,
                &recent_messages,
            ))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(skin.border_style())
                    .title(Span::styled(" details ", skin.accent_style())),
            ),
            detail_area,
        );
    }

    let help_text = if options.len() > visible_rows {
        format!("{help}  {}/{}", cursor_idx + 1, options.len())
    } else {
        help.to_string()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(help_text, skin.muted_style()))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(skin.border_soft_style()),
            ),
        inner[2],
    );
}

fn centered_prompt_rect(area: Rect, desired_width: u16, desired_height: u16) -> Rect {
    let width = desired_width.max(24).min(area.width);
    let height = desired_height.max(8).min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let vertical_slack = area.height.saturating_sub(height);
    let y = area.y + vertical_slack.min(2);
    Rect::new(x, y, width, height)
}

fn prompt_window_start(cursor_idx: usize, len: usize, visible_rows: usize) -> usize {
    if len <= visible_rows || visible_rows == 0 {
        return 0;
    }

    let half = visible_rows / 2;
    let mut start = cursor_idx.saturating_sub(half);
    let max_start = len.saturating_sub(visible_rows);
    if start > max_start {
        start = max_start;
    }
    start
}

fn header_height(hint: Option<&str>) -> u16 {
    if hint.is_some() { 4 } else { 3 }
}

fn select_one_tui(prompt: &str, options: &[&str]) -> io::Result<usize> {
    if options.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "select_one requires at least one option",
        ));
    }

    let options_owned: Vec<String> = options.iter().map(|s| (*s).to_string()).collect();
    with_tui_terminal(|terminal| {
        let mut cursor_idx = 0usize;
        loop {
            terminal.draw(|frame| {
                draw_prompt_list(
                    frame,
                    prompt,
                    None,
                    "Arrow keys move. Enter selects. Ctrl+B goes back. Esc exits setup.",
                    &options_owned,
                    cursor_idx,
                    None,
                )
            })?;

            if event::poll(std::time::Duration::from_millis(250))?
                && let Event::Key(KeyEvent {
                    code, modifiers, ..
                }) = event::read()?
            {
                match code {
                    KeyCode::Up => cursor_idx = cursor_idx.saturating_sub(1),
                    KeyCode::Down if cursor_idx + 1 < options_owned.len() => cursor_idx += 1,
                    KeyCode::Char('k') if modifiers.is_empty() => {
                        cursor_idx = cursor_idx.saturating_sub(1)
                    }
                    KeyCode::Char('j') if modifiers.is_empty() => {
                        if cursor_idx + 1 < options_owned.len() {
                            cursor_idx += 1;
                        }
                    }
                    KeyCode::Enter => return Ok(cursor_idx),
                    KeyCode::Esc => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Esc"));
                    }
                    KeyCode::BackTab => return Err(back_navigation_error()),
                    KeyCode::Char('b') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(back_navigation_error());
                    }
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                    }
                    KeyCode::Char(ch) if ch.is_ascii_digit() => {
                        if let Some(index) = ch.to_digit(10).map(|d| d as usize)
                            && index > 0
                            && index <= options_owned.len()
                        {
                            return Ok(index - 1);
                        }
                    }
                    _ => {}
                }
            }
        }
    })
}

fn select_many_tui(prompt: &str, options: &[(&str, bool)]) -> io::Result<Vec<usize>> {
    if options.is_empty() {
        return Ok(vec![]);
    }

    let options_owned: Vec<String> = options.iter().map(|(s, _)| (*s).to_string()).collect();
    with_tui_terminal(|terminal| {
        let mut cursor_idx = 0usize;
        let mut selected: Vec<bool> = options.iter().map(|(_, s)| *s).collect();

        loop {
            terminal.draw(|frame| {
                draw_prompt_list(
                    frame,
                    prompt,
                    None,
                    "Arrow keys move. Space toggles. Enter confirms. Ctrl+B goes back.",
                    &options_owned,
                    cursor_idx,
                    Some(&selected),
                )
            })?;

            if event::poll(std::time::Duration::from_millis(250))?
                && let Event::Key(KeyEvent {
                    code, modifiers, ..
                }) = event::read()?
            {
                match code {
                    KeyCode::Up => cursor_idx = cursor_idx.saturating_sub(1),
                    KeyCode::Down if cursor_idx + 1 < options_owned.len() => cursor_idx += 1,
                    KeyCode::Char('k') if modifiers.is_empty() => {
                        cursor_idx = cursor_idx.saturating_sub(1)
                    }
                    KeyCode::Char('j') if modifiers.is_empty() => {
                        if cursor_idx + 1 < options_owned.len() {
                            cursor_idx += 1;
                        }
                    }
                    KeyCode::Char(' ') => {
                        if let Some(slot) = selected.get_mut(cursor_idx) {
                            *slot = !*slot;
                        }
                    }
                    KeyCode::Enter => {
                        let indexes = selected
                            .iter()
                            .enumerate()
                            .filter_map(|(idx, enabled)| if *enabled { Some(idx) } else { None })
                            .collect();
                        return Ok(indexes);
                    }
                    KeyCode::Esc => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Esc"));
                    }
                    KeyCode::BackTab => return Err(back_navigation_error()),
                    KeyCode::Char('b') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(back_navigation_error());
                    }
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                    }
                    _ => {}
                }
            }
        }
    })
}

fn input_tui(prompt: &str, hint: Option<&str>, secret: bool) -> io::Result<String> {
    with_tui_terminal(|terminal| {
        let mut value = String::new();
        let mut cursor_pos = 0usize;
        loop {
            terminal.draw(|frame| {
                let branding = TerminalBranding::current();
                let skin = &branding.skin;
                frame.render_widget(Clear, frame.area());
                let content_area = draw_prompt_logo(frame, skin);
                let context = current_tui_prompt_context();
                let context_messages = recent_tui_prompt_messages(MAX_VISIBLE_TUI_MESSAGES);
                let desired_width = content_area.width.saturating_sub(6).min(124);
                let content_width = desired_width.saturating_sub(8) as usize;
                let hero_body_height = if prompt_hero_panel_width(skin, content_width as u16).is_some() {
                    hero_block(skin)
                        .map(|hero| hero.height() as u16 + 2)
                        .unwrap_or(5)
                        .max(5)
                } else {
                    3
                };
                let guide_height = if context.is_some() || !context_messages.is_empty() {
                    estimate_prompt_detail_height(
                        context.as_ref(),
                        None,
                        &context_messages,
                        content_width.saturating_sub(2),
                    )
                    .clamp(5, 9)
                } else {
                    0
                };
                let body_height = hero_body_height.max(guide_height.saturating_add(3));
                let card_area = centered_prompt_rect(
                    content_area,
                    desired_width,
                    header_height(hint)
                        .saturating_add(body_height)
                        .saturating_add(4),
                );
                frame.render_widget(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(skin.border_style()),
                    card_area,
                );

                let constraints = vec![
                    Constraint::Length(header_height(hint)),
                    Constraint::Length(body_height),
                    Constraint::Length(2),
                ];
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints(constraints)
                    .margin(1)
                    .split(card_area);

                let mut header = vec![
                    Line::from(Span::styled(
                        format!("ThinClaw · {} · onboarding", skin.name),
                        skin.accent_soft_style(),
                    )),
                    Line::from(Span::styled(prompt, skin.body_style().bold())),
                ];
                if let Some(h) = hint {
                    header.push(Line::from(Span::styled(h, skin.muted_style())));
                }
                frame.render_widget(
                    Paragraph::new(header)
                        .wrap(Wrap { trim: false })
                        .block(
                            Block::default()
                                .borders(Borders::BOTTOM)
                                .border_style(skin.border_soft_style()),
                        ),
                    layout[0],
                );

                let body_area = layout[1];
                let (hero_area, main_area) =
                    if let Some(hero_width) = prompt_hero_panel_width(skin, body_area.width) {
                        let split = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([Constraint::Length(hero_width), Constraint::Min(20)])
                            .split(body_area);
                        (Some(split[0]), split[1])
                    } else {
                        (None, body_area)
                    };
                if let Some(hero_area) = hero_area {
                    render_prompt_hero(frame, hero_area, skin);
                }

                let main_layout = if guide_height > 0 {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(guide_height), Constraint::Min(3)])
                        .split(main_area)
                } else {
                    Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Min(3)])
                        .split(main_area)
                };

                let editor_area = if guide_height > 0 {
                    frame.render_widget(
                        Paragraph::new(render_prompt_detail_block(
                            skin,
                            context.as_ref(),
                            None,
                            &context_messages,
                        ))
                        .wrap(Wrap { trim: false })
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_style(skin.border_style())
                                .title(Span::styled(" guide ", skin.accent_style())),
                        ),
                        main_layout[0],
                    );
                    main_layout[1]
                } else {
                    main_layout[0]
                };

                let rendered = if secret {
                    "*".repeat(value.chars().count())
                } else {
                    value.clone()
                };
                let prompt_prefix = format!("{} ", skin.prompt_symbol());
                let answer_block = Block::default()
                    .borders(Borders::ALL)
                    .border_style(skin.accent_style());
                let answer_inner = answer_block.inner(editor_area);
                let max_value_width = answer_inner
                    .width
                    .saturating_sub(prompt_prefix.len() as u16 + 1);
                let (visible, cursor_offset) =
                    visible_tail(&rendered, cursor_pos, max_value_width as usize);
                frame.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled(prompt_prefix.clone(), skin.accent_style()),
                        Span::styled(visible, skin.body_style()),
                    ]))
                    .block(answer_block),
                    editor_area,
                );

                frame.render_widget(
                    Paragraph::new(
                        "Type to edit. Enter confirms. Ctrl+B goes back. Esc exits. Left/Right move the cursor.",
                    )
                    .style(skin.muted_style())
                    .block(
                        Block::default()
                            .borders(Borders::TOP)
                            .border_style(skin.border_soft_style()),
                    ),
                    layout[2],
                );

                frame.set_cursor_position((
                    answer_inner.x + prompt_prefix.len() as u16 + cursor_offset,
                    answer_inner.y,
                ));
            })?;

            if event::poll(std::time::Duration::from_millis(250))?
                && let Event::Key(KeyEvent {
                    code, modifiers, ..
                }) = event::read()?
            {
                match code {
                    KeyCode::Enter => return Ok(value),
                    KeyCode::Backspace => {
                        if cursor_pos > 0 {
                            cursor_pos -= 1;
                            if let Some((byte_idx, _)) = value.char_indices().nth(cursor_pos) {
                                value.remove(byte_idx);
                            }
                        }
                    }
                    KeyCode::Delete => {
                        if cursor_pos < value.chars().count()
                            && let Some((byte_idx, _)) = value.char_indices().nth(cursor_pos)
                        {
                            value.remove(byte_idx);
                        }
                    }
                    KeyCode::Left => cursor_pos = cursor_pos.saturating_sub(1),
                    KeyCode::Right => {
                        if cursor_pos < value.chars().count() {
                            cursor_pos += 1;
                        }
                    }
                    KeyCode::Home => cursor_pos = 0,
                    KeyCode::End => cursor_pos = value.chars().count(),
                    KeyCode::BackTab => return Err(back_navigation_error()),
                    KeyCode::Char('b') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(back_navigation_error());
                    }
                    KeyCode::Esc => return Err(io::Error::new(io::ErrorKind::Interrupted, "Esc")),
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                    }
                    KeyCode::Char(ch) => {
                        let byte_pos = value
                            .char_indices()
                            .nth(cursor_pos)
                            .map(|(idx, _)| idx)
                            .unwrap_or(value.len());
                        value.insert(byte_pos, ch);
                        cursor_pos += 1;
                    }
                    _ => {}
                }
            }
        }
    })
}

fn visible_tail(value: &str, cursor_pos: usize, max_width: usize) -> (String, u16) {
    if max_width == 0 {
        return (String::new(), 0);
    }

    let chars: Vec<char> = value.chars().collect();
    let mut start = 0usize;
    if cursor_pos > max_width {
        start = cursor_pos.saturating_sub(max_width);
    }
    let mut visible: String = chars.iter().skip(start).take(max_width).collect();
    if start > 0 && !visible.is_empty() {
        visible.remove(0);
        visible.insert(0, '<');
    }
    let cursor = cursor_pos.saturating_sub(start).min(max_width);
    (visible, cursor as u16)
}

/// Display a numbered menu and get user selection.
///
/// Returns the index (0-based) of the selected option.
/// Pressing Enter without input selects the first option (index 0).
///
/// # Example
///
/// ```ignore
/// let choice = select_one("Choose an option:", &["Option A", "Option B"]);
/// ```
pub fn select_one(prompt: &str, options: &[&str]) -> io::Result<usize> {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        return select_one_tui(prompt, options);
    }

    let mut stdout = io::stdout();

    // Print prompt
    writeln!(stdout, "{}", prompt)?;
    writeln!(stdout)?;

    // Print options
    for (i, option) in options.iter().enumerate() {
        writeln!(stdout, "  [{}] {}", i + 1, option)?;
    }
    writeln!(stdout)?;

    loop {
        print!("> ");
        stdout.flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        // Handle empty input as first option
        if input.is_empty() {
            return Ok(0);
        }

        // Parse number
        if let Ok(num) = input.parse::<usize>()
            && num >= 1
            && num <= options.len()
        {
            return Ok(num - 1);
        }

        writeln!(
            stdout,
            "Invalid choice. Please enter a number 1-{}.",
            options.len()
        )?;
    }
}

/// Multi-select with space to toggle, enter to confirm.
///
/// `options` is a slice of (label, initially_selected) tuples.
/// Returns indices of selected options.
///
/// # Example
///
/// ```ignore
/// let selected = select_many("Select channels:", &[
///     ("CLI/TUI", true),
///     ("HTTP webhook", false),
///     ("Telegram", false),
/// ])?;
/// ```
pub fn select_many(prompt: &str, options: &[(&str, bool)]) -> io::Result<Vec<usize>> {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        return select_many_tui(prompt, options);
    }

    if options.is_empty() {
        return Ok(vec![]);
    }

    let mut stdout = io::stdout();
    let mut selected: Vec<bool> = options.iter().map(|(_, s)| *s).collect();
    let mut cursor_pos = 0;

    terminal::enable_raw_mode()?;
    execute!(stdout, cursor::Hide)?;

    let result = (|| {
        loop {
            // Clear and redraw
            execute!(stdout, cursor::MoveToColumn(0))?;

            writeln!(stdout, "{}\r", prompt)?;
            writeln!(stdout, "\r")?;
            writeln!(
                stdout,
                "  (Use arrow keys to navigate, space to toggle, enter to confirm)\r"
            )?;
            writeln!(stdout, "\r")?;

            for (i, (label, _)) in options.iter().enumerate() {
                let checkbox = if selected[i] { "[x]" } else { "[ ]" };
                let prefix = if i == cursor_pos { ">" } else { " " };

                if i == cursor_pos {
                    execute!(stdout, SetForegroundColor(CrosstermColor::Cyan))?;
                    writeln!(stdout, "  {} {} {}\r", prefix, checkbox, label)?;
                    execute!(stdout, ResetColor)?;
                } else {
                    writeln!(stdout, "  {} {} {}\r", prefix, checkbox, label)?;
                }
            }

            stdout.flush()?;

            // Read key
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Up => {
                        cursor_pos = cursor_pos.saturating_sub(1);
                    }
                    KeyCode::Down if cursor_pos < options.len() - 1 => {
                        cursor_pos += 1;
                    }
                    KeyCode::Char(' ') => {
                        selected[cursor_pos] = !selected[cursor_pos];
                    }
                    KeyCode::Enter => {
                        break;
                    }
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                    }
                    _ => {}
                }

                // Move cursor up to redraw
                execute!(
                    stdout,
                    cursor::MoveUp((options.len() + 4) as u16),
                    terminal::Clear(ClearType::FromCursorDown)
                )?;
            }
        }
        Ok(())
    })();

    // Cleanup
    execute!(stdout, cursor::Show)?;
    terminal::disable_raw_mode()?;
    writeln!(stdout)?;

    result?;

    Ok(selected
        .iter()
        .enumerate()
        .filter_map(|(i, &s)| if s { Some(i) } else { None })
        .collect())
}

/// Password/secret input with hidden characters.
///
/// # Example
///
/// ```ignore
/// let token = secret_input("Bot token")?;
/// ```
pub fn secret_input(prompt: &str) -> io::Result<SecretString> {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        return input_tui(prompt, None, true).map(SecretString::from);
    }

    let mut stdout = io::stdout();

    print!("{}: ", prompt);
    stdout.flush()?;

    terminal::enable_raw_mode()?;
    let result = read_secret_line();
    terminal::disable_raw_mode()?;

    writeln!(stdout)?;
    result
}

fn read_secret_line() -> io::Result<SecretString> {
    let mut input = String::new();
    let mut stdout = io::stdout();

    loop {
        if let Event::Key(KeyEvent {
            code, modifiers, ..
        }) = event::read()?
        {
            match code {
                KeyCode::Enter => {
                    break;
                }
                KeyCode::Backspace if !input.is_empty() => {
                    input.pop();
                    execute!(stdout, Print("\x08 \x08"))?;
                    stdout.flush()?;
                }
                KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    execute!(stdout, Print('*'))?;
                    stdout.flush()?;
                }
                _ => {}
            }
        }
    }

    Ok(SecretString::from(input))
}

/// Yes/no confirmation prompt.
///
/// # Example
///
/// ```ignore
/// if confirm("Enable Telegram channel?", false)? {
///     // ...
/// }
/// ```
pub fn confirm(prompt: &str, default: bool) -> io::Result<bool> {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        let options: [&str; 2] = if default {
            ["Yes (default)", "No"]
        } else {
            ["No (default)", "Yes"]
        };
        let choice = select_one_tui(prompt, &options)?;
        return Ok(if default { choice == 0 } else { choice == 1 });
    }

    let mut stdout = io::stdout();

    let hint = if default { "[Y/n]" } else { "[y/N]" };
    print!("{} {} ", prompt, hint);
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    Ok(match input.as_str() {
        "" => default,
        "y" | "yes" => true,
        "n" | "no" => false,
        _ => default,
    })
}

/// Print a styled header box.
///
/// # Example
///
/// ```ignore
/// print_header("ThinClaw Setup Wizard");
/// ```
pub fn print_header(text: &str) {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        push_tui_prompt_message(TuiPromptMessageTone::Accent, text);
        return;
    }
    TerminalBranding::current().print_banner(text, None);
}

/// Print a step indicator.
///
/// # Example
///
/// ```ignore
/// print_step(1, 3, "NEAR AI Authentication");
/// // Output: Step 1/3: NEAR AI Authentication
/// //         ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
/// ```
pub fn print_step(current: usize, total: usize, name: &str) {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        push_tui_prompt_message(
            TuiPromptMessageTone::Accent,
            &format!("Step {current}/{total} · {name}"),
        );
        return;
    }
    let branding = TerminalBranding::current();
    let progress_width = 24;
    let filled = if total == 0 {
        0
    } else {
        current.saturating_mul(progress_width) / total
    };
    let empty = progress_width.saturating_sub(filled);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

    println!(
        "{}",
        branding.accent(format!("Flight step {current}/{total}: {name}"))
    );
    println!(
        "{}  {}",
        branding.muted(bar),
        branding.body_bold(format!(
            "{:>3}%",
            current.saturating_mul(100) / total.max(1)
        ))
    );
    println!();
}

/// Print a success message with green checkmark.
pub fn print_success(message: &str) {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        push_tui_prompt_message(TuiPromptMessageTone::Accent, message);
        return;
    }
    let branding = TerminalBranding::current();
    println!("{} {}", branding.good("✓"), branding.body(message));
}

/// Print an error message with red X.
pub fn print_error(message: &str) {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        push_tui_prompt_message(TuiPromptMessageTone::Error, message);
        return;
    }
    let branding = TerminalBranding::current();
    eprintln!("{} {}", branding.bad("✗"), branding.body(message));
}

/// Print an info message with blue info icon.
pub fn print_info(message: &str) {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        push_tui_prompt_message(TuiPromptMessageTone::Info, message);
        return;
    }
    let branding = TerminalBranding::current();
    println!("{} {}", branding.accent("ℹ"), branding.body(message));
}

/// Print a warning message with a yellow marker.
pub fn print_warning(message: &str) {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        push_tui_prompt_message(TuiPromptMessageTone::Warning, message);
        return;
    }
    let branding = TerminalBranding::current();
    println!("{} {}", branding.warn("!"), branding.body(message));
}

/// Print a blank line in CLI mode and suppress it in TUI mode.
pub fn print_blank_line() {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        return;
    }
    println!();
}

/// Print a phase banner with a short description.
pub fn print_phase_banner(title: &str, description: &str) {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        let _ = description;
        push_tui_prompt_message(TuiPromptMessageTone::Accent, title);
        return;
    }
    let branding = TerminalBranding::current();
    let width = title.len().max(description.len()).max(24) + 4;
    let border = "═".repeat(width);

    println!();
    println!("{}", branding.accent(format!("╔{}╗", border)));
    println!(
        "{}",
        branding.accent(format!("║  {:width$}  ║", title, width = width))
    );
    println!("{}", branding.accent(format!("╚{}╝", border)));
    println!("  {}", branding.body(description));
    println!(
        "  {}",
        branding.muted("Stay with the recommended route if you want the safest fast path.")
    );
    println!();
}

/// Read a simple line of input with a prompt.
pub fn input(prompt: &str) -> io::Result<String> {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        return input_tui(prompt, None, false);
    }

    let mut stdout = io::stdout();
    print!("{}: ", prompt);
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

/// Read an optional line of input (empty returns None).
pub fn optional_input(prompt: &str, hint: Option<&str>) -> io::Result<Option<String>> {
    if current_prompt_ui_mode() == PromptUiMode::Tui {
        let value = input_tui(prompt, hint, false)?;
        if value.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some(value.trim().to_string()));
    }

    let mut stdout = io::stdout();

    if let Some(h) = hint {
        print!("{} ({}): ", prompt, h);
    } else {
        print!("{}: ", prompt);
    }
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.is_empty() {
        Ok(None)
    } else {
        Ok(Some(input.to_string()))
    }
}

#[cfg(test)]
mod tests {
    // Interactive tests are difficult to unit test, but we can test the non-interactive parts.

    #[test]
    fn test_header_length_calculation() {
        // Just verify it doesn't panic with various inputs
        super::print_header("Test");
        super::print_header("A longer header text");
        super::print_header("");
    }

    #[test]
    fn test_step_indicator() {
        super::print_step(1, 3, "Test Step");
        super::print_step(3, 3, "Final Step");
    }

    #[test]
    fn test_print_functions_do_not_panic() {
        super::print_success("operation completed");
        super::print_error("something went wrong");
        super::print_info("here is some information");
        // Also test with empty strings
        super::print_success("");
        super::print_error("");
        super::print_info("");
    }
}
