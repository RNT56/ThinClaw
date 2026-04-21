//! Full-screen TUI chat interface using `ratatui`.
//!
//! Provides a rich terminal-based chat UI with:
//! - Full-screen layout (header, scrollable chat, input, status bar)
//! - Streaming token display with cursor animation
//! - Tool call display boxes inline
//! - Slash command support
//! - Input history (up/down arrows)
//! - Scroll (PageUp/PageDown)
//! - Ctrl+C: abort active run / double-tap to exit
//! - Local shell via `!` prefix

mod rendering;
pub mod skin;

use std::io;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use crate::channels::StatusUpdate;
use crate::platform::shell_launcher;
use crate::settings::Settings;
use crate::tui::skin::CliSkin;

static RUNTIME_GATEWAY_URL_OVERRIDE: RwLock<Option<String>> = RwLock::new(None);

/// Set or clear a runtime-resolved Web UI URL override for the TUI startup card.
///
/// This is used by the host runtime to inject the live gateway URL that includes
/// the effective auth token (which may be generated at startup and therefore not
/// available in settings/env at render time).
pub fn set_runtime_gateway_url_override(url: Option<String>) {
    if let Ok(mut guard) = RUNTIME_GATEWAY_URL_OVERRIDE.write() {
        *guard = url
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
}

fn runtime_gateway_url_override() -> Option<String> {
    RUNTIME_GATEWAY_URL_OVERRIDE
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
}

/// A message in the chat history for rendering.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    User {
        text: String,
    },
    Assistant {
        text: String,
        model: Option<String>,
    },
    System {
        text: String,
    },
    ToolCall {
        name: String,
        args: String,
        result: Option<String>,
        is_error: bool,
    },
}

/// Action returned by key handler.
enum KeyAction {
    Continue,
    Exit,
    Submit(String),
}

/// State for active streaming response.
struct StreamState {
    content_text: String,
    thinking_text: String,
}

impl StreamState {
    fn display_text(&self) -> String {
        if self.thinking_text.is_empty() {
            self.content_text.clone()
        } else if self.content_text.is_empty() {
            format!("💭 {}", self.thinking_text)
        } else {
            format!("💭 {}\n\n{}", self.thinking_text, self.content_text)
        }
    }
}

/// Full-screen TUI chat application.
pub struct TuiApp {
    /// Chat message history for rendering.
    messages: Vec<ChatMessage>,
    /// Current input text.
    input: String,
    /// Input cursor position.
    cursor_pos: usize,
    /// Input history (up/down arrows).
    input_history: Vec<String>,
    /// Current position in history.
    input_history_idx: Option<usize>,
    /// Scroll offset for chat area.
    scroll_offset: u16,
    /// Active model display name.
    model: String,
    /// Active agent ID.
    agent_id: String,
    /// Active CLI skin.
    skin: CliSkin,
    /// Default skin name captured at startup for reset handling.
    default_skin_name: String,
    /// Connection status text.
    status_text: String,
    /// Currently streaming response.
    active_stream: Option<StreamState>,
    /// Whether to show thinking blocks.
    show_thinking: bool,
    /// Ctrl+C double-tap tracking.
    last_ctrl_c: Option<Instant>,
    /// Exit requested by a slash command.
    pending_exit: bool,
    /// Channel for sending user messages out.
    outgoing_tx: mpsc::Sender<TuiEvent>,
    /// Channel for receiving status updates.
    incoming_rx: mpsc::Receiver<TuiUpdate>,
    /// Total lines in the rendered chat (for scroll bounds).
    total_chat_lines: u16,
    /// Startup guidance shown in the first system card.
    startup_message: String,
}

/// Events the TUI sends to the agent controller.
#[derive(Debug)]
pub enum TuiEvent {
    /// User submitted a message.
    UserMessage(String),
    /// User requested abort.
    Abort,
    /// User exited the TUI.
    Exit,
}

