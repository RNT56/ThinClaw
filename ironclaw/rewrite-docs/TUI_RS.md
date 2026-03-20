> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# TUI: Interactive Terminal Chat Interface

ThinClaw includes a rich terminal-based chat UI built with `ratatui`, providing the same chat experience as the Tauri desktop app — but inside a terminal. This is essential for SSH sessions, headless servers, tmux workflows, and users who prefer terminal-native tools.

---

## 1. What OpenClaw's TUI Does Today

OpenClaw's TUI (`src/tui/`, 31 files, ~24KB core) is a full-screen terminal application using `@mariozechner/pi-tui`:

- **Full-screen layout:** Chat history panel (scrollable), input editor at bottom, status bar
- **Real-time streaming:** Token-by-token display of LLM responses via WebSocket
- **Thinking/reasoning display:** Shows thinking blocks toggled with `/think` levels
- **Slash commands:** `/help`, `/model`, `/agent`, `/session`, `/new`, `/abort`, `/settings`, etc.
- **Autocomplete:** Tab-completion for slash commands and their arguments
- **Overlay selectors:** Interactive popup selectors for model picker, agent picker, session picker
- **Local shell:** `!ls` prefix runs commands locally on the user's machine (with a confirmation prompt)
- **Tool call display:** Shows tool names, arguments, and results inline as the agent works
- **Session management:** Switch between sessions, load history, create new sessions
- **RTL text support:** BiDi isolation for Arabic/Hebrew text
- **OSC8 hyperlinks:** Clickable terminal links in supported terminals
- **Connection status:** Shows gateway connectivity, activity status, reconnection state
- **Ctrl+C handling:** Double-tap to exit, single tap to clear input or abort active run

---

## 2. TUI Architecture

### Layout

```
┌──────────────────────────────────────────┐
│  ThinClaw  │  claude-3-5-sonnet │ Agent: main  │
├──────────────────────────────────────────┤
│                                          │
│  [user] What files are on my Desktop?    │
│                                          │
│  [assistant] Let me check that for you.  │
│  ┌─ Tool: bash ─────────────────┐        │
│  │ $ ls ~/Desktop               │        │
│  │ report.pdf  notes.txt        │        │
│  └──────────────────────────────┘        │
│  I found 2 files on your Desktop:        │
│  - report.pdf                            │
│  - notes.txt                             │
│                                          │
├──────────────────────────────────────────┤
│ > Type a message... (/help for commands) │
├──────────────────────────────────────────┤
│  Connected │ claude-3-5-sonnet │ 2.4k tokens │
└──────────────────────────────────────────┘
```

### Three Rendering Zones

| Zone | Widget | Content |
|---|---|---|
| **Header** | Title bar | App name, model, agent ID, session key |
| **Chat area** | Scrollable `Paragraph` | Message history + tool calls + streaming tokens |
| **Input area** | Editable text input | User's message being composed |
| **Footer** | Status bar | Connection status, model, token usage, thinking mode |

---

## 3. Rust Implementation

### Core TUI Struct

```rust
use ratatui::{prelude::*, widgets::*};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use tokio::sync::mpsc;

pub struct ThinClawTui {
    /// Connection to the Orchestrator
    client: OrchestratorClient,
    /// Chat message history for rendering
    messages: Vec<ChatMessage>,
    /// Current input text
    input: String,
    /// Input cursor position
    cursor_pos: usize,
    /// Input history (up/down arrows)
    input_history: Vec<String>,
    input_history_idx: Option<usize>,
    /// Scroll offset for chat area
    scroll_offset: u16,
    /// Active model display
    model: String,
    /// Active agent
    agent_id: String,
    /// Session key
    session_key: String,
    /// Connection status
    connection_status: ConnectionStatus,
    /// Token usage
    token_usage: Option<TokenUsage>,
    /// Currently streaming response
    active_stream: Option<StreamState>,
    /// Active overlay (model picker, help, etc.)
    overlay: Option<OverlayKind>,
    /// Stream assembler for delta→display conversion
    stream_assembler: StreamAssembler,
    /// Whether to show thinking blocks
    show_thinking: bool,
    /// Local shell execution state
    local_shell_allowed: bool,
    /// Ctrl+C double-tap tracking
    last_ctrl_c: Option<std::time::Instant>,
}

pub enum ChatMessage {
    User { text: String },
    Assistant { text: String, run_id: String, model: Option<String> },
    System { text: String },
    ToolCall {
        id: String,
        name: String,
        args: String,
        result: Option<String>,
        is_error: bool,
    },
}

pub enum ConnectionStatus {
    Connected,
    Connecting,
    Disconnected { reason: String },
}

pub enum OverlayKind {
    ModelSelector(SelectorState),
    AgentSelector(SelectorState),
    SessionSelector(SelectorState),
    Settings(SettingsState),
    Help,
    LocalShellConfirm,
}
```

