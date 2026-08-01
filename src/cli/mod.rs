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
mod backup;
mod browser;
mod channels;
mod comfy;
mod completion;
mod config;
mod context;
mod cron;
mod devices;
mod doctor;
mod experiments;
mod gateway;
mod gateway_client;
mod identity;
mod logs;
mod mcp;
pub mod memory;
mod message;
mod models;
pub mod oauth_defaults;
mod outcome;
mod output;
mod pairing;
mod registry;
mod repo_projects;
mod reset;
mod secrets;
#[cfg(feature = "repl")]
mod service;
pub mod sessions;
pub mod status;
mod tool;
pub mod trajectory;
mod update;

pub use agents::{AgentCommand, run_agents_command};
pub use backup::{BackupCommand, run_backup_command};
pub use browser::{BrowserCommand, run_browser_command};
pub use channels::{ChannelCommand, run_channels_command};
pub use comfy::{ComfyCommand, run_comfy_command};
pub use completion::Completion;
pub use config::{ConfigCommand, run_config_command};
pub use context::{CliContext, CliContextOptions};
pub use cron::{CronCommand, run_cron_command};
pub use devices::{DeviceCommand, run_devices_command};
pub use doctor::run_doctor_command;
pub use experiments::{ExperimentsCommand, run_experiments_command};
pub use gateway::{GatewayCommand, run_gateway_command};
pub use gateway_client::{
    GatewayAuthToken, GatewayClient, GatewayClientError, GatewayRequestBudget,
};
pub use identity::{IdentityCommand, run_identity_command};
pub use logs::{LogCommand, run_log_command};
pub use mcp::{McpCommand, run_mcp_command};
pub use memory::MemoryCommand;
#[cfg(feature = "postgres")]
pub use memory::run_memory_command;
pub use memory::run_memory_command_with_db;
pub use message::{MessageCommand, run_message_command};
pub use models::{ModelCommand, run_model_command};
pub use outcome::{CliDispatch, CliError, CliOutcome, ExitClass};
pub use output::{ColorChoice, OutputFormat, OutputPolicy};
pub use pairing::{PairingCommand, run_pairing_command, run_pairing_command_with_store};
pub use registry::{RegistryCommand, run_registry_command};
pub use repo_projects::{RepoProjectCommand, run_repo_projects_command};
pub use reset::{ResetCommand, run_reset_command};
pub use secrets::{SecretsCommand, run_secrets_command};
#[cfg(feature = "repl")]
pub use service::{ServiceCommand, run_service_command};
pub use sessions::{SessionCommand, run_sessions_command};
pub use status::run_status_command;
pub use tool::{ToolCommand, run_tool_command};
pub use trajectory::{TrajectoryCommand, run_trajectory_command};
pub use update::{UpdateCommand, run_update_command};

use clap::{Args, Parser, Subcommand, ValueEnum};

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

    /// Output presentation format for command data
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    pub output_format: OutputFormat,

    /// Terminal color policy
    #[arg(long, global = true, value_enum, default_value_t = ColorChoice::Auto)]
    pub color: ColorChoice,

    /// Suppress nonessential human diagnostics and progress
    #[arg(long, global = true, conflicts_with = "verbose")]
    pub quiet: bool,

    /// Include additional human diagnostics
    #[arg(long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Deprecated alias for `run --channels none`
    #[arg(long = "cli-only", global = true, hide = true)]
    pub legacy_cli_only: bool,

    /// Deprecated alias for `ask TEXT`
    #[arg(
        short = 'm',
        long = "message",
        global = true,
        hide = true,
        conflicts_with = "command"
    )]
    pub legacy_message: Option<String>,

    /// Configuration file path (optional, uses env vars by default)
    #[arg(short, long, global = true)]
    pub config: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelSelectionArg {
    None,
    Configured,
    Selected(Vec<String>),
}

impl ChannelSelectionArg {
    pub fn allows(&self, channel: &str) -> bool {
        match self {
            Self::None => false,
            Self::Configured => true,
            Self::Selected(channels) => channels.iter().any(|selected| selected == channel),
        }
    }

