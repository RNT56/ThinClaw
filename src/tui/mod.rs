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

mod rendering;
pub mod skin;
pub mod spinner;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::time::{Duration, Instant};

use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::*;
use ratatui_textarea::{Input, Key, TextArea};
use tokio::sync::mpsc;

use crate::settings::Settings;
use crate::tui::skin::CliSkin;
use crate::tui::spinner::KawaiiSpinner;
pub use thinclaw_channels::tui::{TuiApprovalDecision, TuiEvent, TuiUpdate};

pub(crate) fn credential_free_gateway_url(value: &str) -> Option<String> {
    let mut parsed = url::Url::parse(value.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return None;
    }
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.set_path("/");
    Some(parsed.to_string())
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
    /// Neutral system information (help text, shell output, status).
    System {
        text: String,
    },
    /// Positive confirmation (skin changed, command succeeded, etc.).
    Info {
        text: String,
    },
    /// Actionable warning (approval needed, interrupted, etc.).
    Warning {
        text: String,
    },
    /// Error requiring attention.
    Error {
        text: String,
    },
    ToolCall {
        invocation_id: thinclaw_types::ToolInvocationId,
        name: String,
        args: String,
        result: Option<String>,
        is_error: bool,
        completed: bool,
        duration_ms: Option<u64>,
        artifact_count: usize,
    },
    /// Structured note from the agent (warning, question, interim_result).
    AgentNote {
        content: String,
        note_type: String,
    },
    /// Sub-agent lifecycle card.
    SubagentCard {
        agent_id: String,
        name: String,
        detail: String,
        success: Option<bool>,
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

#[derive(Debug)]
struct ApprovalPrompt {
    request_id: String,
    tool_name: String,
    description: String,
    redacted_parameters: String,
    received_at: Instant,
}

impl StreamState {
    fn display_text(&self) -> String {
        // Reasoning text is not a user-visible surface until a policy-safe,
        // explicitly authorized reasoning view exists.
        self.content_text.clone()
    }
}

/// Full-screen TUI chat application.
pub struct TuiApp {
    /// Chat message history for rendering.
    messages: Vec<ChatMessage>,
    /// Multi-line text area widget for input.
    textarea: TextArea<'static>,
    /// Input history (up/down arrows).
    input_history: Vec<String>,
    /// Current position in history.
    input_history_idx: Option<usize>,
    /// Saved input before history navigation started.
    pre_history_input: Option<String>,
    /// Scroll offset for chat area.
    scroll_offset: u16,
    /// Active model display name.
    model: String,
    /// Active agent ID.
    agent_id: String,
    /// Durable direct conversation selected by the runtime.
    conversation_id: String,
    history_before_cursor: Option<String>,
    history_has_more: bool,
    history_loading: bool,
    loaded_history_ids: HashSet<String>,
    capabilities: thinclaw_channels::tui::TuiCapabilitySnapshot,
    /// Active CLI skin.
    skin: CliSkin,
    /// Default skin name captured at startup for reset handling.
    default_skin_name: String,
    /// Connection status text.
    status_text: String,
    /// Whether an authoritative runtime update has been received.
    runtime_state_seen: bool,
    /// Local diagnostic rendering toggle.
    debug_enabled: bool,
    /// Currently streaming response.
    active_stream: Option<StreamState>,
    /// Ctrl+C double-tap tracking.
    last_ctrl_c: Option<Instant>,
    /// Exit requested by a slash command.
    pending_exit: bool,
    /// Ordered pending approvals, preserving the runtime request identity.
    pending_approvals: VecDeque<ApprovalPrompt>,
    /// Message index for each tool invocation so interleaved events update correctly.
    tool_activity: HashMap<thinclaw_types::ToolInvocationId, usize>,
    /// Channel for sending user messages out.
    outgoing_tx: mpsc::Sender<TuiEvent>,
    /// Channel for receiving status updates.
    incoming_rx: mpsc::Receiver<TuiUpdate>,
    /// Total lines in the rendered chat (for scroll bounds).
    total_chat_lines: u16,
    /// Startup guidance shown in the first system card.
    startup_message: String,
    /// Animated spinner for thinking/streaming states.
    spinner: KawaiiSpinner,
    /// Tick counter for animation timing.
    animation_tick: u64,
    /// Timestamp of last meaningful activity (for idle display).
    last_activity: Instant,
}

impl TuiApp {
    /// Create a new TUI application.
    pub fn new(
        outgoing_tx: mpsc::Sender<TuiEvent>,
        incoming_rx: mpsc::Receiver<TuiUpdate>,
        bootstrap: thinclaw_channels::tui::TuiBootstrap,
    ) -> Self {
        let settings = Settings::load();
        let default_skin_name = std::env::var("AGENT_CLI_SKIN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| settings.agent.cli_skin.clone());
        let skin = CliSkin::load(&default_skin_name);
        let spinner = KawaiiSpinner::from_skin(&skin, "thinking");
        let textarea = Self::build_textarea(&skin);
        let loaded_history_ids = bootstrap
            .history
            .messages
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let messages = bootstrap
            .history
            .messages
            .iter()
            .map(history_chat_message)
            .collect();
        let startup_message = build_startup_message(&bootstrap);
        Self {
            messages,
            textarea,
            input_history: load_input_history(),
            input_history_idx: None,
            pre_history_input: None,
            scroll_offset: 0,
            model: bootstrap.model,
            agent_id: bootstrap.agent_name,
            conversation_id: bootstrap.history.conversation_id,
            history_before_cursor: bootstrap.history.before_cursor,
            history_has_more: bootstrap.history.has_more,
            history_loading: false,
            loaded_history_ids,
            capabilities: bootstrap.capabilities,
            skin,
            default_skin_name,
            status_text: "Awaiting runtime state".to_string(),
            runtime_state_seen: false,
            debug_enabled: false,
            active_stream: None,
            last_ctrl_c: None,
            pending_exit: false,
            pending_approvals: VecDeque::new(),
            tool_activity: HashMap::new(),
            outgoing_tx,
            incoming_rx,
            total_chat_lines: 0,
            startup_message,
            spinner,
            animation_tick: 0,
            last_activity: Instant::now(),
        }
    }

    /// Build a fresh TextArea widget styled for the given skin.
    fn build_textarea(_skin: &CliSkin) -> TextArea<'static> {
        let mut textarea = TextArea::default();
        textarea.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
        textarea.set_cursor_line_style(Style::default());
        textarea
    }

    /// Extract the current textarea content as a single string.
    fn textarea_content(&self) -> String {
        self.textarea.lines().join("\n")
    }

    /// Clear the textarea and reset to a single empty line.
    fn clear_textarea(&mut self) {
        self.textarea = Self::build_textarea(&self.skin);
    }

    fn record_input(&mut self, text: String) {
        if self.input_history.last() != Some(&text) {
            self.input_history.push(text);
        }
        const MAX_INPUT_HISTORY: usize = 1_000;
        if self.input_history.len() > MAX_INPUT_HISTORY {
            self.input_history
                .drain(..self.input_history.len() - MAX_INPUT_HISTORY);
        }
        if let Err(error) = persist_input_history(&self.input_history) {
            tracing::debug!(%error, "Could not persist TUI input history");
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
                    // Advance animation tick
                    self.animation_tick += 1;
                    if self.animation_tick.is_multiple_of(6) {
                        self.spinner.tick();
                    }

                    // Check for keyboard input
                    while event::poll(Duration::ZERO)? {
                        if let Event::Key(key) = event::read()? {
                            match self.handle_key(key) {
                                KeyAction::Exit => {
                                    self.deny_pending_approvals().await;
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
                if self.active_stream.is_some() || !self.pending_approvals.is_empty() {
                    self.active_stream = None;
                    let tx = self.outgoing_tx.clone();
                    let pending_request_ids: Vec<_> = self
                        .pending_approvals
                        .drain(..)
                        .map(|prompt| prompt.request_id)
                        .collect();
                    tokio::spawn(async move {
                        for request_id in pending_request_ids {
                            let _ = tx
                                .send(TuiEvent::ApprovalResponse {
                                    request_id,
                                    decision: TuiApprovalDecision::Deny,
                                })
                                .await;
                        }
                        let _ = tx.send(TuiEvent::Abort).await;
                    });
                    self.messages.push(ChatMessage::Warning {
                        text: "Stream aborted".to_string(),
                    });
                } else if self
                    .last_ctrl_c
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(1000))
                {
                    return KeyAction::Exit;
                } else {
                    self.last_ctrl_c = Some(Instant::now());
                    self.clear_textarea();
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
            // Ctrl+B: back-navigate (close last detail card)
            (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
                self.close_last_detail_card();
                KeyAction::Continue
            }
            // Ctrl+Enter: always submit regardless of content
            (KeyModifiers::CONTROL, KeyCode::Enter) => {
                let text = self.textarea_content();
                if text.trim().is_empty() {
                    return KeyAction::Continue;
                }
                self.record_input(text.clone());
                self.input_history_idx = None;
                self.pre_history_input = None;
                self.clear_textarea();
                KeyAction::Submit(text)
            }
            // Alt+Enter or Shift+Enter: insert newline (multi-line continuation)
            (KeyModifiers::ALT, KeyCode::Enter) | (KeyModifiers::SHIFT, KeyCode::Enter) => {
                self.textarea.input(Self::textarea_input(key));
                KeyAction::Continue
            }
            // Enter: submit if single-line, or if starts with '/'
            (_, KeyCode::Enter) => {
                let text = self.textarea_content();
                if text.trim().is_empty() {
                    return KeyAction::Continue;
                }
                // For single-line input or slash commands, Enter submits
                if self.textarea.lines().len() <= 1 || text.starts_with('/') {
                    self.record_input(text.clone());
                    self.input_history_idx = None;
                    self.pre_history_input = None;
                    self.clear_textarea();
                    return KeyAction::Submit(text);
                }
                // For multi-line input, Enter adds a line
                self.textarea.input(Self::textarea_input(key));
                KeyAction::Continue
            }
            // Up: history prev (only when single-line and cursor at first line)
            (_, KeyCode::Up) if self.textarea.lines().len() <= 1 => {
                if self.input_history.is_empty() {
                    return KeyAction::Continue;
                }
                // Save current input before entering history
                if self.input_history_idx.is_none() {
                    self.pre_history_input = Some(self.textarea_content());
                }
                let idx = match self.input_history_idx {
                    Some(i) if i > 0 => i - 1,
                    Some(i) => i,
                    None => self.input_history.len() - 1,
                };
                self.input_history_idx = Some(idx);
                self.textarea = Self::build_textarea(&self.skin);
                self.textarea.insert_str(&self.input_history[idx]);
                KeyAction::Continue
            }
            // Down: history next (only when single-line)
            (_, KeyCode::Down) if self.textarea.lines().len() <= 1 => {
                if let Some(idx) = self.input_history_idx {
                    if idx + 1 < self.input_history.len() {
                        let new_idx = idx + 1;
                        self.input_history_idx = Some(new_idx);
                        self.textarea = Self::build_textarea(&self.skin);
                        self.textarea.insert_str(&self.input_history[new_idx]);
                    } else {
                        self.input_history_idx = None;
                        self.textarea = Self::build_textarea(&self.skin);
                        if let Some(ref saved) = self.pre_history_input {
                            self.textarea.insert_str(saved);
                        }
                        self.pre_history_input = None;
                    }
                }
                KeyAction::Continue
            }
            // PageUp/PageDown: scroll
            (_, KeyCode::PageUp) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                if self.scroll_offset == 0
                    && self.history_has_more
                    && !self.history_loading
                    && !self.conversation_id.is_empty()
                {
                    self.history_loading = true;
                    let tx = self.outgoing_tx.clone();
                    let conversation_id = self.conversation_id.clone();
                    let before_cursor = self.history_before_cursor.clone();
                    tokio::spawn(async move {
                        let _ = tx
                            .send(TuiEvent::LoadOlder {
                                conversation_id,
                                before_cursor,
                            })
                            .await;
                    });
                }
                KeyAction::Continue
            }
            (_, KeyCode::PageDown) => {
                self.scroll_offset = self
                    .scroll_offset
                    .saturating_add(10)
                    .min(self.total_chat_lines);
                KeyAction::Continue
            }
            // Tab: autocomplete slash commands
            (_, KeyCode::Tab) => {
                let content = self.textarea_content();
                if content.starts_with('/') {
                    self.autocomplete_command();
                } else {
                    self.textarea.input(Self::textarea_input(key));
                }
                KeyAction::Continue
            }
            // All other keys: delegate to TextArea
            _ => {
                self.textarea.input(Self::textarea_input(key));
                KeyAction::Continue
            }
        }
    }

    fn textarea_input(key: event::KeyEvent) -> Input {
        Input {
            key: match key.code {
                KeyCode::Char(ch) => Key::Char(ch),
                KeyCode::F(n) => Key::F(n),
                KeyCode::Backspace => Key::Backspace,
                KeyCode::Enter => Key::Enter,
                KeyCode::Left => Key::Left,
                KeyCode::Right => Key::Right,
                KeyCode::Up => Key::Up,
                KeyCode::Down => Key::Down,
                KeyCode::Tab | KeyCode::BackTab => Key::Tab,
                KeyCode::Delete => Key::Delete,
                KeyCode::Home => Key::Home,
                KeyCode::End => Key::End,
                KeyCode::PageUp => Key::PageUp,
                KeyCode::PageDown => Key::PageDown,
                KeyCode::Esc => Key::Esc,
                _ => Key::Null,
            },
            ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
            alt: key.modifiers.contains(KeyModifiers::ALT),
            shift: key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab,
        }
    }

    async fn handle_submit(&mut self, text: &str) {
        // Slash commands
        if text.starts_with('/') {
            self.handle_slash_command(text).await;
            if self.pending_exit {
                self.deny_pending_approvals().await;
                let _ = self.outgoing_tx.send(TuiEvent::Exit).await;
            }
            return;
        }

        // Raw shell escape was intentionally removed. Do not reinterpret it as chat.
        if text.starts_with('!') {
            self.handle_bang_line(text).await;
            return;
        }

        // Check for approval response when approval is pending
        if let Some(prompt) = self.pending_approvals.front() {
            let lower = text.trim().to_ascii_lowercase();
            if matches!(lower.as_str(), "yes" | "y" | "no" | "n" | "always" | "a") {
                let decision = match lower.as_str() {
                    "yes" | "y" => TuiApprovalDecision::ApproveOnce,
                    "always" | "a" => TuiApprovalDecision::ApproveForSession,
                    _ => TuiApprovalDecision::Deny,
                };
                let request_id = prompt.request_id.clone();
                if self
                    .outgoing_tx
                    .send(TuiEvent::ApprovalResponse {
                        request_id,
                        decision,
                    })
                    .await
                    .is_ok()
                {
                    self.pending_approvals.pop_front();
                    self.push_info(match decision {
                        TuiApprovalDecision::ApproveOnce => "Approved once",
                        TuiApprovalDecision::ApproveForSession => "Approved for this tool/session",
                        TuiApprovalDecision::Deny => "Denied",
                    });
                }
                return;
            }
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
        self.last_activity = Instant::now();
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
            TuiUpdate::ToolStarted {
                invocation_id,
                name,
                parameters,
            } => {
                if self.tool_activity.contains_key(&invocation_id) {
                    self.push_warning(format!("Duplicate tool start ignored: {invocation_id}"));
                    return;
                }
                let args = parameters
                    .as_ref()
                    .map(redact_and_bound_parameters)
                    .unwrap_or_default();
                let index = self.messages.len();
                self.messages.push(ChatMessage::ToolCall {
                    invocation_id: invocation_id.clone(),
                    name: name.clone(),
                    args,
                    result: None,
                    is_error: false,
                    completed: false,
                    duration_ms: None,
                    artifact_count: 0,
                });
                self.tool_activity.insert(invocation_id, index);
                self.status_text = format!("Inspecting tool: {}", self.skin.tool_label(&name));
            }
            TuiUpdate::ToolOutput {
                invocation_id,
                name,
                preview,
                artifacts,
            } => {
                let Some(index) = self.tool_activity.get(&invocation_id).copied() else {
                    self.push_warning(format!(
                        "Tool output arrived for unknown invocation {invocation_id} ({name})"
                    ));
                    return;
                };
                if let Some(ChatMessage::ToolCall {
                    result,
                    artifact_count,
                    ..
                }) = self.messages.get_mut(index)
                {
                    *result = Some(bound_preview(&preview));
                    *artifact_count = artifacts.len();
                }
                self.status_text = format!("Tool {} produced output", self.skin.tool_label(&name));
            }
            TuiUpdate::ToolCompleted {
                invocation_id,
                name,
                success,
                result_preview,
                duration_ms,
            } => {
                let Some(index) = self.tool_activity.get(&invocation_id).copied() else {
                    self.push_warning(format!(
                        "Tool completion arrived for unknown invocation {invocation_id} ({name})"
                    ));
                    return;
                };
                if let Some(ChatMessage::ToolCall {
                    result,
                    is_error,
                    completed,
                    duration_ms: stored_duration,
                    ..
                }) = self.messages.get_mut(index)
                {
                    if *completed {
                        return;
                    }
                    if result.is_none() {
                        *result = result_preview.as_deref().map(bound_preview);
                    }
                    *is_error = !success;
                    *completed = true;
                    *stored_duration = duration_ms;
                }
                self.status_text = format!(
                    "Tool {} {}",
                    self.skin.tool_label(&name),
                    if success { "succeeded" } else { "failed" }
                );
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
                    self.runtime_state_seen = true;
                    self.status_text = text;
                }
            }
            TuiUpdate::ModelChanged(model) => {
                self.runtime_state_seen = true;
                self.model = model;
            }
            TuiUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
            } => {
                if self
                    .pending_approvals
                    .iter()
                    .any(|prompt| prompt.request_id == request_id)
                {
                    self.push_warning(format!("Duplicate approval request ignored: {request_id}"));
                    return;
                }
                let redacted_parameters = redact_and_bound_parameters(&parameters);
                self.pending_approvals.push_back(ApprovalPrompt {
                    request_id: request_id.clone(),
                    tool_name: tool_name.clone(),
                    description: description.clone(),
                    redacted_parameters: redacted_parameters.clone(),
                    received_at: Instant::now(),
                });
                self.messages.push(ChatMessage::Warning {
                    text: format!(
                        "Approval needed [{request_id}]: {tool_name} — {description}\n\
                         Arguments: {redacted_parameters}\n\
                         Type yes (y) / no (n) / always (a) to respond.",
                    ),
                });
                self.status_text = format!("⚠ Awaiting approval for {tool_name}");
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::Error(msg) => {
                self.active_stream = None;
                self.messages.push(ChatMessage::Error { text: msg });
                self.status_text = "Needs attention".to_string();
            }
            TuiUpdate::AgentMessage {
                content,
                message_type,
            } => {
                self.messages.push(ChatMessage::AgentNote {
                    content,
                    note_type: message_type,
                });
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::SubagentSpawned {
                agent_id,
                name,
                task,
            } => {
                self.messages.push(ChatMessage::SubagentCard {
                    agent_id: agent_id.clone(),
                    name: name.clone(),
                    detail: format!("task: {task}"),
                    success: None,
                });
                self.status_text = format!("Sub-agent '{name}' ({agent_id}) running");
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::SubagentProgress { agent_id, message } => {
                if let Some(ChatMessage::SubagentCard { detail, .. }) = self.messages.iter_mut().rev().find(|message| {
                    matches!(message, ChatMessage::SubagentCard { agent_id: id, .. } if id == &agent_id)
                }) {
                    *detail = message.clone();
                }
                self.status_text = format!("Sub-agent {agent_id}: {message}");
            }
            TuiUpdate::SubagentCompleted {
                agent_id,
                name,
                success,
                duration_ms,
            } => {
                let secs = duration_ms as f64 / 1000.0;
                let detail = if success {
                    format!("completed in {secs:.1}s")
                } else {
                    format!("failed after {secs:.1}s")
                };
                self.messages.push(ChatMessage::SubagentCard {
                    agent_id: agent_id.clone(),
                    name: name.clone(),
                    detail,
                    success: Some(success),
                });
                self.status_text = if success {
                    format!("Sub-agent '{name}' ({agent_id}) done")
                } else {
                    format!("Sub-agent '{name}' ({agent_id}) failed")
                };
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::JobStarted {
                title,
                job_id,
                browse_url,
            } => {
                self.messages.push(ChatMessage::Info {
                    text: format!("Job started: {title} ({job_id})\n{browse_url}"),
                });
                self.status_text = format!("Job '{title}' running");
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::AuthRequired {
                extension_name,
                instructions,
            } => {
                let detail = instructions.unwrap_or_default();
                self.messages.push(ChatMessage::Warning {
                    text: format!("Authentication required for {extension_name}\n{detail}"),
                });
                self.status_text = format!("Auth needed: {extension_name}");
                self.scroll_offset = u16::MAX;
            }
            TuiUpdate::AuthCompleted {
                extension_name,
                success,
                message,
            } => {
                if success {
                    self.messages.push(ChatMessage::Info {
                        text: format!("{extension_name}: {message}"),
                    });
                } else {
                    self.messages.push(ChatMessage::Error {
                        text: format!("{extension_name}: {message}"),
                    });
                }
            }
            TuiUpdate::HistoryPage(page) => {
                self.history_loading = false;
                if page.conversation_id != self.conversation_id {
                    self.push_warning("Ignored history page for a different conversation");
                    return;
                }
                let mut older = Vec::new();
                for message in &page.messages {
                    if self.loaded_history_ids.insert(message.id.clone()) {
                        older.push(history_chat_message(message));
                    }
                }
                let added = older.len() as u16;
                older.append(&mut self.messages);
                self.messages = older;
                self.history_before_cursor = page.before_cursor;
                self.history_has_more = page.has_more;
                self.scroll_offset = self.scroll_offset.saturating_add(added);
            }
            TuiUpdate::CapabilitySnapshot(snapshot) => {
                if snapshot.sealed && snapshot.revision > self.capabilities.revision {
                    self.capabilities = snapshot;
                    self.runtime_state_seen = true;
                }
            }
        }
    }

    async fn handle_slash_command(&mut self, cmd: &str) {
        let trimmed = cmd.trim();
        if trimmed.eq_ignore_ascii_case("/think") {
            self.push_warning(
                "`/think` was removed because it did not control a real reasoning view.",
            );
            return;
        }
        let lower = trimmed.to_ascii_lowercase();
        let Some(spec) = thinclaw_types::slash_commands::match_surface_command(&lower) else {
            self.push_warning(format!(
                "Unknown command: {}. Type /help for available commands.",
                lower.split_whitespace().next().unwrap_or(&lower)
            ));
            return;
        };
        let arg = trimmed
            .split_once(char::is_whitespace)
            .map(|(_, value)| value.trim())
            .unwrap_or("");
        match spec.tui {
            thinclaw_types::slash_commands::SurfaceRoute::Local(command) => {
                self.handle_local_command(command, arg).await;
            }
            thinclaw_types::slash_commands::SurfaceRoute::Forward(_) => {
                let _ = self
                    .outgoing_tx
                    .send(TuiEvent::UserMessage(trimmed.to_string()))
                    .await;
                self.scroll_offset = u16::MAX;
                self.status_text = format!("Running {}...", spec.name);
            }
            thinclaw_types::slash_commands::SurfaceRoute::Unsupported => {
                self.push_warning(format!("{} is not available in the TUI", spec.name));
            }
        }
    }

    async fn handle_local_command(
        &mut self,
        command: thinclaw_types::slash_commands::LocalCommand,
        arg: &str,
    ) {
        use thinclaw_types::slash_commands::LocalCommand;
        match command {
            LocalCommand::Help => {
                self.push_system_note(crate::agent::command_catalog::tui_help_text());
            }
            LocalCommand::ClearConversation => {
                self.messages.clear();
                self.scroll_offset = 0;
                let _ = self
                    .outgoing_tx
                    .send(TuiEvent::UserMessage("/clear".to_string()))
                    .await;
            }
            LocalCommand::ClearScreen => {
                self.messages.clear();
                self.scroll_offset = 0;
            }
            LocalCommand::NewConversation => {
                self.messages.clear();
                self.scroll_offset = 0;
                let _ = self
                    .outgoing_tx
                    .send(TuiEvent::UserMessage("/new".to_string()))
                    .await;
            }
            LocalCommand::Quit => {
                self.pending_exit = true;
            }
            LocalCommand::Back => {
                self.close_last_detail_card();
            }
            LocalCommand::Bottom => {
                self.scroll_offset = u16::MAX;
                self.status_text = "Jumped to latest activity".to_string();
            }
            LocalCommand::Top => {
                self.scroll_offset = 0;
                self.status_text = "Jumped to oldest activity".to_string();
            }
            LocalCommand::Status => {
                if self.runtime_state_seen {
                    self.push_system_note(format!(
                        "Runtime model: {} | Agent: {} | Conversation: {} | capabilities r{} | {}",
                        self.model,
                        self.agent_id,
                        if self.conversation_id.is_empty() {
                            "new"
                        } else {
                            &self.conversation_id
                        },
                        self.capabilities.revision,
                        self.status_text
                    ));
                } else {
                    self.push_warning(
                        "Authoritative runtime status has not arrived yet; no placeholder health is shown.",
                    );
                }
            }
            LocalCommand::Debug => {
                self.debug_enabled = !self.debug_enabled;
                self.push_info(format!(
                    "TUI diagnostics {}",
                    if self.debug_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ));
            }
            LocalCommand::Interrupt => {
                self.deny_pending_approvals().await;
                let _ = self.outgoing_tx.send(TuiEvent::Abort).await;
                self.active_stream = None;
                self.status_text = "Interrupted".to_string();
                self.push_warning("Operation interrupted.");
            }
            LocalCommand::Skin => {
                self.handle_skin_command(arg);
            }
            LocalCommand::Tools => {
                self.push_system_note(self.render_tools(arg));
            }
        }
    }

    async fn handle_bang_line(&mut self, _line: &str) {
        self.push_warning(
            "Raw shell escape was removed. Use an approved ThinClaw tool or a separate terminal.",
        );
    }

    async fn deny_pending_approvals(&mut self) {
        while let Some(prompt) = self.pending_approvals.pop_front() {
            let _ = self
                .outgoing_tx
                .send(TuiEvent::ApprovalResponse {
                    request_id: prompt.request_id,
                    decision: TuiApprovalDecision::Deny,
                })
                .await;
        }
    }

    fn autocomplete_command(&mut self) {
        let content = self.textarea_content();
        let matches: Vec<&str> =
            thinclaw_types::slash_commands::autocomplete_names(|spec| spec.tui)
                .filter(|c| c.starts_with(&content))
                .collect();

        if matches.len() == 1 {
            let completed = format!("{} ", matches[0]);
            self.textarea = Self::build_textarea(&self.skin);
            self.textarea.insert_str(&completed);
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
        self.spinner = KawaiiSpinner::from_skin(&self.skin, "thinking");
        self.textarea = Self::build_textarea(&self.skin);
        self.status_text = format!("Skin switched to {}", self.skin.name);
        self.push_info(format!(
            "Skin switched to '{}'. Prompt symbol: {}",
            self.skin.name,
            self.skin.prompt_symbol()
        ));
    }

    fn render_tools(&self, arg: &str) -> String {
        if arg == "--all" {
            let live = self
                .capabilities
                .identities
                .iter()
                .map(|identity| identity.name.as_str())
                .collect::<HashSet<_>>();
            let mut lines = vec![format!(
                "Tool catalog — capability revision {}",
                self.capabilities.revision
            )];
            for descriptor in thinclaw_tools::STATIC_TOOL_CATALOG {
                lines.push(format!(
                    "{}  {:<18} {}",
                    if live.contains(descriptor.name) {
                        "registered"
                    } else {
                        "unavailable"
                    },
                    descriptor.origin,
                    descriptor.name
                ));
            }
            return lines.join("\n");
        }
        if !arg.is_empty() {
            if let Some(identity) = self
                .capabilities
                .identities
                .iter()
                .find(|identity| identity.name == arg)
            {
                return format!(
                    "{} — revision {}\norigin: {}\nsource: {}\ncompiled: {}\nconfigured: {}\nregistered: {}\ndependency: {}\nexposed: {}\nready: {}\napproval: {}\nhealth: {}{}",
                    identity.name,
                    self.capabilities.revision,
                    identity.origin,
                    identity.source_id,
                    identity.compiled,
                    identity
                        .configured
                        .map_or("unknown", |value| if value { "yes" } else { "no" }),
                    identity.registered,
                    identity.dependency,
                    identity.exposed,
                    identity.ready,
                    identity.approval,
                    identity.health,
                    if identity.reasons.is_empty() {
                        String::new()
                    } else {
                        format!("\nreasons: {}", identity.reasons.join(", "))
                    }
                );
            }
            if let Some(descriptor) = thinclaw_tools::static_tool_descriptor(arg) {
                return format!(
                    "{} — revision {}\norigin: {}\nsource: builtin/{}\nregistered: no\nexposed: no\nready: unknown\napproval: conditional\nreason: catalogued but absent from the live registry",
                    descriptor.name, self.capabilities.revision, descriptor.origin, descriptor.name
                );
            }
            return format!("Unknown tool identity: {arg}");
        }

        let mut groups = std::collections::BTreeMap::<&str, usize>::new();
        for identity in &self.capabilities.identities {
            *groups.entry(identity.origin.as_str()).or_default() += 1;
        }
        let mut lines = vec![format!(
            "{} registered tools — capability revision {}",
            self.capabilities.identities.len(),
            self.capabilities.revision
        )];
        lines.extend(
            groups
                .into_iter()
                .map(|(origin, count)| format!("{origin}: {count} registered")),
        );
        lines.push("Use /tools NAME for provenance or /tools --all for the catalog.".to_string());
        lines.join("\n")
    }

    fn push_system_note(&mut self, text: impl Into<String>) {
        self.messages
            .push(ChatMessage::System { text: text.into() });
        self.scroll_offset = u16::MAX;
    }

    fn push_info(&mut self, text: impl Into<String>) {
        self.messages.push(ChatMessage::Info { text: text.into() });
        self.scroll_offset = u16::MAX;
    }

    fn push_warning(&mut self, text: impl Into<String>) {
        self.messages
            .push(ChatMessage::Warning { text: text.into() });
        self.scroll_offset = u16::MAX;
    }

    #[allow(dead_code)]
    fn push_error(&mut self, text: impl Into<String>) {
        self.messages.push(ChatMessage::Error { text: text.into() });
        self.scroll_offset = u16::MAX;
    }

    fn close_last_detail_card(&mut self) {
        if self.active_stream.is_some() {
            self.status_text = "No drawer can be closed while a run is active".to_string();
            return;
        }
        // Transcript entries are durable conversation content, not navigation
        // state. Back may close a typed drawer/modal only; none is open here.
        self.status_text = "No drawer or modal is open".to_string();
    }

    // Rendering methods are in tui/rendering.rs
}

fn redact_and_bound_parameters(parameters: &serde_json::Value) -> String {
    fn redact(value: &serde_json::Value, key: Option<&str>) -> serde_json::Value {
        let sensitive_key = key.is_some_and(|key| {
            let key = key.to_ascii_lowercase();
            ["token", "secret", "password", "api_key", "authorization"]
                .iter()
                .any(|needle| key.contains(needle))
        });
        if sensitive_key {
            return serde_json::Value::String("[REDACTED]".to_string());
        }
        match value {
            serde_json::Value::Object(map) => serde_json::Value::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), redact(value, Some(key))))
                    .collect(),
            ),
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(|value| redact(value, None)).collect())
            }
            _ => value.clone(),
        }
    }

    const MAX_PARAMETER_PREVIEW_BYTES: usize = 8 * 1024;
    let rendered = serde_json::to_string(&redact(parameters, None))
        .unwrap_or_else(|_| "[unavailable]".to_string());
    if rendered.len() <= MAX_PARAMETER_PREVIEW_BYTES {
        rendered
    } else {
        let mut boundary = MAX_PARAMETER_PREVIEW_BYTES;
        while !rendered.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}… [truncated]", &rendered[..boundary])
    }
}

fn bound_preview(preview: &str) -> String {
    const MAX_PREVIEW_BYTES: usize = 8 * 1024;
    if preview.len() <= MAX_PREVIEW_BYTES {
        return preview.to_string();
    }
    let mut boundary = MAX_PREVIEW_BYTES;
    while !preview.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}… [truncated]", &preview[..boundary])
}

fn history_chat_message(message: &thinclaw_channels::tui::TuiHistoryMessage) -> ChatMessage {
    match message.role.as_str() {
        "user" => ChatMessage::User {
            text: message.content.clone(),
        },
        "assistant" => ChatMessage::Assistant {
            text: message.content.clone(),
            model: message.model.clone(),
        },
        _ => ChatMessage::System {
            text: message.content.clone(),
        },
    }
}

#[cfg(not(test))]
fn input_history_path() -> std::path::PathBuf {
    crate::platform::resolve_data_dir("tui-input-history.json")
}

fn load_input_history() -> Vec<String> {
    #[cfg(test)]
    return Vec::new();

    #[cfg(not(test))]
    {
        const MAX_HISTORY_BYTES: u64 = 1024 * 1024;
        let bytes = match thinclaw_platform::read_regular_file_bounded(
            &input_history_path(),
            MAX_HISTORY_BYTES,
        ) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(error) => {
                tracing::debug!(%error, "Could not load TUI input history");
                return Vec::new();
            }
        };
        serde_json::from_slice::<Vec<String>>(&bytes)
            .unwrap_or_default()
            .into_iter()
            .filter(|value| !value.trim().is_empty() && value.len() <= 64 * 1024)
            .rev()
            .take(1_000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

fn persist_input_history(history: &[String]) -> std::io::Result<()> {
    #[cfg(test)]
    {
        let _ = history;
        Ok(())
    }
    #[cfg(not(test))]
    {
        let path = input_history_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(history)
            .map_err(|error| std::io::Error::other(format!("encode input history: {error}")))?;
        thinclaw_platform::write_private_file_atomic(&path, &bytes, true)
    }
}

fn build_startup_message(bootstrap: &thinclaw_channels::tui::TuiBootstrap) -> String {
    let mut lines = vec![
        format!(
            "{} · {} · {} · capability revision {}",
            bootstrap.agent_name,
            bootstrap.model,
            if bootstrap.history.conversation_id.is_empty() {
                "new conversation"
            } else {
                bootstrap.history.conversation_id.as_str()
            },
            bootstrap.capabilities.revision
        ),
        "Type /help for controls, or send a message to begin.".to_string(),
    ];
    if let Some(origin) = bootstrap.gateway_origin.as_deref() {
        lines.push(String::new());
        lines.push("Access:".to_string());
        lines.push(format!("  Web UI: {origin}"));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use thinclaw_channels::StatusUpdate;

    fn test_bootstrap() -> thinclaw_channels::tui::TuiBootstrap {
        thinclaw_channels::tui::TuiBootstrap {
            principal_id: "default".to_string(),
            actor_id: "default".to_string(),
            agent_id: "main".to_string(),
            agent_name: "Test Agent".to_string(),
            model: "test-model".to_string(),
            provider: "test".to_string(),
            workspace: Some("default".to_string()),
            profile: "server".to_string(),
            gateway_origin: None,
            history: thinclaw_channels::tui::TuiHistoryPage {
                conversation_id: "conversation-1".to_string(),
                messages: Vec::new(),
                before_cursor: None,
                has_more: false,
            },
            capabilities: thinclaw_channels::tui::TuiCapabilitySnapshot {
                revision: 7,
                sealed: true,
                identities: Vec::new(),
            },
        }
    }

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
        assert_eq!(state.display_text(), "Hello");
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
        let mut app = TuiApp::new(tx, update_rx, test_bootstrap());
        app.scroll_offset = 0;

        app.handle_slash_command("/help").await;

        assert_eq!(app.scroll_offset, u16::MAX);
        assert!(matches!(
            app.messages.last(),
            Some(ChatMessage::System { text }) if text.contains("Agent cockpit controls")
        ));
    }

    #[tokio::test]
    async fn test_back_command_never_deletes_transcript_content() {
        let (tx, _rx) = mpsc::channel(4);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut app = TuiApp::new(tx, update_rx, test_bootstrap());
        app.messages.push(ChatMessage::User {
            text: "/context detail".to_string(),
        });
        app.messages.push(ChatMessage::Assistant {
            text: "full context detail".to_string(),
            model: Some("test-model".to_string()),
        });

        app.handle_slash_command("/back").await;

        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.status_text, "No drawer or modal is open");
    }

    #[test]
    fn bootstrap_hydrates_nondefault_identity_model_and_history() {
        let (tx, _rx) = mpsc::channel(4);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut bootstrap = test_bootstrap();
        bootstrap.agent_name = "Researcher".to_string();
        bootstrap.model = "custom/model".to_string();
        bootstrap.history.messages = vec![thinclaw_channels::tui::TuiHistoryMessage {
            id: "m1".to_string(),
            role: "assistant".to_string(),
            content: "persisted".to_string(),
            model: Some("older/model".to_string()),
            created_at: "2026-08-01T00:00:00Z".to_string(),
        }];
        let app = TuiApp::new(tx, update_rx, bootstrap);
        assert_eq!(app.agent_id, "Researcher");
        assert_eq!(app.model, "custom/model");
        assert!(matches!(
            app.messages.first(),
            Some(ChatMessage::Assistant { text, model })
                if text == "persisted" && model.as_deref() == Some("older/model")
        ));
    }

    #[tokio::test]
    async fn history_pages_prepend_in_order_without_duplicates() {
        let (tx, _rx) = mpsc::channel(4);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut bootstrap = test_bootstrap();
        bootstrap.history.messages = vec![thinclaw_channels::tui::TuiHistoryMessage {
            id: "m2".to_string(),
            role: "assistant".to_string(),
            content: "second".to_string(),
            model: Some("old".to_string()),
            created_at: "2026-08-01T00:00:02Z".to_string(),
        }];
        let mut app = TuiApp::new(tx, update_rx, bootstrap);
        app.handle_update(TuiUpdate::HistoryPage(
            thinclaw_channels::tui::TuiHistoryPage {
                conversation_id: "conversation-1".to_string(),
                messages: vec![
                    thinclaw_channels::tui::TuiHistoryMessage {
                        id: "m1".to_string(),
                        role: "user".to_string(),
                        content: "first".to_string(),
                        model: None,
                        created_at: "2026-08-01T00:00:01Z".to_string(),
                    },
                    thinclaw_channels::tui::TuiHistoryMessage {
                        id: "m2".to_string(),
                        role: "assistant".to_string(),
                        content: "duplicate".to_string(),
                        model: Some("wrong".to_string()),
                        created_at: "2026-08-01T00:00:02Z".to_string(),
                    },
                ],
                before_cursor: Some("row:0".to_string()),
                has_more: false,
            },
        ));
        assert_eq!(app.messages.len(), 2);
        assert!(matches!(app.messages[0], ChatMessage::User { ref text } if text == "first"));
        assert!(
            matches!(app.messages[1], ChatMessage::Assistant { ref text, .. } if text == "second")
        );
    }

    #[tokio::test]
    async fn capability_updates_replace_atomically_and_ignore_replays() {
        let (tx, mut rx) = mpsc::channel(4);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut app = TuiApp::new(tx, update_rx, test_bootstrap());
        let identity = thinclaw_channels::tui::TuiCapabilityIdentity {
            name: "dynamic".to_string(),
            origin: "mcp".to_string(),
            source_id: "mcp/test".to_string(),
            revision: 8,
            compiled: true,
            configured: Some(true),
            registered: true,
            dependency: "available".to_string(),
            exposed: true,
            ready: "unknown".to_string(),
            approval: "conditional".to_string(),
            health: "unknown".to_string(),
            reasons: Vec::new(),
        };
        app.handle_update(TuiUpdate::CapabilitySnapshot(
            thinclaw_channels::tui::TuiCapabilitySnapshot {
                revision: 8,
                sealed: true,
                identities: vec![identity],
            },
        ));
        app.handle_update(TuiUpdate::CapabilitySnapshot(
            thinclaw_channels::tui::TuiCapabilitySnapshot {
                revision: 7,
                sealed: true,
                identities: Vec::new(),
            },
        ));
        assert_eq!(app.capabilities.revision, 8);
        assert_eq!(app.capabilities.identities.len(), 1);
        app.handle_slash_command("/tools dynamic").await;
        assert!(rx.try_recv().is_err(), "local /tools must not be forwarded");
        assert!(matches!(
            app.messages.last(),
            Some(ChatMessage::System { text }) if text.contains("mcp/test") && text.contains("revision 8")
        ));
    }

    #[tokio::test]
    async fn bang_input_is_rejected_without_forwarding() {
        let (tx, mut rx) = mpsc::channel(4);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut app = TuiApp::new(tx, update_rx, test_bootstrap());

        app.handle_submit("!whoami").await;

        assert!(matches!(
            app.messages.last(),
            Some(ChatMessage::Warning { text })
                if text.contains("Raw shell escape was removed")
        ));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn approval_is_id_bound_and_unrelated_text_does_not_dismiss_it() {
        let (tx, mut rx) = mpsc::channel(8);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut app = TuiApp::new(tx, update_rx, test_bootstrap());
        app.handle_update(TuiUpdate::ApprovalNeeded {
            request_id: "request-123".to_string(),
            tool_name: "shell".to_string(),
            description: "run a command".to_string(),
            parameters: serde_json::json!({
                "command": "pwd",
                "api_token": "top-secret"
            }),
        });

        assert_eq!(app.pending_approvals.len(), 1);
        assert!(
            app.pending_approvals[0]
                .redacted_parameters
                .contains("[REDACTED]")
        );
        assert!(
            !app.pending_approvals[0]
                .redacted_parameters
                .contains("top-secret")
        );

        app.handle_submit("please wait").await;
        assert_eq!(app.pending_approvals.len(), 1);
        assert!(matches!(
            rx.recv().await,
            Some(TuiEvent::UserMessage(text)) if text == "please wait"
        ));

        app.handle_submit("always").await;
        assert!(app.pending_approvals.is_empty());
        assert!(matches!(
            rx.recv().await,
            Some(TuiEvent::ApprovalResponse {
                request_id,
                decision: TuiApprovalDecision::ApproveForSession,
            }) if request_id == "request-123"
        ));
    }

    #[test]
    fn interleaved_tool_events_update_their_matching_cards() {
        let (tx, _rx) = mpsc::channel(8);
        let (_update_tx, update_rx) = mpsc::channel(4);
        let mut app = TuiApp::new(tx, update_rx, test_bootstrap());
        let first = thinclaw_types::ToolInvocationId::from_provider("first");
        let second = thinclaw_types::ToolInvocationId::from_provider("second");

        for (id, name) in [(first.clone(), "one"), (second.clone(), "two")] {
            app.handle_update(TuiUpdate::ToolStarted {
                invocation_id: id,
                name: name.to_string(),
                parameters: Some(serde_json::json!({"value": name})),
            });
        }
        app.handle_update(TuiUpdate::ToolOutput {
            invocation_id: first.clone(),
            name: "one".to_string(),
            preview: "first output".to_string(),
            artifacts: Vec::new(),
        });
        app.handle_update(TuiUpdate::ToolCompleted {
            invocation_id: second.clone(),
            name: "two".to_string(),
            success: false,
            result_preview: Some("second failed".to_string()),
            duration_ms: Some(20),
        });
        app.handle_update(TuiUpdate::ToolCompleted {
            invocation_id: first.clone(),
            name: "one".to_string(),
            success: true,
            result_preview: None,
            duration_ms: Some(10),
        });

        let first_index = app.tool_activity[&first];
        let second_index = app.tool_activity[&second];
        assert!(matches!(
            &app.messages[first_index],
            ChatMessage::ToolCall { result: Some(result), is_error: false, completed: true, .. }
                if result == "first output"
        ));
        assert!(matches!(
            &app.messages[second_index],
            ChatMessage::ToolCall { result: Some(result), is_error: true, completed: true, .. }
                if result == "second failed"
        ));
    }

    #[test]
    fn startup_message_uses_only_the_credential_free_bootstrap_origin() {
        let mut bootstrap = test_bootstrap();
        bootstrap.gateway_origin = credential_free_gateway_url(
            "http://operator:secret@127.0.0.1:3100/?token=runtime-token#fragment",
        );
        let rendered = build_startup_message(&bootstrap);
        assert!(rendered.contains("Web UI: http://127.0.0.1:3100/"));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("runtime-token"));
    }

    #[test]
    fn transcript_rendering_strips_ansi_osc_and_c1_controls() {
        let input = Text::from(Line::from(Span::raw(
            "safe\u{1b}]0;owned\u{7}\u{1b}[31mred\u{1b}[0m\u{009b}tail",
        )));
        let rendered = rendering::sanitize_terminal_text(input);
        let plain = rendered
            .lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(!plain.chars().any(|character| character.is_control()));
        assert!(!plain.contains('\u{1b}'));
        assert!(plain.contains("safe"));
        assert!(plain.contains("red"));
        assert!(plain.contains("tail"));
    }
}
