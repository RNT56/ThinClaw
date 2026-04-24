//! Interactive REPL channel with line editing and markdown rendering.
//!
//! Provides the primary CLI interface for interacting with the agent.
//! Uses rustyline for line editing, history, and tab-completion.
//! Uses termimad for rendering markdown responses inline.
//!
//! ## Commands
//!
//! - `/help` - Show available commands
//! - `/quit` or `/exit` - Exit the REPL
//! - `/debug` - Toggle debug mode (verbose tool output)
//! - `/undo` - Undo the last turn
//! - `/redo` - Redo an undone turn
//! - `/clear` - Clear the conversation
//! - `/compress` - Compact the context (`/compact` alias)
//! - `/new` or `/thread new` - Start a new thread
//! - `/rollback` - Filesystem rollback command family
//! - `/identity` - Show the active identity stack
//! - `/memory` - Show memory and learning surfaces
//! - `/personality` - Set, show, or clear a temporary session personality (`/vibe` alias)
//! - `/skin` - Runtime CLI skin command family
//! - `yes`/`no`/`always` - Respond to tool approval prompts
//! - `Esc` - Interrupt current operation

use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use rustyline::completion::Completer;
use rustyline::config::Config;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{
    Cmd as ReadlineCmd, CompletionType, ConditionalEventHandler, Editor, Event, EventContext,
    EventHandler, Helper, KeyCode, KeyEvent, Modifiers, RepeatCount,
};
use termimad::MadSkin;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::agent::truncate_for_preview;
use crate::channels::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};
use crate::error::ChannelError;
use crate::terminal_branding::{TerminalBranding, resolve_cli_skin_name};
use crate::tui::skin::CliSkin;

/// Max characters for tool result previews in the terminal.
const CLI_TOOL_RESULT_MAX: usize = 200;

/// Max characters for thinking/status messages in the terminal.
const CLI_STATUS_MAX: usize = 200;

/// Slash commands available in the REPL.
const SLASH_COMMANDS: &[&str] = &[
    "/help",
    "/quit",
    "/exit",
    "/debug",
    "/model",
    "/undo",
    "/redo",
    "/clear",
    "/compress",
    "/compact",
    "/new",
    "/interrupt",
    "/version",
    "/tools",
    "/ping",
    "/context",
    "/job",
    "/status",
    "/cancel",
    "/list",
    "/identity",
    "/memory",
    "/skills",
    "/heartbeat",
    "/summarize",
    "/suggest",
    "/thread",
    "/resume",
    "/rollback",
    "/personality",
    "/vibe",
    "/skin",
];

/// Rustyline helper for slash-command tab completion.
struct ReplHelper;

impl Completer for ReplHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        if !line.starts_with('/') {
            return Ok((0, vec![]));
        }

        let prefix = &line[..pos];
        let matches: Vec<String> = SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.starts_with(prefix))
            .map(|cmd| cmd.to_string())
            .collect();

        Ok((0, matches))
    }
}

impl Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        if !line.starts_with('/') || pos < line.len() {
            return None;
        }

        SLASH_COMMANDS
            .iter()
            .find(|cmd| cmd.starts_with(line) && **cmd != line)
            .map(|cmd| cmd[line.len()..].to_string())
    }
}

impl Highlighter for ReplHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(format!("\x1b[90m{hint}\x1b[0m"))
    }
}

impl Validator for ReplHelper {
    fn validate(
        &self,
        ctx: &mut rustyline::validate::ValidationContext,
    ) -> rustyline::Result<rustyline::validate::ValidationResult> {
        let input = ctx.input();
        // Backslash continuation: if the line ends with '\', request more input.
        if input.ends_with('\\') {
            return Ok(rustyline::validate::ValidationResult::Incomplete);
        }
        // Triple-backtick fencing: if odd number of ```, request more input.
        if input.matches("```").count() % 2 != 0 {
            return Ok(rustyline::validate::ValidationResult::Incomplete);
        }
        Ok(rustyline::validate::ValidationResult::Valid(None))
    }
}
impl Helper for ReplHelper {}