    pub const fn disables_external_ingress(&self) -> bool {
        matches!(self, Self::None)
    }
}

impl std::str::FromStr for ChannelSelectionArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        match value {
            "none" => Ok(Self::None),
            "configured" => Ok(Self::Configured),
            "" => Err("channel selection cannot be empty".to_string()),
            _ => {
                let mut channels = Vec::new();
                for channel in value.split(',') {
                    let channel = channel.trim();
                    if channel.is_empty()
                        || channel.len() > 128
                        || !channel.chars().all(|character| {
                            character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
                        })
                    {
                        return Err(format!("invalid channel name '{channel}'"));
                    }
                    let channel = channel.to_ascii_lowercase().replace('_', "-");
                    if !channels.contains(&channel) {
                        channels.push(channel);
                    }
                }
                Ok(Self::Selected(channels))
            }
        }
    }
}

#[derive(Args, Debug, Clone, Default)]
pub struct RuntimeArgs {
    /// Skip database connection (testing and diagnostics only)
    #[arg(long, hide = true)]
    pub no_db: bool,

    /// Skip the first-run setup check
    #[arg(long)]
    pub skip_setup_check: bool,

    /// Deprecated alias for `--skip-setup-check`
    #[arg(long = "no-onboard", hide = true)]
    pub legacy_no_onboard: bool,

    /// Select nonlocal ingress: none, configured, or comma-separated channel names
    #[arg(long, value_name = "SELECTION")]
    pub channels: Option<ChannelSelectionArg>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRuntimeArgs {
    pub no_db: bool,
    pub skip_setup_check: bool,
    pub channels: ChannelSelectionArg,
    pub one_shot_message: Option<String>,
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
    Run(RuntimeArgs),

    /// Run the agent with the full-screen terminal UI
    Tui(RuntimeArgs),

    /// Run one local agent turn, then exit
    Ask {
        /// Message text for the local agent turn
        text: String,

        #[command(flatten)]
        runtime: RuntimeArgs,
    },

    /// Inject a message through the running web runtime
    Send {
        /// Message text to inject
        text: String,

        /// Principal/user identity for the injected message
        #[arg(long, default_value = "cli")]
        user_id: String,

        /// Explicit running-runtime URL
        #[arg(long)]
        gateway_url: Option<String>,
    },

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

    /// Manage paired mobile devices (pair, list, rename, revoke)
    #[command(subcommand)]
    Devices(DeviceCommand),

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

    /// Manage ComfyUI media generation
    #[command(subcommand)]
    Comfy(ComfyCommand),

    /// Manage WASM tools
    #[command(subcommand)]
    Tool(ToolCommand),

    /// Browse and install extensions from the registry
    #[command(subcommand)]
    Registry(RegistryCommand),

    /// Manage the GitHub repository project supervisor
    #[command(subcommand)]
    RepoProjects(RepoProjectCommand),

    /// Export or restore an encrypted whole-agent backup
    #[command(subcommand)]
    Backup(BackupCommand),

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
    #[command(hide = true)]
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
    #[command(hide = true)]
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

        /// Claude model to use (e.g. "claude-sonnet-5", "claude-opus-4-5").
        #[arg(long, default_value = "claude-sonnet-5")]
        model: String,
    },

    /// Run as a Codex bridge inside a Docker container (internal use).
    /// Spawns the `codex` CLI and streams output back to the orchestrator.
    #[cfg(feature = "docker-sandbox")]
    #[command(hide = true)]
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

    /// Run the fixed-target sandbox network relay (internal use only).
    #[cfg(feature = "docker-sandbox")]
    #[command(name = "network-relay", hide = true)]
    NetworkRelay {
        /// Forward in LISTEN_PORT=host.docker.internal:TARGET_PORT form.
        #[arg(long = "forward", required = true)]
        forwards: Vec<String>,
    },

