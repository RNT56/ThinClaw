//! Interactive prompt utilities for the setup wizard.
//!
//! Provides terminal UI components for:
//! - Single selection menus
//! - Multi-select with toggles
//! - Password/secret input (hidden)
//! - Yes/no confirmations
//! - Styled headers and step indicators

use std::{
    cell::Cell,
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
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use secrecy::SecretString;

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
    static PROMPT_TUI_SESSION_DEPTH: Cell<usize> = const { Cell::new(0) };
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

/// Returns true when the current thread has an active shared TUI prompt session.
pub fn tui_prompt_session_active() -> bool {
    PROMPT_TUI_SESSION_DEPTH.with(|depth| depth.get() > 0)
}

/// Guard for a shared prompt TUI session.
///
/// When active, TUI prompts reuse the same terminal session instead of repeatedly
/// entering/leaving alternate screen mode.
pub struct TuiPromptSessionGuard {
    owns_terminal: bool,
}

impl Drop for TuiPromptSessionGuard {
    fn drop(&mut self) {
        let should_cleanup = PROMPT_TUI_SESSION_DEPTH.with(|depth| {
            let current = depth.get();
            let next = current.saturating_sub(1);
            depth.set(next);
            next == 0 && self.owns_terminal
        });
        if should_cleanup {
            let _ = terminal::disable_raw_mode();
            let _ = io::stdout().execute(LeaveAlternateScreen);
            let _ = io::stdout().execute(cursor::Show);
        }
    }
}

/// Begin (or join) a shared TUI prompt session on the current thread.
pub fn begin_tui_prompt_session() -> io::Result<TuiPromptSessionGuard> {
    let is_first = PROMPT_TUI_SESSION_DEPTH.with(|depth| {
        let current = depth.get();
        depth.set(current + 1);
        current == 0
    });

    if is_first
        && let Err(error) = (|| -> io::Result<()> {
            terminal::enable_raw_mode()?;
            let mut stdout = io::stdout();
            stdout.execute(EnterAlternateScreen)?;
            stdout.execute(cursor::Hide)?;
            Ok(())
        })()
    {
        PROMPT_TUI_SESSION_DEPTH.with(|depth| {
            let current = depth.get();
            depth.set(current.saturating_sub(1));
        });
        return Err(error);
    }

    Ok(TuiPromptSessionGuard {
        owns_terminal: is_first,
    })
}

fn with_tui_terminal<F, T>(mut body: F) -> io::Result<T>
where
    F: FnMut(&mut Terminal<CrosstermBackend<io::Stdout>>) -> io::Result<T>,
{
    if tui_prompt_session_active() {
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        return body(&mut terminal);
    }

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
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(9),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header_lines = if let Some(sub) = subtitle {
        vec![
            Line::from(Span::styled(
                "ThinClaw Humanist Cockpit",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(title, Style::default().fg(Color::Cyan).bold())),
            Line::from(Span::styled(sub, Style::default().fg(Color::Gray))),
        ]
    } else {
        vec![
            Line::from(Span::styled(
                "ThinClaw Humanist Cockpit",
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(title, Style::default().fg(Color::Cyan).bold())),
        ]
    };

    frame.render_widget(
        Paragraph::new(header_lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            ),
        layout[0],
    );

    let items: Vec<ListItem> = options
        .iter()
        .enumerate()
        .map(|(idx, option)| {
            let is_cursor = idx == cursor_idx;
            let is_selected = selected.is_some_and(|s| s.get(idx).copied().unwrap_or(false));
            let marker = if selected.is_some() {
                if is_selected { "[x]" } else { "[ ]" }
            } else if is_cursor {
                "›"
            } else {
                " "
            };
            let style = if is_cursor {
                Style::default().fg(Color::Cyan).bold().bg(Color::Black)
            } else if is_selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(
                format!("{} {}", marker, option),
                style,
            )))
        })
        .collect();

    frame.render_widget(
        List::new(items).block(
            Block::default()
                .title(" Choices ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        layout[1],
    );

    frame.render_widget(
        Paragraph::new(help).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        layout[2],
    );
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
                    "Arrow keys move, Enter selects, Esc leaves onboarding",
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
                    "Arrow keys move, Space toggles, Enter confirms the loadout",
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
        loop {
            terminal.draw(|frame| {
                let layout = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(5),
                        Constraint::Length(5),
                        Constraint::Min(4),
                    ])
                    .split(frame.area());

                let mut header = vec![
                    Line::from(Span::styled(
                        "ThinClaw Humanist Cockpit",
                        Style::default().fg(Color::Yellow),
                    )),
                    Line::from(Span::styled(
                        prompt,
                        Style::default().fg(Color::Cyan).bold(),
                    )),
                ];
                if let Some(h) = hint {
                    header.push(Line::from(Span::styled(
                        h,
                        Style::default().fg(Color::Gray),
                    )));
                }
                frame.render_widget(
                    Paragraph::new(header).block(
                        Block::default()
                            .borders(Borders::BOTTOM)
                            .border_style(Style::default().fg(Color::DarkGray)),
                    ),
                    layout[0],
                );

                let visible = if secret {
                    "*".repeat(value.chars().count())
                } else {
                    value.clone()
                };
                frame.render_widget(
                    Paragraph::new(visible).block(
                        Block::default()
                            .title(" Input ")
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Cyan)),
                    ),
                    layout[1],
                );

                frame.render_widget(
                    Paragraph::new(
                        "Type to edit. Backspace deletes, Enter confirms, Esc leaves this prompt.",
                    )
                    .block(
                        Block::default()
                            .borders(Borders::TOP)
                            .border_style(Style::default().fg(Color::DarkGray)),
                    ),
                    layout[2],
                );
            })?;

            if event::poll(std::time::Duration::from_millis(250))?
                && let Event::Key(KeyEvent {
                    code, modifiers, ..
                }) = event::read()?
            {
                match code {
                    KeyCode::Enter => return Ok(value),
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Esc => return Err(io::Error::new(io::ErrorKind::Interrupted, "Esc")),
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Err(io::Error::new(io::ErrorKind::Interrupted, "Ctrl-C"));
                    }
                    KeyCode::Char(ch) => value.push(ch),
                    _ => {}
                }
            }
        }
    })
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
    let width = text.len() + 4;
    let border = "─".repeat(width);

    println!();
    println!("╭{}╮", border);
    println!("│  {}  │", text);
    println!("╰{}╯", border);
    println!();
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
    let progress_width = 24;
    let filled = if total == 0 {
        0
    } else {
        current.saturating_mul(progress_width) / total
    };
    let empty = progress_width.saturating_sub(filled);
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));

    println!("Flight step {}/{}: {}", current, total, name);
    println!(
        "{}  {:>3}%",
        bar,
        current.saturating_mul(100) / total.max(1)
    );
    println!();
}