/// Updates sent to the TUI from the agent/channel manager.
#[derive(Debug, Clone)]
pub enum TuiUpdate {
    /// Agent is thinking/processing.
    Thinking(String),
    /// Streaming text chunk.
    StreamChunk(String),
    /// Tool started.
    ToolStarted { name: String },
    /// Tool completed with result.
    ToolResult {
        name: String,
        result: String,
        is_error: bool,
    },
    /// Final response from the agent.
    Response(String),
    /// Status message.
    Status(String),
    /// Model changed.
    ModelChanged(String),
    /// Approval needed.
    ApprovalNeeded {
        tool_name: String,
        description: String,
    },
    /// Error.
    Error(String),
}

impl TuiApp {
    /// Create a new TUI application.
    pub fn new(
        outgoing_tx: mpsc::Sender<TuiEvent>,
        incoming_rx: mpsc::Receiver<TuiUpdate>,
    ) -> Self {
        let settings = Settings::load();
        let default_skin_name = std::env::var("AGENT_CLI_SKIN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| settings.agent.cli_skin.clone());
        let skin = CliSkin::load(&default_skin_name);
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            input_history: Vec::new(),
            input_history_idx: None,
            scroll_offset: 0,
            model: "default".to_string(),
            agent_id: "main".to_string(),
            skin,
            default_skin_name,
            status_text: "Connected • ready".to_string(),
            active_stream: None,
            show_thinking: true,
            last_ctrl_c: None,
            pending_exit: false,
            outgoing_tx,
            incoming_rx,
            total_chat_lines: 0,
            startup_message: build_startup_message(&settings),
        }
    }

