> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`../CLAUDE.md`](../CLAUDE.md) for current documentation.

---

# CLI: Command-Line Interface for ThinClaw

ThinClaw ships a full CLI built with `clap` for managing the Orchestrator, channels, models, memory, cron, hooks, security, and more — accessible when the Tauri GUI isn't available (headless servers, SSH sessions, automation scripts).

---

## 1. Design Philosophy

- **One binary, three modes:**
  - `thinclaw` → launches Tauri desktop app (default)
  - `thinclaw tui` → interactive terminal chat UI
  - `thinclaw --headless` → starts headless Orchestrator
  - `thinclaw <command>` → CLI management commands
- **Feature-gated:** `#[cfg(feature = "cli")]` means CLI code is stripped from the Tauri-only desktop build if desired.
- **Shared core:** CLI commands call the same Rust functions as the Tauri IPC handlers — zero code duplication.

---

## 2. Full Command Tree

### Top-Level Commands

```
thinclaw
├── onboard           # First-time setup wizard (see SETUP_WIZARD_RS.md)
├── start             # Start the Orchestrator (foreground or daemon)
├── stop              # Stop a running Orchestrator
├── tui               # Launch interactive terminal chat UI
├── status            # Show gateway status summary
├── health            # Run health checks (connectivity, config, services)
├── doctor            # Diagnose and repair issues
├── dashboard         # Open the web control UI (if enabled)
├── reset             # Reset config/state (scoped: config | config+creds | full)
├── uninstall         # Remove service + state + workspace
│
├── config            # Configuration management
│   ├── get <path>    # Read a config value (dot-notation)
│   ├── set <path> <value>  # Set a config value
│   ├── unset <path>  # Remove a config value
│   └── edit          # Open config.toml in $EDITOR
│
├── models            # Model management
│   ├── list          # List available models (cloud + local)
│   ├── status        # Show active model, provider, context usage
│   ├── set <model>   # Set the default model
│   └── probe         # Test model connectivity and latency
│
├── agents            # Multi-agent management
│   ├── list          # List all configured agents
│   ├── add <id>      # Create a new agent identity
│   ├── delete <id>   # Remove an agent
│   └── set-identity <id> <file>  # Set agent's SOUL/IDENTITY file
│
├── sessions          # Session management
│   ├── list          # List active sessions with metadata
│   ├── cleanup       # Remove stale/orphaned sessions
│   └── delete <key>  # Delete a specific session
│
├── memory            # Vector memory management
│   ├── status        # Show memory stats (entry count, index health)
│   ├── search <query># Search memory with a natural language query
│   ├── add <text>    # Manually add a memory entry
│   ├── clear         # Clear all memory (with confirmation)
│   └── export        # Export memory to JSON
│
├── cron              # Scheduled task management
│   ├── list          # List all cron jobs with next-run times
│   ├── add           # Create a new cron job (interactive)
│   ├── run <id>      # Force-run a job immediately
│   ├── enable <id>   # Enable a paused job
│   ├── disable <id>  # Pause a job
│   └── remove <id>   # Delete a job
│
├── channels          # Messaging channel management
│   ├── list          # List configured channels with status
│   ├── add <type>    # Add a new channel (telegram, discord, etc.)
│   ├── remove <type> # Remove a channel
│   ├── test <type>   # Send a test message
│   └── auth <type>   # Re-authenticate a channel
│
├── hooks             # Event hook management
│   ├── list          # List registered hooks
│   ├── gmail setup   # Configure Gmail Pub/Sub integration
│   └── test <hook>   # Fire a test event
│
├── webhooks          # Webhook endpoint management
│   ├── list          # List webhook endpoints
│   ├── add           # Create a new webhook endpoint
│   └── remove <name> # Delete a webhook endpoint
│
├── plugins           # MCP plugin management
│   ├── list          # List installed plugins
│   ├── install <uri> # Install a plugin from URI
│   ├── remove <name> # Uninstall a plugin
│   └── update        # Update all plugins
│
├── skills            # Agent skill management
│   ├── list          # List installed skills
│   ├── install <path># Install a skill from a directory
│   └── remove <name> # Remove a skill
│
├── browser           # Browser tool management
│   ├── inspect       # Show active browser sessions
│   ├── manage        # Chrome profile management
│   └── debug         # Debug CDP connections
│
├── sandbox           # Sandbox management
│   ├── status        # Show sandbox engine status
│   └── test          # Run a test command in the sandbox
│
├── security          # Security tools
│   ├── audit         # Run security audit (--deep, --fix)
│   └── approvals     # Manage execution approval policies
│
├── daemon            # OS service management
│   ├── install       # Install as launchd/systemd service
│   ├── uninstall     # Remove the service
│   ├── start         # Start the service
│   ├── stop          # Stop the service
│   ├── restart       # Restart the service
│   ├── status        # Show service status
│   └── logs          # Tail service logs
│
├── logs              # Log management
│   ├── tail          # Tail Orchestrator logs
│   └── search <q>    # Search logs
│
├── dns               # Tailscale / discovery helpers
│   ├── status        # Show Tailscale discovery status
│   └── resolve       # Resolve Orchestrator address
│
├── devices           # Device pairing
│   ├── list          # List paired devices
│   ├── pair          # Generate a pairing token/QR
│   └── revoke <id>   # Revoke a device
│
├── update            # Self-update
│   ├── check         # Check for available updates
│   └── apply         # Download and apply update
│
└── completion        # Shell completion generation
    ├── bash          # Generate bash completions
    ├── zsh           # Generate zsh completions
    └── fish          # Generate fish completions
```