/// Print a success message with green checkmark.
pub fn print_success(message: &str) {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, SetForegroundColor(CrosstermColor::Green));
    print!("✓");
    let _ = execute!(stdout, ResetColor);
    println!(" {}", message);
}

/// Print an error message with red X.
pub fn print_error(message: &str) {
    let mut stderr = io::stderr();
    let _ = execute!(stderr, SetForegroundColor(CrosstermColor::Red));
    eprint!("✗");
    let _ = execute!(stderr, ResetColor);
    eprintln!(" {}", message);
}

/// Print an info message with blue info icon.
pub fn print_info(message: &str) {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, SetForegroundColor(CrosstermColor::Blue));
    print!("ℹ");
    let _ = execute!(stdout, ResetColor);
    println!(" {}", message);
}

/// Print a warning message with a yellow marker.
pub fn print_warning(message: &str) {
    let mut stdout = io::stdout();
    let _ = execute!(stdout, SetForegroundColor(CrosstermColor::Yellow));
    print!("!");
    let _ = execute!(stdout, ResetColor);
    println!(" {}", message);
}

/// Print a phase banner with a short description.
pub fn print_phase_banner(title: &str, description: &str) {
    let width = title.len().max(description.len()).max(24) + 4;
    let border = "═".repeat(width);

    println!();
    let mut stdout = io::stdout();
    let _ = execute!(stdout, SetForegroundColor(CrosstermColor::Cyan));
    println!("╔{}╗", border);
    println!("║  {:width$}  ║", title, width = width);
    let _ = execute!(stdout, ResetColor);
    println!("╚{}╝", border);
    println!("  {}", description);
    println!("  Stay with the recommended route if you want the safest fast path.");
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
