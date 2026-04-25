//! CLI command handling.
//!
//! Provides subcommands for:
//! - Running the agent (`run`)
//! - Interactive onboarding wizard (`onboard`)
//! - Managing configuration (`config list`, `config get`, `config set`)
//! - Managing WASM tools (`tool install`, `tool list`, `tool remove`)
//! - Managing MCP servers (`mcp add`, `mcp auth`, `mcp list`, `mcp test`)
//! - Querying workspace memory (`memory search`, `memory read`, `memory write`)
//! - Managing agent workspaces (`agents add`, `agents list`, `agents remove`)
//! - Listing sessions (`sessions list`, `sessions show`, `sessions prune`)
//! - Managing OS service (`service install`, `service start`, `service stop`)
//! - Active health diagnostics (`doctor`)
//! - Checking system health (`status`)

pub mod agents;
mod browser;
mod channels;
mod completion;
mod config;
mod cron;
mod doctor;
mod experiments;
mod gateway;
mod identity;
mod logs;
mod mcp;
pub mod memory;
mod message;
mod models;
pub mod nodes;
pub mod oauth_defaults;
mod pairing;
mod registry;
mod reset;
mod secrets;
#[cfg(feature = "repl")]
mod service;
pub mod session_export;
pub mod sessions;
pub mod status;
pub mod subagent_spawn;
mod tool;
pub mod trajectory;
mod update;

pub use agents::{AgentCommand, run_agents_command};
pub use browser::{BrowserCommand, run_browser_command};
pub use channels::{ChannelCommand, run_channels_command};
pub use completion::Completion;
pub use config::{ConfigCommand, run_config_command};
pub use cron::{CronCommand, run_cron_command};
pub use doctor::run_doctor_command;
pub use experiments::{ExperimentsCommand, run_experiments_command};
pub use gateway::{GatewayCommand, run_gateway_command};
pub use identity::{IdentityCommand, run_identity_command};
pub use logs::{LogCommand, run_log_command};
pub use mcp::{McpCommand, run_mcp_command};
pub use memory::MemoryCommand;
#[cfg(feature = "postgres")]
pub use memory::run_memory_command;
pub use memory::run_memory_command_with_db;
pub use message::{MessageCommand, run_message_command};
pub use models::{ModelCommand, run_model_command};
pub use pairing::{PairingCommand, run_pairing_command, run_pairing_command_with_store};
pub use registry::{RegistryCommand, run_registry_command};
pub use reset::{ResetCommand, run_reset_command};
pub use secrets::{SecretsCommand, run_secrets_command};
#[cfg(feature = "repl")]
pub use service::{ServiceCommand, run_service_command};
pub use sessions::{SessionCommand, run_sessions_command};
pub use status::run_status_command;
pub use tool::{ToolCommand, run_tool_command};
pub use trajectory::{TrajectoryCommand, run_trajectory_command};
pub use update::{UpdateCommand, run_update_command};

use clap::{Parser, Subcommand, ValueEnum};

use crate::setup::{GuideTopic, OnboardingProfile, UiMode};

#[derive(Parser, Debug)]
#[command(name = "thinclaw")]
#[command(about = "Secure personal agent that protects your data and expands its capabilities")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Enable verbose terminal logs for debugging
    #[arg(long, global = true)]
    pub debug: bool,

    /// Run in interactive CLI mode only (disable other channels)
    #[arg(long, global = true)]
    pub cli_only: bool,

    /// Skip database connection (for testing)
    #[arg(long, global = true)]
    pub no_db: bool,

    /// Single message mode - send one message and exit
    #[arg(short, long, global = true)]
    pub message: Option<String>,

    /// Configuration file path (optional, uses env vars by default)
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,

    /// Skip first-run onboarding check
    #[arg(long, global = true)]
    pub no_onboard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum LinuxReadinessCliProfile {
    Server,
    Remote,
    #[value(name = "desktop-linux", alias = "desktop-gnome", alias = "desktop")]
    DesktopLinux,
    #[value(name = "pi-os-lite-64")]
    PiOsLite64,
    AllFeatures,
}