    /// Run as a lease-scoped remote experiment runner (internal/automation use).
    #[command(hide = true)]
    ExperimentRunner {
        #[arg(long)]
        gateway_url: String,

        #[arg(
            long,
            required_unless_present = "auth_file",
            conflicts_with = "auth_file"
        )]
        auth_stdin: bool,

        #[arg(
            long,
            required_unless_present = "auth_stdin",
            conflicts_with = "auth_stdin"
        )]
        auth_file: Option<std::path::PathBuf>,

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
        matches!(
            self.command,
            None | Some(Command::Run(_)) | Some(Command::Tui(_)) | Some(Command::Ask { .. })
        ) || self.legacy_message.is_some()
    }

    pub fn resolve_runtime_args(&self) -> Result<Option<ResolvedRuntimeArgs>, CliError> {
        let (args, one_shot_message) = match &self.command {
            None => (RuntimeArgs::default(), self.legacy_message.clone()),
            Some(Command::Run(args) | Command::Tui(args)) => (args.clone(), None),
            Some(Command::Ask { text, runtime }) => (runtime.clone(), Some(text.clone())),
            Some(_) => {
                if self.legacy_cli_only || self.legacy_message.is_some() {
                    return Err(CliError::usage(
                        "--cli-only and --message are valid only for the local runtime",
                    ));
                }
                return Ok(None);
            }
        };

        if self.legacy_cli_only && args.channels.is_some() {
            return Err(CliError::usage(
                "--cli-only conflicts with the canonical --channels option",
            ));
        }

        Ok(Some(ResolvedRuntimeArgs {
            no_db: args.no_db,
            skip_setup_check: args.skip_setup_check || args.legacy_no_onboard,
            channels: if self.legacy_cli_only {
                ChannelSelectionArg::None
            } else {
                args.channels.unwrap_or(ChannelSelectionArg::Configured)
            },
            one_shot_message,
        }))
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
        assert!(matches!(cli.command, Some(Command::Tui(_))));
    }

    #[test]
    fn runtime_only_flags_are_rejected_on_status() {
        assert!(Cli::try_parse_from(["thinclaw", "status", "--no-db"]).is_err());
        assert!(Cli::try_parse_from(["thinclaw", "--no-db", "status"]).is_err());
    }

    #[test]
    fn ask_and_legacy_message_resolve_to_one_shot_runtime() {
        let ask = Cli::try_parse_from(["thinclaw", "ask", "hello"]).expect("parse ask");
        assert_eq!(
            ask.resolve_runtime_args()
                .expect("resolve ask")
                .expect("runtime")
                .one_shot_message
                .as_deref(),
            Some("hello")
        );

        let legacy = Cli::try_parse_from(["thinclaw", "-m", "hello"]).expect("parse legacy");
        assert_eq!(
            legacy
                .resolve_runtime_args()
                .expect("resolve legacy")
                .expect("runtime")
                .one_shot_message
                .as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn channel_selection_normalizes_and_deduplicates() {
        let cli = Cli::try_parse_from(["thinclaw", "run", "--channels", "discord,matrix,discord"])
            .expect("parse selected channels");
        let runtime = cli
            .resolve_runtime_args()
            .expect("resolve")
            .expect("runtime");
        assert_eq!(
            runtime.channels,
            ChannelSelectionArg::Selected(vec!["discord".into(), "matrix".into()])
        );
    }

    #[test]
    fn experiment_runner_requires_one_private_auth_source_and_has_no_secret_argv() {
        assert!(
            Cli::try_parse_from([
                "thinclaw",
                "experiment-runner",
                "--gateway-url",
                "https://gateway.example"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "thinclaw",
                "experiment-runner",
                "--gateway-url",
                "https://gateway.example",
                "--auth-stdin",
                "--auth-file",
                "/tmp/auth.json"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "thinclaw",
                "experiment-runner",
                "--gateway-url",
                "https://gateway.example",
                "--token",
                "secret"
            ])
            .is_err()
        );
    }
}