---

## 3. Rust Implementation with `clap`

### Top-Level Parser

```rust
use clap::{Parser, Subcommand, Args};

#[derive(Parser)]
#[command(
    name = "thinclaw",
    about = "ThinClaw AI Agent — Personal AI Assistant",
    version,
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Start in headless mode (no GUI)
    #[arg(long)]
    pub headless: bool,

    /// Path to config file
    #[arg(long, default_value = "~/.config/thinclaw/config.toml")]
    pub config: String,

    /// Log verbosity
    #[arg(long, default_value = "info")]
    pub log_level: String,
}

#[derive(Subcommand)]
pub enum Commands {
    /// First-time setup wizard
    Onboard(OnboardArgs),
    /// Start the Orchestrator
    Start(StartArgs),
    /// Stop a running Orchestrator
    Stop,
    /// Launch interactive terminal chat
    Tui(TuiArgs),
    /// Show gateway status
    Status(StatusArgs),
    /// Health checks
    Health(HealthArgs),
    /// Diagnose and repair
    Doctor(DoctorArgs),
    /// Reset config/state
    Reset(ResetArgs),

    // --- Subsystem CLIs ---
    /// Configuration management
    Config {
        #[command(subcommand)]
        action: ConfigCommands,
    },
    /// Model management
    Models {
        #[command(subcommand)]
        action: ModelCommands,
    },
    /// Agent management
    Agents {
        #[command(subcommand)]
        action: AgentCommands,
    },
    /// Session management
    Sessions {
        #[command(subcommand)]
        action: SessionCommands,
    },
    /// Memory management
    Memory {
        #[command(subcommand)]
        action: MemoryCommands,
    },
    /// Cron job management
    Cron {
        #[command(subcommand)]
        action: CronCommands,
    },
    /// Channel management
    Channels {
        #[command(subcommand)]
        action: ChannelCommands,
    },
    /// Hook management
    Hooks {
        #[command(subcommand)]
        action: HookCommands,
    },
    /// Plugin management
    Plugins {
        #[command(subcommand)]
        action: PluginCommands,
    },
    /// Security tools
    Security {
        #[command(subcommand)]
        action: SecurityCommands,
    },
    /// Daemon/service management
    Daemon {
        #[command(subcommand)]
        action: DaemonCommands,
    },
    /// Self-update
    Update {
        #[command(subcommand)]
        action: UpdateCommands,
    },
    /// Shell completions
    Completion {
        #[command(subcommand)]
        shell: CompletionShell,
    },
}
```

### Example Subcommand Group

```rust
#[derive(Subcommand)]
pub enum CronCommands {
    /// List all cron jobs
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Create a new cron job
    Add(CronAddArgs),
    /// Force-run a job
    Run {
        /// Job ID to run
        id: String,
    },
    /// Enable a paused job
    Enable { id: String },
    /// Disable a job
    Disable { id: String },
    /// Delete a job
    Remove { id: String },
}

#[derive(Args)]
pub struct CronAddArgs {
    /// Job name
    #[arg(long)]
    pub name: String,
    /// Schedule (e.g., "every 24h" or "0 9 * * *")
    #[arg(long)]
    pub schedule: String,
    /// Message to send to the agent
    #[arg(long)]
    pub message: String,
    /// Delivery channel (e.g., "telegram")
    #[arg(long)]
    pub deliver: Option<String>,
    /// Delete after first run
    #[arg(long)]
    pub once: bool,
}
```