struct EscInterruptHandler {
    triggered: Arc<AtomicBool>,
}

impl ConditionalEventHandler for EscInterruptHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<ReadlineCmd> {
        self.triggered.store(true, Ordering::Relaxed);
        Some(ReadlineCmd::Interrupt)
    }
}

/// Build a termimad skin with our color scheme.
fn make_skin(skin: &CliSkin) -> MadSkin {
    skin.to_termimad_skin()
}

/// Format JSON params as `key: value` lines for the approval card.
fn format_json_params(
    branding: &TerminalBranding,
    params: &serde_json::Value,
    indent: &str,
) -> String {
    match params {
        serde_json::Value::Object(map) => {
            let mut lines = Vec::new();
            for (key, value) in map {
                let val_str = match value {
                    serde_json::Value::String(s) => {
                        let display = if s.len() > 120 {
                            let end = s
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i < 120)
                                .last()
                                .unwrap_or(0);
                            &s[..end]
                        } else {
                            s
                        };
                        branding.good(format!("\"{display}\""))
                    }
                    other => {
                        let rendered = other.to_string();
                        if rendered.len() > 120 {
                            let end = rendered
                                .char_indices()
                                .map(|(i, _)| i)
                                .take_while(|&i| i < 120)
                                .last()
                                .unwrap_or(0);
                            format!("{}...", &rendered[..end])
                        } else {
                            rendered
                        }
                    }
                };
                lines.push(format!("{indent}{}: {val_str}", branding.accent(key)));
            }
            lines.join("\n")
        }
        other => {
            let pretty = serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string());
            let truncated = if pretty.len() > 300 {
                let end = pretty
                    .char_indices()
                    .map(|(i, _)| i)
                    .take_while(|&i| i < 300)
                    .last()
                    .unwrap_or(0);
                format!("{}...", &pretty[..end])
            } else {
                pretty
            };
            truncated
                .lines()
                .map(|l| format!("{indent}{}", branding.muted(l)))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

/// REPL channel with line editing and markdown rendering.
pub struct ReplChannel {
    /// Optional single message to send (for -m flag).
    single_message: Option<String>,
    /// Debug mode flag (shared with input thread).
    debug_mode: Arc<AtomicBool>,
    /// Active skin shared with the reader thread and responders.
    skin: Arc<RwLock<CliSkin>>,
    /// Default skin name used when the local client resets the skin.
    default_skin_name: String,
    /// Whether we're currently streaming (chunks have been printed without a trailing newline).
    is_streaming: Arc<AtomicBool>,
    /// When true, the one-liner startup banner is suppressed (boot screen shown instead).
    suppress_banner: Arc<AtomicBool>,
}

impl ReplChannel {
    /// Create a new REPL channel.
    pub fn new() -> Self {
        let default_skin_name = resolve_cli_skin_name();
        Self {
            single_message: None,
            debug_mode: Arc::new(AtomicBool::new(false)),
            skin: Arc::new(RwLock::new(CliSkin::load(&default_skin_name))),
            default_skin_name,
            is_streaming: Arc::new(AtomicBool::new(false)),
            suppress_banner: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Create a REPL channel that sends a single message and exits.
    pub fn with_message(message: String) -> Self {
        let default_skin_name = resolve_cli_skin_name();
        Self {
            single_message: Some(message),
            debug_mode: Arc::new(AtomicBool::new(false)),
            skin: Arc::new(RwLock::new(CliSkin::load(&default_skin_name))),
            default_skin_name,
            is_streaming: Arc::new(AtomicBool::new(false)),
            suppress_banner: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Suppress the one-liner startup banner (boot screen will be shown instead).
    pub fn suppress_banner(&self) {
        self.suppress_banner.store(true, Ordering::Relaxed);
    }

    fn is_debug(&self) -> bool {
        self.debug_mode.load(Ordering::Relaxed)
    }

    fn current_skin(&self) -> CliSkin {
        self.skin
            .read()
            .map(|skin| skin.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }
}

impl Default for ReplChannel {
    fn default() -> Self {
        Self::new()
    }
}

fn print_help(skin: &CliSkin) {
    let branding = TerminalBranding::from_skin(skin.clone());

    branding.print_banner(
        "Agent REPL",
        Some("Interactive shell with shared identity, memory, and skin-aware controls."),
    );
    println!("  {}", branding.body_bold("Commands"));
    println!(
        "  {} {}",
        branding.accent("/help"),
        branding.muted("show this help")
    );
    println!(
        "  {} {}",
        branding.accent("/debug"),
        branding.muted("toggle verbose output")
    );
    println!(
        "  {} {}",
        branding.accent("/quit /exit"),
        branding.muted("exit the repl")
    );
    println!();
    println!("  {}", branding.body_bold("Conversation"));
    println!(
        "  {} {}",
        branding.accent("/undo"),
        branding.muted("undo the last turn")
    );
    println!(
        "  {} {}",
        branding.accent("/redo"),
        branding.muted("redo an undone turn")
    );
    println!(
        "  {} {}",
        branding.accent("/clear"),
        branding.muted("clear conversation")
    );
    println!(
        "  {} {}",
        branding.accent("/compress"),
        branding.muted("compact context window (/compact alias)")
    );
    println!(
        "  {} {}",
        branding.accent("/new"),
        branding.muted("new conversation thread")
    );
    println!(
        "  {} {}",
        branding.accent("/thread new"),
        branding.muted("new conversation thread")
    );
    println!(
        "  {} {}",
        branding.accent("/thread <id>"),
        branding.muted("switch to an existing thread")
    );
    println!(
        "  {} {}",
        branding.accent("/resume <id>"),
        branding.muted("restore a saved checkpoint into the current thread")
    );
    println!(
        "  {} {}",
        branding.accent("/interrupt"),
        branding.muted("stop current operation")
    );
    println!(
        "  {} {}",
        branding.accent("esc"),
        branding.muted("stop current operation")
    );
    println!(
        "  {} {}",
        branding.accent("/rollback"),
        branding.muted("filesystem rollback command family")
    );
    println!(
        "  {} {}",
        branding.accent("/identity"),
        branding.muted("show the active agent name, base pack, skin, and session overlay")
    );
    println!(
        "  {} {}",
        branding.accent("/memory"),
        branding.muted("show memory, recall, learning, and continuity surfaces")
    );
    println!(
        "  {} {}",
        branding.accent("/heartbeat"),
        branding.muted("run the live heartbeat check")
    );
    println!(
        "  {} {}",
        branding.accent("/skills"),
        branding.muted("list installed skills or search the registry")
    );
    println!(
        "  {} {}",
        branding.accent("/personality"),
        branding.muted("set, show, or clear a temporary session personality (/vibe alias)")
    );
    println!(
        "  {} {}",
        branding.accent("/skin [name]"),
        branding.muted("switch the local CLI skin or show the current skin")
    );
    println!();
    println!("  {}", branding.body_bold("Approval responses"));
    println!(
        "  {} {}",
        branding.good("yes (y)"),
        branding.muted("approve tool execution")
    );
    println!(
        "  {} {}",
        branding.bad("no (n)"),
        branding.muted("deny tool execution")
    );
    println!(
        "  {} {}",
        branding.warn("always (a)"),
        branding.muted("approve for this session")
    );
    println!();
}

/// Get the history file path (~/.thinclaw/history).
fn history_path() -> std::path::PathBuf {
    crate::platform::resolve_data_dir("history")
}

#[async_trait]
impl Channel for ReplChannel {
    fn name(&self) -> &str {
        "repl"
    }

    fn formatting_hints(&self) -> Option<String> {
        None
    }

    async fn start(&self) -> Result<MessageStream, ChannelError> {
        let (tx, rx) = mpsc::channel(32);
        let single_message = self.single_message.clone();
        let debug_mode = Arc::clone(&self.debug_mode);
        let skin = Arc::clone(&self.skin);
        let default_skin_name = self.default_skin_name.clone();
        let suppress_banner = Arc::clone(&self.suppress_banner);
        let esc_interrupt_triggered_for_thread = Arc::new(AtomicBool::new(false));

        std::thread::spawn(move || {
            // Single message mode: send it and return
            if let Some(msg) = single_message {
                let incoming = IncomingMessage::new("repl", "default", &msg);
                let _ = tx.blocking_send(incoming);
                return;
            }

            // Set up rustyline
            let config = Config::builder()
                .history_ignore_dups(true)
                .expect("valid config")
                .auto_add_history(true)
                .completion_type(CompletionType::List)
                .build();

            let mut rl = match Editor::with_config(config) {
                Ok(editor) => editor,
                Err(e) => {
                    eprintln!("Failed to initialize line editor: {e}");
                    return;
                }
            };

            rl.set_helper(Some(ReplHelper));

            rl.bind_sequence(
                KeyEvent(KeyCode::Esc, Modifiers::NONE),
                EventHandler::Conditional(Box::new(EscInterruptHandler {
                    triggered: Arc::clone(&esc_interrupt_triggered_for_thread),
                })),
            );

            // Load history
            let hist_path = history_path();
            if let Some(parent) = hist_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = rl.load_history(&hist_path);

            if !suppress_banner.load(Ordering::Relaxed) {
                let current_skin = skin
                    .read()
                    .map(|skin| skin.clone())
                    .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
                TerminalBranding::from_skin(current_skin)
                    .print_banner("ThinClaw REPL", Some("/help for commands, /quit to exit"));
            }

            loop {
                let current_skin = skin
                    .read()
                    .map(|skin| skin.clone())
                    .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
                let prompt = if debug_mode.load(Ordering::Relaxed) {
                    format!(
                        "{}[debug]{} {}{}{} ",
                        current_skin.ansi_fg(current_skin.warn),
                        current_skin.ansi_reset(),
                        current_skin.ansi_fg(current_skin.accent),
                        current_skin.prompt_symbol(),
                        current_skin.ansi_reset()
                    )
                } else {
                    format!(
                        "{}{}{} ",
                        current_skin.ansi_fg(current_skin.accent),
                        current_skin.prompt_symbol(),
                        current_skin.ansi_reset()
                    )
                };

                match rl.readline(&prompt) {
                    Ok(line) => {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }

                        // Handle local REPL commands (only commands that need
                        // immediate local handling stay here)
                        match line.to_lowercase().as_str() {
                            "/quit" | "/exit" => {
                                // Forward shutdown command so the agent loop exits even
                                // when other channels (e.g. web gateway) are still active.
                                let msg = IncomingMessage::new("repl", "default", "/quit");
                                let _ = tx.blocking_send(msg);
                                break;
                            }
                            "/help" => {
                                print_help(&current_skin);
                                continue;
                            }
                            "/debug" => {
                                let current = debug_mode.load(Ordering::Relaxed);
                                debug_mode.store(!current, Ordering::Relaxed);
                                let branding = TerminalBranding::from_skin(current_skin.clone());
                                if !current {
                                    println!("{}", branding.muted("debug mode on"));
                                } else {
                                    println!("{}", branding.muted("debug mode off"));
                                }
                                continue;
                            }
                            _ if line.starts_with("/skin") => {
                                let arg = line.strip_prefix("/skin").map(str::trim).unwrap_or("");
                                let mut guard = match skin.write() {
                                    Ok(guard) => guard,
                                    Err(poisoned) => poisoned.into_inner(),
                                };
                                let branding = TerminalBranding::from_skin(guard.clone());
                                if arg.is_empty() || arg.eq_ignore_ascii_case("current") {
                                    println!(
                                        "{}",
                                        branding.muted(format!("Current skin: {}", guard.name))
                                    );
                                    println!(
                                        "{}",
                                        branding.muted(format!(
                                            "Available skins: {}",
                                            CliSkin::available_names().join(", ")
                                        ))
                                    );
                                } else if arg.eq_ignore_ascii_case("list") {
                                    println!(
                                        "{}",
                                        branding.muted(format!(
                                            "Available skins: {}",
                                            CliSkin::available_names().join(", ")
                                        ))
                                    );
                                } else {
                                    let requested = if arg.eq_ignore_ascii_case("reset") {
                                        default_skin_name.as_str()
                                    } else {
                                        arg
                                    };
                                    *guard = CliSkin::load(requested);
                                    let branding = TerminalBranding::from_skin(guard.clone());
                                    println!(
                                        "{}",
                                        branding.muted(format!(
                                            "Skin switched to '{}' (prompt: {})",
                                            guard.name,
                                            guard.prompt_symbol()
                                        ))
                                    );
                                }
                                continue;
                            }
                            _ => {}
                        }

                        let msg = IncomingMessage::new("repl", "default", line);
                        if tx.blocking_send(msg).is_err() {
                            break;
                        }
                    }
                    Err(ReadlineError::Interrupted) => {
                        if esc_interrupt_triggered_for_thread.swap(false, Ordering::Relaxed) {
                            // Esc: interrupt current operation and keep REPL open.
                            let msg = IncomingMessage::new("repl", "default", "/interrupt");
                            if tx.blocking_send(msg).is_err() {
                                break;
                            }
                        } else {
                            // Ctrl+C (VINTR): request graceful shutdown.
                            let msg = IncomingMessage::new("repl", "default", "/quit");
                            let _ = tx.blocking_send(msg);
                            break;
                        }
                    }
                    Err(ReadlineError::Eof) => {
                        // Ctrl+D: send /quit so the agent loop runs graceful shutdown
                        let msg = IncomingMessage::new("repl", "default", "/quit");
                        let _ = tx.blocking_send(msg);
                        break;
                    }
                    Err(e) => {
                        eprintln!("Input error: {e}");
                        break;
                    }
                }
            }

            // Save history on exit
            let _ = rl.save_history(&history_path());
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        _msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);

        // If we were streaming, the content was already printed via StreamChunk.
        // Just finish the line and reset.
        if self.is_streaming.swap(false, Ordering::Relaxed) {
            println!();
            println!();
            return Ok(());
        }

        // Dim separator line before the response
        let sep_width = width.min(80);
        let branding = TerminalBranding::from_skin(self.current_skin());
        eprintln!("{}", branding.separator(sep_width));

        // Render markdown
        let skin = make_skin(&self.current_skin());
        let text = termimad::FmtText::from(&skin, &response.content, Some(width));

        print!("{text}");
        println!();
        Ok(())
    }

    async fn send_status(
        &self,
        status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        let debug = self.is_debug();
        let skin = self.current_skin();
        let branding = TerminalBranding::from_skin(skin.clone());
        let muted = skin.ansi_fg(skin.muted);
        let accent = skin.ansi_fg(skin.accent);
        let warn = skin.ansi_fg(skin.warn);
        let good = skin.ansi_fg(skin.good);
        let bad = skin.ansi_fg(skin.bad);
        let reset = skin.ansi_reset();

        match status {
            StatusUpdate::Thinking(msg) => {
                let display = truncate_for_preview(&msg, CLI_STATUS_MAX);
                eprintln!("  {muted}\u{25CB} {display}{reset}");
            }
            StatusUpdate::ToolStarted { name, .. } => {
                eprintln!("  {warn}\u{25CB} {}{reset}", skin.tool_label(&name));
            }
            StatusUpdate::ToolCompleted { name, success, .. } => {
                if success {
                    eprintln!("  {good}\u{25CF} {}{reset}", skin.tool_label(&name));
                } else {
                    eprintln!("  {bad}\u{2717} {} (failed){reset}", skin.tool_label(&name));
                }
            }
            StatusUpdate::ToolResult { name: _, preview } => {
                let display = truncate_for_preview(&preview, CLI_TOOL_RESULT_MAX);
                eprintln!("    {muted}{display}{reset}");
            }
            StatusUpdate::StreamChunk(chunk) => {
                // Print separator on the false-to-true transition
                if !self.is_streaming.swap(true, Ordering::Relaxed) {
                    let width = crossterm::terminal::size()
                        .map(|(w, _)| w as usize)
                        .unwrap_or(80);
                    let sep_width = width.min(80);
                    eprintln!("{}", branding.separator(sep_width));
                }
                print!("{chunk}");
                let _ = io::stdout().flush();
            }
            StatusUpdate::JobStarted {
                job_id,
                title,
                browse_url,
            } => {
                eprintln!(
                    "  {accent}[job]{reset} {title} {muted}({job_id}){reset} \x1b[4m{browse_url}\x1b[0m"
                );
            }
            StatusUpdate::Status(msg) => {
                if debug || msg.contains("approval") || msg.contains("Approval") {
                    let display = truncate_for_preview(&msg, CLI_STATUS_MAX);
                    eprintln!("  {muted}{display}{reset}");
                }
            }
            StatusUpdate::ApprovalNeeded {
                request_id,
                tool_name,
                description,
                parameters,
            } => {
                let term_width = crossterm::terminal::size()
                    .map(|(w, _)| w as usize)
                    .unwrap_or(80);
                let box_width = (term_width.saturating_sub(4)).clamp(40, 60);

                // Short request ID for the bottom border
                let short_id = if request_id.len() > 8 {
                    &request_id[..8]
                } else {
                    &request_id
                };

                // Top border: ┌ tool_name requires approval ───
                let top_label = format!(" {tool_name} requires approval ");
                let top_fill = box_width.saturating_sub(top_label.len() + 1);
                let top_border = format!(
                    "\u{250C}{}{}",
                    branding.warn(top_label),
                    "\u{2500}".repeat(top_fill)
                );

                // Bottom border: └─ short_id ─────
                let bot_label = format!(" {short_id} ");
                let bot_fill = box_width.saturating_sub(bot_label.len() + 2);
                let bot_border = format!(
                    "\u{2514}\u{2500}{}{}",
                    branding.muted(bot_label),
                    "\u{2500}".repeat(bot_fill)
                );

                eprintln!();
                eprintln!("  {top_border}");
                eprintln!("  \u{2502} {}", branding.muted(description));
                eprintln!("  \u{2502}");

                // Params
                let param_lines = format_json_params(&branding, &parameters, "  \u{2502}   ");
                // The format_json_params already includes the indent prefix
                // but we need to handle the case where each line already starts with it
                for line in param_lines.lines() {
                    eprintln!("{line}");
                }

                eprintln!("  \u{2502}");
                eprintln!(
                    "  \u{2502} {} / {} / {}",
                    branding.good("yes (y)"),
                    branding.warn("always (a)"),
                    branding.bad("no (n)")
                );
                eprintln!("  {bot_border}");
                eprintln!();
            }
            StatusUpdate::AuthRequired {
                extension_name,
                instructions,
                setup_url,
                ..
            } => {
                eprintln!();
                eprintln!(
                    "  {}",
                    branding.warn(format!("Authentication required for {extension_name}"))
                );
                if let Some(ref instr) = instructions {
                    eprintln!("  {}", branding.body(instr));
                }
                if let Some(ref url) = setup_url {
                    eprintln!("  \x1b[4m{url}\x1b[0m");
                }
                eprintln!();
            }
            StatusUpdate::AuthCompleted {
                extension_name,
                success,
                message,
                ..
            } => {
                if success {
                    eprintln!(
                        "  {}",
                        branding.good(format!("{extension_name}: {message}"))
                    );
                } else {
                    eprintln!("  {}", branding.bad(format!("{extension_name}: {message}")));
                }
            }
            StatusUpdate::Error { message, code } => {
                let code_str = code.as_deref().unwrap_or("error");
                eprintln!(
                    "  {}",
                    branding.bad(format!("\u{2717} [{code_str}] {message}"))
                );
            }
            StatusUpdate::CanvasAction(ref action) => {
                let summary = match action {
                    crate::tools::builtin::CanvasAction::Show {
                        panel_id, title, ..
                    } => {
                        format!("show \"{title}\" ({panel_id})")
                    }
                    crate::tools::builtin::CanvasAction::Update { panel_id, .. } => {
                        format!("update ({panel_id})")
                    }
                    crate::tools::builtin::CanvasAction::Dismiss { panel_id } => {
                        format!("dismiss ({panel_id})")
                    }
                    crate::tools::builtin::CanvasAction::Notify { message, level, .. } => {
                        format!("notify [{level:?}] {message}")
                    }
                };
                eprintln!(
                    "  {}",
                    branding.accent(format!("\u{25A0} canvas:{summary}"))
                );
            }
            StatusUpdate::AgentMessage {
                content,
                message_type,
            } => {
                let width = crossterm::terminal::size()
                    .map(|(w, _)| w as usize)
                    .unwrap_or(80);
                let title = match message_type.as_str() {
                    "warning" => branding.warn("┌─ agent warning ─"),
                    "question" => branding.accent("┌─ agent question ─"),
                    "interim_result" => branding.good("┌─ agent interim result ─"),
                    _ => branding.accent_soft("┌─ agent note ─"),
                };
                eprintln!("  {title}");
                let skin = make_skin(&self.current_skin());
                let text = termimad::FmtText::from(&skin, &content, Some(width));
                eprint!("{text}");
                eprintln!();
                eprintln!("  {}", branding.muted("└────────────────────────────────"));
            }
            // Lifecycle events are informational for the frontend;
            // the REPL already shows a streaming indicator via is_streaming.
            StatusUpdate::LifecycleStart { .. } | StatusUpdate::LifecycleEnd { .. } => {}

            // Sub-agent lifecycle events
            StatusUpdate::SubagentSpawned { name, task, .. } => {
                eprintln!("  {}", branding.accent(format!("┌─ sub-agent: {name}")));
                eprintln!("  {}", branding.muted(format!("│ task: {task}")));
                eprintln!("  {}", branding.muted("└────────────────────────────────"));
            }
            StatusUpdate::SubagentProgress { message, .. } => {
                let display = truncate_for_preview(&message, CLI_STATUS_MAX);
                eprintln!("  {}", branding.muted(format!("│ progress: {display}")));
            }
            StatusUpdate::SubagentCompleted {
                name,
                success,
                duration_ms,
                ..
            } => {
                let secs = duration_ms as f64 / 1000.0;
                if success {
                    eprintln!(
                        "  {}",
                        branding.good(format!("└─ sub-agent '{name}' completed in {secs:.1}s"))
                    );
                } else {
                    eprintln!(
                        "  {}",
                        branding.bad(format!("└─ sub-agent '{name}' failed after {secs:.1}s"))
                    );
                }
            }
        }
        Ok(())
    }

    async fn broadcast(
        &self,
        _user_id: &str,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        let current_skin = self.current_skin();
        let skin = make_skin(&current_skin);
        let width = crossterm::terminal::size()
            .map(|(w, _)| w as usize)
            .unwrap_or(80);

        eprintln!(
            "{}",
            TerminalBranding::from_skin(current_skin).accent("\u{25CF} notification")
        );
        let text = termimad::FmtText::from(&skin, &response.content, Some(width));
        eprint!("{text}");
        eprintln!();
        Ok(())
    }

    async fn health_check(&self) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repl_formatting_hints_are_absent() {
        assert_eq!(ReplChannel::new().formatting_hints(), None);
    }
}