### Main Event Loop

```rust
impl ThinClawTui {
    pub async fn run(&mut self, terminal: &mut Terminal<impl Backend>) -> Result<()> {
        // Channel for Orchestrator events (streaming tokens, tool calls, etc.)
        let (event_tx, mut event_rx) = mpsc::channel::<OrchestratorEvent>(256);
        self.client.subscribe(event_tx).await?;

        loop {
            // Render current state
            terminal.draw(|frame| self.render(frame))?;

            // Handle events (keyboard + orchestrator)
            tokio::select! {
                // Keyboard input (with 50ms poll for smooth streaming)
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if event::poll(Duration::ZERO)? {
                        if let Event::Key(key) = event::read()? {
                            match self.handle_key(key).await? {
                                KeyAction::Continue => {},
                                KeyAction::Exit => return Ok(()),
                            }
                        }
                    }
                },
                // Events from Orchestrator (streaming tokens, tool calls)
                Some(event) = event_rx.recv() => {
                    self.handle_orchestrator_event(event).await?;
                },
            }
        }
    }

    async fn handle_key(&mut self, key: event::KeyEvent) -> Result<KeyAction> {
        // If an overlay is active, route keys there
        if self.overlay.is_some() {
            return self.handle_overlay_key(key).await;
        }

        match (key.modifiers, key.code) {
            // Ctrl+C: abort active run, or double-tap to exit
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                if let Some(stream) = &self.active_stream {
                    self.client.abort(&stream.run_id).await?;
                    self.active_stream = None;
                } else if self.last_ctrl_c.map_or(false, |t| t.elapsed() < Duration::from_millis(1000)) {
                    return Ok(KeyAction::Exit);
                } else {
                    self.last_ctrl_c = Some(std::time::Instant::now());
                    self.input.clear();
                    self.cursor_pos = 0;
                }
            },
            // Enter: submit message or command
            (_, KeyCode::Enter) => {
                if !self.input.is_empty() {
                    self.submit().await?;
                }
            },
            // Up/Down: input history
            (_, KeyCode::Up) => self.history_prev(),
            (_, KeyCode::Down) => self.history_next(),
            // Page Up/Down: scroll chat
            (_, KeyCode::PageUp) => self.scroll_up(10),
            (_, KeyCode::PageDown) => self.scroll_down(10),
            // Character input
            (_, KeyCode::Char(c)) => {
                self.input.insert(self.cursor_pos, c);
                self.cursor_pos += 1;
            },
            // Backspace
            (_, KeyCode::Backspace) => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                    self.input.remove(self.cursor_pos);
                }
            },
            _ => {},
        }
        Ok(KeyAction::Continue)
    }
}
```

### Rendering