---

## 4. Shared Command Implementations

Commands are implemented as standalone async functions that both CLI and Tauri IPC can call:

```rust
// src/commands/cron.rs — Used by BOTH CLI and Tauri IPC

pub async fn cron_list(orchestrator: &Orchestrator, json: bool) -> Result<String> {
    let jobs = orchestrator.cron_service().list().await;
    if json {
        Ok(serde_json::to_string_pretty(&jobs)?)
    } else {
        Ok(format_cron_table(&jobs))
    }
}

// CLI handler (in src/cli/cron.rs)
async fn handle_cron_list(args: &CronListArgs) -> Result<()> {
    let orchestrator = connect_to_orchestrator().await?;
    let output = cron_list(&orchestrator, args.json).await?;
    println!("{}", output);
    Ok(())
}

// Tauri IPC handler (in src-tauri/src/commands/cron.rs)
#[tauri::command]
async fn cron_list(state: State<'_, OrchestratorState>) -> Result<Vec<CronJob>, String> {
    let orchestrator = state.orchestrator.lock().await;
    orchestrator.cron_service().list().await.map_err(|e| e.to_string())
}
```

---

## 5. Global Flags

Every command supports these global flags:

| Flag | Type | Description |
|---|---|---|
| `--config <path>` | `String` | Path to config.toml (default: `~/.config/thinclaw/config.toml`) |
| `--log-level <level>` | `String` | `trace`, `debug`, `info`, `warn`, `error` |
| `--json` | `bool` | Output in JSON format (for scripting) |
| `--quiet` | `bool` | Suppress non-essential output |
| `--verbose` | `bool` | Show detailed output |
| `--timeout <ms>` | `u64` | Timeout for gateway communication |
| `--non-interactive` | `bool` | Disable all prompts (for CI/scripts) |

---

## 6. Shell Completions

Auto-generated via `clap_complete`:

```rust
use clap_complete::{generate, shells::{Bash, Zsh, Fish}};

fn generate_completions(shell: CompletionShell) {
    let mut cmd = Cli::command();
    match shell {
        CompletionShell::Bash => generate(Bash, &mut cmd, "thinclaw", &mut io::stdout()),
        CompletionShell::Zsh => generate(Zsh, &mut cmd, "thinclaw", &mut io::stdout()),
        CompletionShell::Fish => generate(Fish, &mut cmd, "thinclaw", &mut io::stdout()),
    }
}
```

Users install with:
```bash
# Bash
thinclaw completion bash > ~/.bash_completion.d/thinclaw

# Zsh
thinclaw completion zsh > ~/.zfunc/_thinclaw

# Fish
thinclaw completion fish > ~/.config/fish/completions/thinclaw.fish
```

---

## 7. CLI → Orchestrator Communication

When the CLI needs to talk to a running Orchestrator (for `status`, `cron list`, etc.), it connects via the same WebSocket protocol defined in `NETWORKING_RS.md`:

```rust
pub async fn connect_to_orchestrator() -> Result<OrchestratorClient> {
    let config = load_config()?;
    let url = format!("ws://127.0.0.1:{}", config.gateway.port);
    let token = config.gateway.auth.token.as_deref();

    let client = OrchestratorClient::connect(&url, token).await
        .context("Cannot connect to Orchestrator. Is it running? Try: thinclaw start")?;

    Ok(client)
}
```

For commands that don't need a running Orchestrator (e.g., `config get`, `completion`, `onboard`), they work directly with the config file.

---

## 8. Crate Dependencies

```toml
[dependencies]
clap = { version = "4", features = ["derive", "env"] }
clap_complete = "4"        # Shell completion generation
comfy-table = "7"          # Pretty table formatting
indicatif = "0.17"         # Progress bars and spinners
console = "0.15"           # Terminal colors and styles
dialoguer = "0.11"         # Interactive prompts (for non-TUI commands)
```