    /// Run the TUI event loop.
    pub async fn run(&mut self) -> io::Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        io::stdout().execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal).await;

        // Restore terminal
        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<()> {
        // Add welcome message
        self.messages.push(ChatMessage::System {
            text: self.startup_message.clone(),
        });

        loop {
            // Render
            terminal.draw(|frame| self.render(frame))?;

            // Poll for events with 50ms tick for smooth streaming
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    // Check for keyboard input
                    while event::poll(Duration::ZERO)? {
                        if let Event::Key(key) = event::read()? {
                            match self.handle_key(key) {
                                KeyAction::Exit => {
                                    let _ = self.outgoing_tx.send(TuiEvent::Exit).await;
                                    return Ok(());
                                }
                                KeyAction::Submit(text) => {
                                    self.handle_submit(&text).await;
                                    if self.pending_exit {
                                        return Ok(());
                                    }
                                }
                                KeyAction::Continue => {}
                            }
                        }
                    }
                }
                Some(update) = self.incoming_rx.recv() => {
                    self.handle_update(update);
                }
            }
        }
    }

    fn handle_key(&mut self, key: event::KeyEvent) -> KeyAction {
        match (key.modifiers, key.code) {
            // Ctrl+C: abort active or double-tap to exit
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                if self.active_stream.is_some() {
                    self.active_stream = None;
                    let tx = self.outgoing_tx.clone();
                    tokio::spawn(async move {
                        let _ = tx.send(TuiEvent::Abort).await;
                    });
                    self.messages.push(ChatMessage::System {
                        text: "[aborted]".to_string(),
                    });
                } else if self
                    .last_ctrl_c
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(1000))
                {
                    return KeyAction::Exit;
                } else {
                    self.last_ctrl_c = Some(Instant::now());
                    self.input.clear();
                    self.cursor_pos = 0;
                    self.status_text = "Press Ctrl+C again to exit".to_string();
                }
                KeyAction::Continue
            }
            // Ctrl+L: clear screen
            (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
                self.messages.clear();
                self.scroll_offset = 0;
                KeyAction::Continue
            }
            // Enter: submit
            (_, KeyCode::Enter) => {
                if self.input.is_empty() {
                    return KeyAction::Continue;
                }
                let text = self.input.clone();
                self.input_history.push(text.clone());
                self.input_history_idx = None;
                self.input.clear();
                self.cursor_pos = 0;
                KeyAction::Submit(text)
            }
            // Up: history prev
            (_, KeyCode::Up) => {
                if self.input_history.is_empty() {
                    return KeyAction::Continue;
                }
                let idx = match self.input_history_idx {
                    Some(i) if i > 0 => i - 1,
                    Some(i) => i,
                    None => self.input_history.len() - 1,
                };
                self.input_history_idx = Some(idx);
                self.input = self.input_history[idx].clone();
                self.cursor_pos = self.input.chars().count();
                KeyAction::Continue
            }
            // Down: history next
            (_, KeyCode::Down) => {
                if let Some(idx) = self.input_history_idx {
                    if idx + 1 < self.input_history.len() {
                        let new_idx = idx + 1;
                        self.input_history_idx = Some(new_idx);
                        self.input = self.input_history[new_idx].clone();
                        self.cursor_pos = self.input.chars().count();
                    } else {
                        self.input_history_idx = None;
                        self.input.clear();
                        self.cursor_pos = 0;
                    }
                }
                KeyAction::Continue
            }
            // PageUp/PageDown: scroll
            (_, KeyCode::PageUp) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                KeyAction::Continue
            }
            (_, KeyCode::PageDown) => {
                self.scroll_offset = self
                    .scroll_offset
                    .saturating_add(10)
                    .min(self.total_chat_lines);
                KeyAction::Continue
            }
            // Home/End in input
            (_, KeyCode::Home) => {
                self.cursor_pos = 0;
                KeyAction::Continue
            }
            (_, KeyCode::End) => {
                self.cursor_pos = self.input.chars().count();
                KeyAction::Continue
            }
            // Left/Right cursor
            (_, KeyCode::Left) => {
                self.cursor_pos = self.cursor_pos.saturating_sub(1);
                KeyAction::Continue
            }
            (_, KeyCode::Right) => {
                if self.cursor_pos < self.input.chars().count() {
                    self.cursor_pos += 1;
                }
                KeyAction::Continue
            }
            // Backspace
            (_, KeyCode::Backspace) => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    // Convert char index to byte offset for String::remove()
                    if let Some((byte_idx, _)) = self.input.char_indices().nth(self.cursor_pos) {
                        self.input.remove(byte_idx);
                    }
                }
                KeyAction::Continue
            }
            // Delete
            (_, KeyCode::Delete) => {
                if self.cursor_pos < self.input.chars().count() {
                    // Convert char index to byte offset for String::remove()
                    if let Some((byte_idx, _)) = self.input.char_indices().nth(self.cursor_pos) {
                        self.input.remove(byte_idx);
                    }
                }
                KeyAction::Continue
            }
            // Tab: autocomplete slash commands
            (_, KeyCode::Tab) => {
                if self.input.starts_with('/') {
                    self.autocomplete_command();
                }
                KeyAction::Continue
            }
            // Character input
            (_, KeyCode::Char(c)) => {
                // Convert char index to byte offset for String::insert()
                let byte_pos = self
                    .input
                    .char_indices()
                    .nth(self.cursor_pos)
                    .map(|(i, _)| i)
                    .unwrap_or(self.input.len());
                self.input.insert(byte_pos, c);
                self.cursor_pos += 1;
                KeyAction::Continue
            }
            _ => KeyAction::Continue,
        }
    }

    async fn handle_submit(&mut self, text: &str) {
        // Slash commands
        if text.starts_with('/') {
            self.handle_slash_command(text).await;
            if self.pending_exit {
                let _ = self.outgoing_tx.send(TuiEvent::Exit).await;
            }
            return;
        }

        // Local shell
        if text.starts_with('!') {
            self.handle_bang_line(text).await;
            return;
        }

        // Regular message → send to agent
        self.messages.push(ChatMessage::User {
            text: text.to_string(),
        });

        // Auto-scroll to bottom
        self.scroll_offset = u16::MAX;

        // Start streaming state
        self.active_stream = Some(StreamState {
            content_text: String::new(),
            thinking_text: String::new(),
        });

        let _ = self
            .outgoing_tx
            .send(TuiEvent::UserMessage(text.to_string()))
            .await;
    }

    fn handle_update(&mut self, update: TuiUpdate) {
        match update {
            TuiUpdate::StreamChunk(chunk) => {
                if let Some(stream) = &mut self.active_stream {
                    stream.content_text.push_str(&chunk);
                } else {
                    // Start a new stream if one wasn't active
                    self.active_stream = Some(StreamState {
                        content_text: chunk,
                        thinking_text: String::new(),
                    });
                }
                // Auto-scroll while streaming
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::Thinking(text) => {
                if let Some(stream) = &mut self.active_stream {
                    stream.thinking_text = text;
                }
            }
            TuiUpdate::ToolStarted { name } => {
                self.messages.push(ChatMessage::ToolCall {
                    name: name.clone(),
                    args: String::new(),
                    result: None,
                    is_error: false,
                });
                self.status_text = format!("Inspecting tool: {}", self.skin.tool_label(&name));
            }
            TuiUpdate::ToolResult {
                name,
                result,
                is_error,
            } => {
                // Update the last tool call message
                if let Some(ChatMessage::ToolCall {
                    result: r,
                    is_error: e,
                    ..
                }) = self.messages.last_mut()
                {
                    *r = Some(result);
                    *e = is_error;
                }
                self.status_text = format!("Tool {} finished", self.skin.tool_label(&name));
            }
            TuiUpdate::Response(text) => {
                // Finalize the stream
                let final_text = if let Some(stream) = self.active_stream.take() {
                    if stream.content_text.is_empty() {
                        text
                    } else {
                        stream.content_text
                    }
                } else {
                    text
                };

                self.messages.push(ChatMessage::Assistant {
                    text: final_text,
                    model: Some(self.model.clone()),
                });
                self.status_text = "Ready for the next turn".to_string();
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::Status(text) => {
                if !text.trim().is_empty() {
                    self.status_text = text;
                }
            }
            TuiUpdate::ModelChanged(model) => {
                self.model = model;
            }
            TuiUpdate::ApprovalNeeded {
                tool_name,
                description,
            } => {
                self.messages.push(ChatMessage::System {
                    text: format!("⚠ Approval needed: {} — {}", tool_name, description),
                });
                self.status_text = format!("Awaiting approval for {tool_name}");
            }
            TuiUpdate::Error(msg) => {
                self.active_stream = None;
                self.messages.push(ChatMessage::System {
                    text: format!("❌ {msg}"),
                });
                self.status_text = "Needs attention".to_string();
            }
        }
    }

    async fn handle_slash_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        let command = parts[0].to_ascii_lowercase();
        let arg = parts.get(1).copied().unwrap_or("").trim();

        match command.as_str() {
            "/help" => {
                self.push_system_note(crate::agent::command_catalog::tui_help_text());
            }
            "/clear" => {
                self.messages.clear();
                self.scroll_offset = 0;
                let _ = self
                    .outgoing_tx
                    .send(TuiEvent::UserMessage("/clear".to_string()))
                    .await;
            }
            "/cls" => {
                self.messages.clear();
                self.scroll_offset = 0;
            }
            "/new" | "/reset" => {
                self.messages.clear();
                self.scroll_offset = 0;
                let forwarded = if command == "/reset" {
                    "/new".to_string()
                } else {
                    command.to_string()
                };
                let _ = self
                    .outgoing_tx
                    .send(TuiEvent::UserMessage(forwarded))
                    .await;
            }
            "/exit" | "/quit" => {
                self.pending_exit = true;
            }
            "/back" | "/close" | "/dismiss" => {
                self.close_last_detail_card();
            }
            "/bottom" => {
                self.scroll_offset = u16::MAX;
                self.status_text = "Jumped to latest activity".to_string();
            }
            "/top" => {
                self.scroll_offset = 0;
                self.status_text = "Jumped to oldest activity".to_string();
            }
            "/think" => {
                self.show_thinking = !self.show_thinking;
                self.push_system_note(format!(
                    "Thinking display: {}",
                    if self.show_thinking { "on" } else { "off" }
                ));
            }
            "/status" => {
                self.push_system_note(format!(
                    "Model: {} | Agent: {} | {}",
                    self.model, self.agent_id, self.status_text
                ));
            }
            "/interrupt" => {
                let _ = self.outgoing_tx.send(TuiEvent::Abort).await;
                self.active_stream = None;
                self.status_text = "Interrupted".to_string();
                self.push_system_note("Operation interrupted.");
            }
            "/skin" => {
                self.handle_skin_command(arg);
            }
            // Commands forwarded to the agent loop (they need agent-side handling)
            cmd if crate::agent::command_catalog::tui_forwarded_commands().contains(&cmd) => {
                let forwarded = if arg.is_empty() {
                    command.to_string()
                } else {
                    format!("{command} {arg}")
                };
                let _ = self
                    .outgoing_tx
                    .send(TuiEvent::UserMessage(forwarded))
                    .await;
                self.scroll_offset = u16::MAX;
                self.status_text = format!("Running {command}...");
            }
            _ => {
                self.push_system_note(format!(
                    "Unknown command: {command}. Type /help for available commands."
                ));
            }
        }
    }

    async fn handle_bang_line(&mut self, line: &str) {
        let cmd = &line[1..];
        if cmd.is_empty() {
            return;
        }

        self.messages.push(ChatMessage::System {
            text: format!("$ {cmd}"),
        });
        self.scroll_offset = u16::MAX;

        match shell_launcher().tokio_command(cmd).output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}{stderr}");
                let truncated: String = combined.lines().take(50).collect::<Vec<_>>().join("\n");

                if !truncated.is_empty() {
                    self.messages.push(ChatMessage::System { text: truncated });
                    self.scroll_offset = u16::MAX;
                }

                self.messages.push(ChatMessage::System {
                    text: format!("exit {}", output.status.code().unwrap_or(-1)),
                });
                self.scroll_offset = u16::MAX;
            }
            Err(e) => {
                self.messages.push(ChatMessage::System {
                    text: format!("Shell error: {e}"),
                });
                self.scroll_offset = u16::MAX;
            }
        }
    }

    fn autocomplete_command(&mut self) {
        let matches: Vec<&&str> = crate::agent::command_catalog::tui_autocomplete_commands()
            .iter()
            .filter(|c| c.starts_with(&self.input))
            .collect();

        if matches.len() == 1 {
            self.input = format!("{} ", matches[0]);
            self.cursor_pos = self.input.chars().count();
        }
    }

    fn handle_skin_command(&mut self, arg: &str) {
        if arg.is_empty() || arg.eq_ignore_ascii_case("current") {
            self.push_system_note(format!(
                "Current skin: {}\nAvailable skins: {}",
                self.skin.name,
                CliSkin::available_names().join(", ")
            ));
            return;
        }

        if arg.eq_ignore_ascii_case("list") {
            self.push_system_note(format!(
                "Available skins: {}",
                CliSkin::available_names().join(", ")
            ));
            return;
        }

        let requested = if arg.eq_ignore_ascii_case("reset") {
            self.default_skin_name.clone()
        } else {
            arg.to_string()
        };
        self.skin = CliSkin::load(&requested);
        self.status_text = format!("Skin switched to {}", self.skin.name);
        self.push_system_note(format!(
            "Skin switched to '{}'. Prompt symbol: {}",
            self.skin.name,
            self.skin.prompt_symbol()
        ));
    }

    fn push_system_note(&mut self, text: impl Into<String>) {
        self.messages
            .push(ChatMessage::System { text: text.into() });
        self.scroll_offset = u16::MAX;
    }

    fn close_last_detail_card(&mut self) {
        if self.active_stream.is_some() {
            self.status_text = "Cannot close detail cards while a run is active".to_string();
            return;
        }

        if let Some(index) = self
            .messages
            .iter()
            .rposition(|message| !matches!(message, ChatMessage::User { .. }))
        {
            self.messages.remove(index);
            self.scroll_offset = u16::MAX;
            self.status_text = "Closed last detail card".to_string();
        } else {
            self.push_system_note("Nothing to close.");
        }
    }

    // Rendering methods are in tui/rendering.rs
}