impl From<LinuxReadinessCliProfile> for crate::platform::LinuxReadinessProfile {
    fn from(value: LinuxReadinessCliProfile) -> Self {
        match value {
            LinuxReadinessCliProfile::Server => Self::Server,
            LinuxReadinessCliProfile::Remote => Self::Remote,
            LinuxReadinessCliProfile::DesktopLinux => Self::DesktopLinux,
            LinuxReadinessCliProfile::PiOsLite64 => Self::PiOsLite64,
            LinuxReadinessCliProfile::AllFeatures => Self::AllFeatures,
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Run the agent (default if no subcommand given)
    Run,

    /// Run the agent with the full-screen terminal UI
    Tui,

    /// Interactive onboarding wizard
    Onboard {
        /// Skip authentication (use existing session)
        #[arg(long)]
        skip_auth: bool,

        /// Reconfigure channels only
        #[arg(long)]
        channels_only: bool,

        /// Revisit guided settings by topic. Use without a value to open the topic menu.
        #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "menu")]
        guide: Option<GuideTopic>,

        /// Onboarding interface mode
        #[arg(long, value_enum, default_value_t = UiMode::Auto)]
        ui: UiMode,

        /// Preselect an onboarding profile, e.g. remote for SSH-managed hosts.
        #[arg(long, value_enum)]
        profile: Option<OnboardingProfile>,
    },

    /// Fully reset ThinClaw state so onboarding can start fresh
    Reset(ResetCommand),

    /// Manage encrypted user/API secrets
    #[command(subcommand)]
    Secrets(SecretsCommand),

    /// Manage configuration settings
    #[command(subcommand)]
    Config(ConfigCommand),

    /// Manage scheduled routines (cron jobs)
    #[command(subcommand)]
    Cron(CronCommand),

    /// Manage optional experiment/research automation, including opportunities, targets, providers, and campaigns.
    #[command(subcommand)]
    Experiments(ExperimentsCommand),

    /// Manage the web gateway
    #[command(subcommand)]
    Gateway(GatewayCommand),

    /// Manage household actors and linked endpoints
    #[command(subcommand)]
    Identity(IdentityCommand),

    /// Manage messaging channels
    #[command(subcommand)]
    Channels(ChannelCommand),

    /// Manage WASM tools
    #[command(subcommand)]
    Tool(ToolCommand),

    /// Browse and install extensions from the registry
    #[command(subcommand)]
    Registry(RegistryCommand),

    /// Manage MCP servers (hosted tool providers)
    #[command(subcommand)]
    Mcp(McpCommand),

    /// Query and manage workspace memory
    #[command(subcommand)]
    Memory(MemoryCommand),

    /// Send messages to the agent
    #[command(subcommand)]
    Message(MessageCommand),

    /// List and inspect available LLM models
    #[command(subcommand)]
    Models(ModelCommand),

    /// DM pairing (approve inbound requests from unknown senders)
    #[command(subcommand)]
    Pairing(PairingCommand),

    /// Manage agent workspaces (register, list, remove agents)
    #[command(subcommand)]
    Agents(AgentCommand),

    /// Manage active sessions (list, show, prune)
    #[command(subcommand)]
    Sessions(SessionCommand),

    /// Manage OS service (launchd / systemd / Windows Service Control Manager)
    #[cfg(feature = "repl")]
    #[command(subcommand)]
    Service(ServiceCommand),

    /// Internal Windows SCM entrypoint.
    #[cfg(all(feature = "repl", target_os = "windows"))]
    #[command(name = "__windows-service", hide = true)]
    WindowsServiceRuntime {
        /// Preserve the configured ThinClaw home for the service account.
        #[arg(long)]
        home: Option<std::path::PathBuf>,
    },

    /// Probe external dependencies and validate configuration
    Doctor {
        /// Linux readiness profile to evaluate
        #[arg(long, value_enum, default_value_t = LinuxReadinessCliProfile::Server)]
        profile: LinuxReadinessCliProfile,
    },

    /// Show system health and diagnostics
    Status {
        /// Linux readiness profile to summarize
        #[arg(long, value_enum, default_value_t = LinuxReadinessCliProfile::Server)]
        profile: LinuxReadinessCliProfile,
    },

    /// Query and filter logs
    #[command(subcommand)]
    Logs(LogCommand),

    /// Browser automation (headless Chrome)
    #[command(subcommand)]
    Browser(BrowserCommand),

    /// Export or inspect archived agent trajectories
    #[command(subcommand)]
    Trajectory(TrajectoryCommand),

    /// Check for updates and self-update
    #[command(subcommand)]
    Update(UpdateCommand),

    /// Generate shell completion scripts
    Completion(Completion),

    /// Run as a sandboxed worker inside a Docker container (internal use).
    /// This is invoked automatically by the orchestrator, not by users directly.
    #[cfg(feature = "docker-sandbox")]
    Worker {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum iterations before stopping.
        #[arg(long, default_value = "50")]
        max_iterations: u32,
    },

    /// Run as a Claude Code bridge inside a Docker container (internal use).
    /// Spawns the `claude` CLI and streams output back to the orchestrator.
    #[cfg(feature = "docker-sandbox")]
    ClaudeBridge {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Maximum agentic turns for Claude Code.
        #[arg(long, default_value = "50")]
        max_turns: u32,

        /// Claude model to use (e.g. "claude-sonnet-4-6", "claude-opus-4-5").
        #[arg(long, default_value = "claude-sonnet-4-6")]
        model: String,
    },

    /// Run as a Codex bridge inside a Docker container (internal use).
    /// Spawns the `codex` CLI and streams output back to the orchestrator.
    #[cfg(feature = "docker-sandbox")]
    CodexBridge {
        /// Job ID to execute.
        #[arg(long)]
        job_id: uuid::Uuid,

        /// URL of the orchestrator's internal API.
        #[arg(long, default_value = "http://host.docker.internal:50051")]
        orchestrator_url: String,

        /// Codex model to use (e.g. "gpt-5.3-codex").
        #[arg(long, default_value = "gpt-5.3-codex")]
        model: String,
    },

    /// Run as a lease-scoped remote experiment runner (internal/automation use).
    ExperimentRunner {
        #[arg(long)]
        lease_id: uuid::Uuid,

        #[arg(long)]
        gateway_url: String,

        #[arg(long)]
        token: String,

        #[arg(long)]
        workspace_root: Option<std::path::PathBuf>,
    },

    /// Run the desktop autonomy shadow canary manifest (internal use).
    #[command(name = "autonomy-shadow-canary", hide = true)]
    AutonomyShadowCanary {
        #[arg(long)]
        manifest: std::path::PathBuf,
    },
}

impl Cli {
    /// Check if we should run the agent (default behavior or explicit `run` command).
    pub fn should_run_agent(&self) -> bool {
        matches!(self.command, None | Some(Command::Run) | Some(Command::Tui))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_version() {
        let cmd = Cli::command();
        assert_eq!(
            cmd.get_version().unwrap_or("unknown"),
            env!("CARGO_PKG_VERSION")
        );
    }

    #[test]
    fn test_debug_flag_defaults_to_false() {
        let cli = Cli::try_parse_from(["thinclaw"]).expect("parse default cli");
        assert!(!cli.debug);
    }

    #[test]
    fn test_debug_flag_parses_globally() {
        let cli = Cli::try_parse_from(["thinclaw", "--debug", "status"])
            .expect("parse cli with global debug flag");
        assert!(cli.debug);
        assert!(matches!(cli.command, Some(Command::Status { .. })));
    }

    #[test]
    fn test_linux_readiness_profile_parses() {
        let cli = Cli::try_parse_from(["thinclaw", "doctor", "--profile", "desktop-gnome"])
            .expect("parse doctor profile");
        assert!(matches!(
            cli.command,
            Some(Command::Doctor {
                profile: LinuxReadinessCliProfile::DesktopLinux
            })
        ));
    }

    #[test]
    fn test_remote_readiness_profile_parses() {
        let cli = Cli::try_parse_from(["thinclaw", "doctor", "--profile", "remote"])
            .expect("parse remote doctor profile");
        assert!(matches!(
            cli.command,
            Some(Command::Doctor {
                profile: LinuxReadinessCliProfile::Remote
            })
        ));

        let cli = Cli::try_parse_from(["thinclaw", "status", "--profile", "remote"])
            .expect("parse remote status profile");
        assert!(matches!(
            cli.command,
            Some(Command::Status {
                profile: LinuxReadinessCliProfile::Remote
            })
        ));
    }

    #[test]
    fn test_onboard_remote_profile_parses() {
        let cli = Cli::try_parse_from(["thinclaw", "onboard", "--profile", "remote"])
            .expect("parse remote onboarding profile");
        assert!(matches!(
            cli.command,
            Some(Command::Onboard {
                profile: Some(OnboardingProfile::RemoteServer),
                ..
            })
        ));
    }

    #[test]
    fn test_onboard_pi_os_lite_profile_parses() {
        let cli = Cli::try_parse_from(["thinclaw", "onboard", "--profile", "pi-os-lite-64"])
            .expect("parse Pi OS Lite onboarding profile");
        assert!(matches!(
            cli.command,
            Some(Command::Onboard {
                profile: Some(OnboardingProfile::PiOsLite64),
                ..
            })
        ));
    }

    #[test]
    fn test_pi_os_lite_readiness_profile_parses() {
        let cli = Cli::try_parse_from(["thinclaw", "doctor", "--profile", "pi-os-lite-64"])
            .expect("parse pi doctor profile");
        assert!(matches!(
            cli.command,
            Some(Command::Doctor {
                profile: LinuxReadinessCliProfile::PiOsLite64
            })
        ));

        let cli = Cli::try_parse_from(["thinclaw", "status", "--profile", "pi-os-lite-64"])
            .expect("parse pi status profile");
        assert!(matches!(
            cli.command,
            Some(Command::Status {
                profile: LinuxReadinessCliProfile::PiOsLite64
            })
        ));
    }

    #[test]
    fn test_linux_desktop_readiness_alias_parses() {
        let cli = Cli::try_parse_from(["thinclaw", "doctor", "--profile", "desktop-linux"])
            .expect("parse linux desktop doctor profile alias");
        assert!(matches!(
            cli.command,
            Some(Command::Doctor {
                profile: LinuxReadinessCliProfile::DesktopLinux
            })
        ));
    }

    #[test]
    fn test_tui_command_runs_agent() {
        let cli = Cli::try_parse_from(["thinclaw", "tui"]).expect("parse tui command");
        assert!(cli.should_run_agent());
        assert!(matches!(cli.command, Some(Command::Tui)));
    }
}