```rust
impl ThinClawTui {
    fn render(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),    // Header
                Constraint::Min(5),      // Chat area
                Constraint::Length(3),    // Input
                Constraint::Length(1),    // Footer/status
            ])
            .split(frame.area());

        // Header
        let header = Paragraph::new(Line::from(vec![
            Span::styled(" ThinClaw ", Style::default().fg(Color::White).bg(Color::Blue).bold()),
            Span::raw(" │ "),
            Span::styled(&self.model, Style::default().fg(Color::Cyan)),
            Span::raw(" │ Agent: "),
            Span::styled(&self.agent_id, Style::default().fg(Color::Yellow)),
        ]));
        frame.render_widget(header, chunks[0]);

        // Chat messages
        let chat_text = self.render_messages();
        let chat = Paragraph::new(chat_text)
            .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset, 0));
        frame.render_widget(chat, chunks[1]);

        // Input
        let input = Paragraph::new(self.input.as_str())
            .block(Block::default()
                .borders(Borders::ALL)
                .title(" Message (/help for commands) "));
        frame.render_widget(input, chunks[2]);

        // Footer/status
        let status_line = self.render_status_line();
        frame.render_widget(Paragraph::new(status_line), chunks[3]);

        // Cursor position in input
        frame.set_cursor_position((
            chunks[2].x + self.cursor_pos as u16 + 1,
            chunks[2].y + 1,
        ));

        // Overlay (if active)
        if let Some(overlay) = &self.overlay {
            self.render_overlay(frame, overlay);
        }
    }

    fn render_messages(&self) -> Text<'_> {
        let mut lines = Vec::new();
        for msg in &self.messages {
            match msg {
                ChatMessage::User { text } => {
                    lines.push(Line::from(vec![
                        Span::styled("You: ", Style::default().fg(Color::Green).bold()),
                        Span::raw(text),
                    ]));
                },
                ChatMessage::Assistant { text, model, .. } => {
                    let label = model.as_deref().unwrap_or("AI");
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("{}: ", label),
                            Style::default().fg(Color::Cyan).bold()
                        ),
                        Span::raw(text),
                    ]));
                },
                ChatMessage::ToolCall { name, args, result, is_error, .. } => {
                    lines.push(Line::from(Span::styled(
                        format!("  ┌─ Tool: {} ─", name),
                        Style::default().fg(Color::DarkGray),
                    )));
                    if !args.is_empty() {
                        lines.push(Line::from(Span::styled(
                            format!("  │ {}", args),
                            Style::default().fg(Color::DarkGray),
                        )));
                    }
                    if let Some(result) = result {
                        let color = if *is_error { Color::Red } else { Color::DarkGray };
                        for line in result.lines().take(10) {
                            lines.push(Line::from(Span::styled(
                                format!("  │ {}", line), Style::default().fg(color),
                            )));
                        }
                    }
                    lines.push(Line::from(Span::styled(
                        "  └──────────────",
                        Style::default().fg(Color::DarkGray),
                    )));
                },
                ChatMessage::System { text } => {
                    lines.push(Line::from(Span::styled(
                        text, Style::default().fg(Color::Yellow).italic(),
                    )));
                },
            }
            lines.push(Line::from("")); // Spacing between messages
        }

        // Active streaming
        if let Some(stream) = &self.active_stream {
            lines.push(Line::from(vec![
                Span::styled("AI: ", Style::default().fg(Color::Cyan).bold()),
                Span::raw(&stream.display_text),
                Span::styled("▊", Style::default().fg(Color::Cyan)), // Blinking cursor
            ]));
        }

        Text::from(lines)
    }
}
```

---

## 4. Slash Commands

The TUI intercepts lines starting with `/` as commands:

| Command | Action |
|---|---|
| `/help` | Show available commands |
| `/status` | Gateway status summary |
| `/model <name>` | Set active model (or open picker if no arg) |
| `/models` | Open model selector overlay |
| `/agent <id>` | Switch agent (or open picker) |
| `/agents` | Open agent selector overlay |
| `/session <key>` | Switch session (or open picker) |
| `/sessions` | Open session selector overlay |
| `/think <level>` | Set thinking level (off / low / medium / high) |
| `/verbose on\|off` | Toggle verbose tool output |
| `/reasoning on\|off` | Toggle extended reasoning display |
| `/usage off\|tokens\|full` | Token usage footer display |
| `/elevated on\|off\|ask\|full` | Elevated permissions mode |
| `/activation mention\|always` | Group chat activation mode |
| `/new` or `/reset` | Start a new session |
| `/abort` | Abort the active agent run |
| `/settings` | Open settings overlay |
| `/exit` or `/quit` | Exit the TUI |

