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

use std::io;
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use crate::channels::StatusUpdate;

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
    /// Connection status text.
    status_text: String,
    /// Currently streaming response.
    active_stream: Option<StreamState>,
    /// Whether to show thinking blocks.
    show_thinking: bool,
    /// Ctrl+C double-tap tracking.
    last_ctrl_c: Option<Instant>,
    /// Channel for sending user messages out.
    outgoing_tx: mpsc::Sender<TuiEvent>,
    /// Channel for receiving status updates.
    incoming_rx: mpsc::Receiver<TuiUpdate>,
    /// Total lines in the rendered chat (for scroll bounds).
    total_chat_lines: u16,
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
        Self {
            messages: Vec::new(),
            input: String::new(),
            cursor_pos: 0,
            input_history: Vec::new(),
            input_history_idx: None,
            scroll_offset: 0,
            model: "default".to_string(),
            agent_id: "main".to_string(),
            status_text: "Connected".to_string(),
            active_stream: None,
            show_thinking: true,
            last_ctrl_c: None,
            outgoing_tx,
            incoming_rx,
            total_chat_lines: 0,
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
            text: "Welcome to ThinClaw TUI. Type /help for commands.".to_string(),
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
                self.status_text = format!("Running tool: {name}");
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
                self.status_text = format!("Tool {name} completed");
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
                self.status_text = "Ready".to_string();
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::Status(text) => {
                self.status_text = text;
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
                self.status_text = format!("Waiting for approval: {tool_name}");
            }
            TuiUpdate::Error(msg) => {
                self.active_stream = None;
                self.messages.push(ChatMessage::System {
                    text: format!("❌ {msg}"),
                });
                self.status_text = "Error".to_string();
            }
        }
    }

    async fn handle_slash_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        let command = parts[0];
        let _arg = parts.get(1).copied().unwrap_or("");

        match command {
            "/help" => {
                self.messages.push(ChatMessage::System {
                    text: HELP_TEXT.to_string(),
                });
            }
            "/clear" | "/cls" => {
                self.messages.clear();
                self.scroll_offset = 0;
            }
            "/new" | "/reset" => {
                self.messages.clear();
                self.scroll_offset = 0;
                self.messages.push(ChatMessage::System {
                    text: "Session reset.".to_string(),
                });
            }
            "/exit" | "/quit" => {
                // Will be handled by the event loop
            }
            "/think" => {
                self.show_thinking = !self.show_thinking;
                self.messages.push(ChatMessage::System {
                    text: format!(
                        "Thinking display: {}",
                        if self.show_thinking { "on" } else { "off" }
                    ),
                });
            }
            "/status" => {
                self.messages.push(ChatMessage::System {
                    text: format!("Model: {} | Agent: {} | {}", self.model, self.agent_id, self.status_text),
                });
            }
            "/interrupt" => {
                let _ = self.outgoing_tx.send(TuiEvent::Abort).await;
                self.active_stream = None;
                self.status_text = "Interrupted".to_string();
                self.messages.push(ChatMessage::System {
                    text: "Operation interrupted.".to_string(),
                });
            }
            // Commands forwarded to the agent loop (they need agent-side handling)
            "/undo" | "/redo" | "/compact" | "/model" | "/models" | "/agent" | "/agents" => {
                let _ = self
                    .outgoing_tx
                    .send(TuiEvent::UserMessage(cmd.to_string()))
                    .await;
            }
            _ => {
                self.messages.push(ChatMessage::System {
                    text: format!("Unknown command: {command}. Type /help for available commands."),
                });
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

        match tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}{stderr}");
                let truncated: String = combined.lines().take(50).collect::<Vec<_>>().join("\n");

                if !truncated.is_empty() {
                    self.messages.push(ChatMessage::System { text: truncated });
                }

                self.messages.push(ChatMessage::System {
                    text: format!("exit {}", output.status.code().unwrap_or(-1)),
                });
            }
            Err(e) => {
                self.messages.push(ChatMessage::System {
                    text: format!("Shell error: {e}"),
                });
            }
        }
    }

    fn autocomplete_command(&mut self) {
        const COMMANDS: &[&str] = &[
            "/help", "/clear", "/new", "/reset", "/exit", "/quit", "/think",
            "/status", "/interrupt", "/undo", "/redo", "/compact",
            "/model", "/models", "/agent", "/agents",
        ];

        let matches: Vec<&&str> = COMMANDS
            .iter()
            .filter(|c| c.starts_with(&self.input))
            .collect();

        if matches.len() == 1 {
            self.input = format!("{} ", matches[0]);
            self.cursor_pos = self.input.chars().count();
        }
    }

    // Rendering methods are in tui/rendering.rs
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

const HELP_TEXT: &str = "\
━━━ ThinClaw TUI Commands ━━━

  /help          Show this help
  /clear         Clear chat history
  /new, /reset   Start a new session
  /think         Toggle thinking display
  /status        Show current model and status
  /interrupt     Abort current operation
  /exit, /quit   Exit the TUI

  /undo          Undo the last turn
  /redo          Redo an undone turn
  /compact       Compact context window
  /model <name>  Switch model
  /models        List available models
  /agent <name>  Switch agent
  /agents        List available agents

  !<command>     Run a local shell command

━━━ Key Bindings ━━━

  Enter          Send message
  Ctrl+C         Abort active run / double-tap to exit
  Ctrl+L         Clear screen
  Up/Down        Input history
  PageUp/Down    Scroll chat
  Tab            Autocomplete commands
  Home/End       Jump to start/end of input
";

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(HELP_TEXT.contains("/help"));
        assert!(HELP_TEXT.contains("Ctrl+C"));
    }
}