fn build_startup_message(settings: &Settings) -> String {
    let mut lines = vec![
        "ThinClaw cockpit online. Type /help for controls, or send a message to begin.".to_string(),
    ];
    let access = runtime_access_lines(settings);
    if !access.is_empty() {
        lines.push(String::new());
        lines.push("Access:".to_string());
        lines.extend(access.into_iter().map(|line| format!("  {line}")));
    }
    lines.join("\n")
}

fn runtime_access_lines(settings: &Settings) -> Vec<String> {
    let mut lines = Vec::new();
    if gateway_enabled_from_env() {
        let host = std::env::var("GATEWAY_HOST")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = std::env::var("GATEWAY_PORT")
            .ok()
            .and_then(|value| value.trim().parse::<u16>().ok())
            .or(settings.channels.gateway_port)
            .unwrap_or(3000);
        let base_url = format!("http://{host}:{port}/");
        if let Some(url) = runtime_gateway_url_override() {
            lines.push(format!("Web UI: {url}"));
        } else {
            let gateway_token = std::env::var("GATEWAY_AUTH_TOKEN")
                .ok()
                .or_else(|| settings.channels.gateway_auth_token.clone())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            if let Some(token) = gateway_token {
                lines.push(format!("Web UI: {base_url}?token={token}"));
            } else {
                lines.push(format!("Web UI: {base_url}"));
            }
        }
    }

    let tunnel_url = std::env::var("TUNNEL_URL")
        .ok()
        .or_else(|| settings.tunnel.public_url.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(url) = tunnel_url {
        lines.push(format!("Tunnel: {url}"));
    }

    lines
}

fn gateway_enabled_from_env() -> bool {
    match std::env::var("GATEWAY_ENABLED") {
        Ok(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => true,
    }
}

/// Convert a StatusUpdate to a TuiUpdate.
impl From<StatusUpdate> for TuiUpdate {
    fn from(status: StatusUpdate) -> Self {
        match status {
            StatusUpdate::StreamChunk(chunk) => TuiUpdate::StreamChunk(chunk),
            StatusUpdate::Thinking(text) => TuiUpdate::Thinking(text),
            StatusUpdate::ToolStarted { name, .. } => TuiUpdate::ToolStarted { name },
            StatusUpdate::ToolResult { name, preview } => TuiUpdate::ToolResult {
                name,
                result: preview,
                is_error: false,
            },
            StatusUpdate::ToolCompleted {
                name,
                success: false,
                ..
            } => TuiUpdate::ToolResult {
                name,
                result: "Failed".to_string(),
                is_error: true,
            },
            StatusUpdate::ToolCompleted { .. } => TuiUpdate::Status("Ready".to_string()),
            StatusUpdate::Status(text) => TuiUpdate::Status(text),
            StatusUpdate::Error { message, .. } => TuiUpdate::Error(message),
            StatusUpdate::ApprovalNeeded {
                tool_name,
                description,
                ..
            } => TuiUpdate::ApprovalNeeded {
                tool_name,
                description,
            },
            _ => TuiUpdate::Status(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::helpers::lock_env;

    #[test]
    fn test_stream_state_display() {
        let mut state = StreamState {
            content_text: String::new(),
            thinking_text: String::new(),
        };
        assert_eq!(state.display_text(), "");

        state.content_text = "Hello".to_string();
        assert_eq!(state.display_text(), "Hello");

        state.thinking_text = "Let me think...".to_string();
        assert!(state.display_text().contains("💭"));
        assert!(state.display_text().contains("Hello"));
    }

    #[test]
    fn test_tui_update_from_status() {
        let chunk = StatusUpdate::StreamChunk("hello".to_string());
        let update: TuiUpdate = chunk.into();
        assert!(matches!(update, TuiUpdate::StreamChunk(s) if s == "hello"));

        let error = StatusUpdate::Error {
            message: "oops".to_string(),
            code: None,
        };
        let update: TuiUpdate = error.into();
        assert!(matches!(update, TuiUpdate::Error(s) if s == "oops"));
    }

    #[test]
    fn test_help_text() {
        let help = crate::agent::command_catalog::tui_help_text();
        assert!(help.contains("/help"));
        assert!(help.contains("Ctrl+C"));
        assert!(help.contains("/back"));
    }

    #[tokio::test]
    async fn test_help_command_scrolls_to_latest() {
        let (tx, _rx) = mpsc::channel(4);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut app = TuiApp::new(tx, update_rx);
        app.scroll_offset = 0;

        app.handle_slash_command("/help").await;

        assert_eq!(app.scroll_offset, u16::MAX);
        assert!(matches!(
            app.messages.last(),
            Some(ChatMessage::System { text }) if text.contains("Agent cockpit controls")
        ));
    }

    #[tokio::test]
    async fn test_back_command_closes_last_detail_card() {
        let (tx, _rx) = mpsc::channel(4);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut app = TuiApp::new(tx, update_rx);
        app.messages.push(ChatMessage::User {
            text: "/context detail".to_string(),
        });
        app.messages.push(ChatMessage::Assistant {
            text: "full context detail".to_string(),
            model: Some("test-model".to_string()),
        });

        app.handle_slash_command("/back").await;

        assert!(matches!(
            app.messages.last(),
            Some(ChatMessage::User { text }) if text == "/context detail"
        ));
        assert_eq!(app.status_text, "Closed last detail card");
    }

    #[test]
    fn test_runtime_access_lines_include_webui_and_tunnel() {
        let _guard = lock_env();
        set_runtime_gateway_url_override(None);
        unsafe {
            std::env::set_var("GATEWAY_ENABLED", "true");
            std::env::set_var("GATEWAY_HOST", "127.0.0.1");
            std::env::set_var("GATEWAY_PORT", "3100");
            std::env::set_var("GATEWAY_AUTH_TOKEN", "abc123");
            std::env::set_var("TUNNEL_URL", "https://agent.example.com");
        }
        let settings = Settings::default();
        let lines = runtime_access_lines(&settings);
        assert!(
            lines
                .iter()
                .any(|line| line == "Web UI: http://127.0.0.1:3100/?token=abc123")
        );
        assert!(
            lines
                .iter()
                .any(|line| line == "Tunnel: https://agent.example.com")
        );
        unsafe {
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
            std::env::remove_var("TUNNEL_URL");
        }
        set_runtime_gateway_url_override(None);
    }

    #[test]
    fn test_runtime_access_lines_hide_webui_when_gateway_disabled() {
        let _guard = lock_env();
        set_runtime_gateway_url_override(None);
        unsafe {
            std::env::set_var("GATEWAY_ENABLED", "false");
            std::env::set_var("TUNNEL_URL", "https://agent.example.com");
        }
        let settings = Settings::default();
        let lines = runtime_access_lines(&settings);
        assert!(!lines.iter().any(|line| line.starts_with("Web UI:")));
        assert!(
            lines
                .iter()
                .any(|line| line == "Tunnel: https://agent.example.com")
        );
        unsafe {
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("TUNNEL_URL");
        }
        set_runtime_gateway_url_override(None);
    }

    #[test]
    fn test_runtime_access_lines_prefers_runtime_gateway_override() {
        let _guard = lock_env();
        unsafe {
            std::env::set_var("GATEWAY_ENABLED", "true");
            std::env::set_var("GATEWAY_HOST", "127.0.0.1");
            std::env::set_var("GATEWAY_PORT", "3100");
            std::env::set_var("GATEWAY_AUTH_TOKEN", "env-token");
        }
        set_runtime_gateway_url_override(Some(
            "http://127.0.0.1:3100/?token=runtime-token".to_string(),
        ));
        let settings = Settings::default();
        let lines = runtime_access_lines(&settings);
        assert!(
            lines
                .iter()
                .any(|line| line == "Web UI: http://127.0.0.1:3100/?token=runtime-token")
        );
        assert!(!lines.iter().any(|line| line.contains("env-token")));
        unsafe {
            std::env::remove_var("GATEWAY_ENABLED");
            std::env::remove_var("GATEWAY_HOST");
            std::env::remove_var("GATEWAY_PORT");
            std::env::remove_var("GATEWAY_AUTH_TOKEN");
        }
        set_runtime_gateway_url_override(None);
    }
}