Commands can also include gateway-registered chat commands (see `CHAT_COMMANDS_RS.md`).

---

## 5. Streaming Token Assembly

The `StreamAssembler` handles progressive token display — the same concept as OpenClaw's `TuiStreamAssembler`:

```rust
pub struct StreamAssembler {
    runs: HashMap<String, RunStreamState>,
}

pub struct RunStreamState {
    thinking_text: String,
    content_text: String,
    display_text: String,
}

impl StreamAssembler {
    /// Process a streaming delta and return updated display text
    pub fn ingest_delta(&mut self, run_id: &str, delta: &StreamDelta) -> Option<String> {
        let state = self.runs.entry(run_id.to_string()).or_default();

        if let Some(thinking) = &delta.thinking {
            state.thinking_text.push_str(thinking);
        }
        if let Some(content) = &delta.content {
            state.content_text.push_str(content);
        }

        state.display_text = if state.thinking_text.is_empty() {
            state.content_text.clone()
        } else {
            format!("💭 {}\n\n{}", state.thinking_text, state.content_text)
        };

        Some(state.display_text.clone())
    }

    /// Finalize a completed run
    pub fn finalize(&mut self, run_id: &str, final_message: &str) -> String {
        self.runs.remove(run_id);
        final_message.to_string()
    }
}
```

---

## 6. Local Shell Execution

Lines prefixed with `!` execute on the local machine (not via the agent):

```rust
impl ThinClawTui {
    async fn handle_bang_line(&mut self, line: &str) -> Result<()> {
        let cmd = &line[1..]; // Strip the `!`
        if cmd.is_empty() { return Ok(()); }

        // First-time confirmation
        if !self.local_shell_allowed {
            self.overlay = Some(OverlayKind::LocalShellConfirm);
            return Ok(());
        }

        self.messages.push(ChatMessage::System {
            text: format!("[local] $ {}", cmd),
        });

        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{}{}", stdout, stderr);

        for line in combined.lines().take(100) {
            self.messages.push(ChatMessage::System {
                text: format!("[local] {}", line),
            });
        }

        self.messages.push(ChatMessage::System {
            text: format!("[local] exit {}", output.status.code().unwrap_or(-1)),
        });

        Ok(())
    }
}
```

---

## 7. Overlay System

Overlays are popup components rendered on top of the chat area:

```rust
impl ThinClawTui {
    fn render_overlay(&self, frame: &mut Frame, overlay: &OverlayKind) {
        // Center a floating box
        let area = centered_rect(60, 60, frame.area());
        frame.render_widget(Clear, area); // Clear background

        match overlay {
            OverlayKind::ModelSelector(state) => {
                let items: Vec<ListItem> = state.filtered_items.iter()
                    .map(|m| ListItem::new(format!("  {} ({})", m.name, m.provider)))
                    .collect();
                let list = List::new(items)
                    .block(Block::default().title(" Select Model ").borders(Borders::ALL))
                    .highlight_style(Style::default().bg(Color::Blue).fg(Color::White));
                frame.render_stateful_widget(list, area, &mut state.list_state.clone());
            },
            OverlayKind::Help => {
                let help = help_text();
                let p = Paragraph::new(help)
                    .block(Block::default().title(" Help ").borders(Borders::ALL))
                    .wrap(Wrap { trim: false });
                frame.render_widget(p, area);
            },
            _ => {},
        }
    }
}
```

---

## 8. Crate Dependencies

```toml
[dependencies]
ratatui = "0.29"        # Terminal UI framework
crossterm = "0.28"      # Terminal input/output (cross-platform)
tui-textarea = "0.7"    # Multi-line text input widget
unicode-width = "0.2"   # Proper character width handling
textwrap = "0.16"       # Text wrapping for chat messages
```

---

## 9. Feature Gate

```toml
[features]
default = ["desktop-ui", "cli", "tui"]
tui = ["ratatui", "crossterm", "tui-textarea"]
```

When compiled without `tui`, the binary size shrinks and the `thinclaw tui` command prints an error: `"TUI not compiled. Rebuild with --features tui"`.
