//! Main setup wizard orchestration.

use std::{collections::BTreeMap, io::IsTerminal, sync::Arc};

use secrecy::{ExposeSecret, SecretString};

use crate::secrets::SecretsCrypto;
use crate::settings::{
    OnboardingFollowup, OnboardingFollowupCategory, OnboardingFollowupStatus, Settings,
};
use crate::setup::prompts::{
    PromptUiMode as PromptRenderMode, TuiPromptContext, clear_tui_prompt_context,
    clear_tui_prompt_messages, current_prompt_ui_mode, is_back_navigation, print_header,
    print_info, print_phase_banner, print_step, print_success, print_warning, push_prompt_ui_mode,
    select_one, set_tui_prompt_context,
};
use crate::terminal_branding::set_runtime_cli_skin_override;

pub use self::contracts::{
    FollowupDraft, GuideTopic, OnboardingProfile, ReadinessSummary, StepDescriptor, StepStatus,
    UiMode, ValidationItem, ValidationLevel, WizardPhase, WizardPhaseId, WizardPlan, WizardStepId,
};

/// Setup wizard error.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Channel setup error: {0}")]
    Channel(String),

    #[error("User cancelled")]
    Cancelled,
}

impl From<crate::setup::channels::ChannelSetupError> for SetupError {
    fn from(e: crate::setup::channels::ChannelSetupError) -> Self {
        SetupError::Channel(e.to_string())
    }
}

/// Setup wizard configuration.
#[derive(Debug, Clone)]
pub struct SetupConfig {
    /// Skip authentication step (use existing session).
    pub skip_auth: bool,
    /// Only reconfigure channels.
    pub channels_only: bool,
    /// Preferred onboarding UI mode.
    pub ui_mode: UiMode,
    /// Optional guided settings topic.
    pub guide_topic: Option<GuideTopic>,
    /// Optional profile supplied by the CLI.
    pub profile: Option<OnboardingProfile>,
    /// When true, save settings and return without continuing into runtime.
    pub pause_after_completion: bool,
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            skip_auth: false,
            channels_only: false,
            ui_mode: UiMode::Auto,
            guide_topic: None,
            profile: None,
            pause_after_completion: false,
        }
    }
}

/// Interactive setup wizard for ThinClaw.
pub struct SetupWizard {
    config: SetupConfig,
    settings: Settings,

    /// Database pool (created during setup, postgres only).
    #[cfg(feature = "postgres")]
    db_pool: Option<deadpool_postgres::Pool>,
    /// libSQL backend (created during setup, libsql only).
    #[cfg(feature = "libsql")]
    db_backend: Option<crate::db::libsql::LibSqlBackend>,
    /// Secrets crypto (created during setup).
    secrets_crypto: Option<Arc<SecretsCrypto>>,
    /// Cached API key from provider setup (used by model fetcher without env mutation).
    llm_api_key: Option<SecretString>,
    /// Selected onboarding profile for the current run.
    selected_profile: OnboardingProfile,
    /// Shared step/phase plan for the current onboarding run.
    plan: Option<WizardPlan>,
    /// Live execution status per step for progress UIs.
    step_statuses: BTreeMap<WizardStepId, StepStatus>,
    /// Ephemeral follow-up tasks collected during the current run.
    followups: Vec<FollowupDraft>,
    /// Latest non-destructive channel verification readiness map.
    verified_channels: BTreeMap<String, bool>,
    /// Quick-setup primary channel selection for notification defaults.
    quick_primary_channel: Option<String>,
    /// Generated env-backed secrets master key when no secure store is available.
    generated_env_master_key: Option<String>,
    /// Actual prompt/runtime mode chosen for this onboarding run.
    resolved_ui_mode: UiMode,
}

#[derive(Clone)]
struct WizardCheckpoint {
    settings: Settings,
    #[cfg(feature = "postgres")]
    db_pool: Option<deadpool_postgres::Pool>,
    #[cfg(feature = "libsql")]
    db_backend: Option<crate::db::libsql::LibSqlBackend>,
    secrets_crypto: Option<Arc<SecretsCrypto>>,
    llm_api_key: Option<SecretString>,
    selected_profile: OnboardingProfile,
    step_statuses: BTreeMap<WizardStepId, StepStatus>,
    followups: Vec<FollowupDraft>,
    verified_channels: BTreeMap<String, bool>,
    quick_primary_channel: Option<String>,
    generated_env_master_key: Option<String>,
}

impl SetupWizard {
    /// Create a new setup wizard.
    pub fn new() -> Self {
        Self {
            config: SetupConfig::default(),
            settings: Settings::default(),
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
            selected_profile: OnboardingProfile::default(),
            plan: None,
            step_statuses: BTreeMap::new(),
            followups: Vec::new(),
            verified_channels: BTreeMap::new(),
            quick_primary_channel: None,
            generated_env_master_key: None,
            resolved_ui_mode: UiMode::Cli,
        }
    }

    /// Create a wizard with custom configuration.
    pub fn with_config(config: SetupConfig) -> Self {
        let selected_profile = config.profile.unwrap_or_default();
        Self {
            config,
            settings: Settings::default(),
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
            selected_profile,
            plan: None,
            step_statuses: BTreeMap::new(),
            followups: Vec::new(),
            verified_channels: BTreeMap::new(),
            quick_primary_channel: None,
            generated_env_master_key: None,
            resolved_ui_mode: UiMode::Cli,
        }
    }

    fn resolve_ui_mode(&self) -> UiMode {
        match self.config.ui_mode {
            UiMode::Auto => {
                let stdin_is_tty = std::io::stdin().is_terminal();
                let stdout_is_tty = std::io::stdout().is_terminal();
                let fits_shell = crossterm::terminal::size()
                    .map(|(width, height)| width >= 100 && height >= 28)
                    .unwrap_or(false);
                if stdin_is_tty && stdout_is_tty && fits_shell {
                    UiMode::Tui
                } else {
                    UiMode::Cli
                }
            }
            mode => mode,
        }
    }

    fn is_guide_mode(&self) -> bool {
        self.config.guide_topic.is_some()
    }

    fn is_quick_setup(&self) -> bool {
        !self.config.channels_only && !self.is_guide_mode()
    }

    pub fn runtime_ui_mode(&self) -> UiMode {
        match self.resolved_ui_mode {
            UiMode::Cli | UiMode::Tui => self.resolved_ui_mode,
            UiMode::Auto => UiMode::Cli,
        }
    }

    pub fn should_continue_to_runtime(&self) -> bool {
        !self.config.pause_after_completion
    }

    pub(super) fn primary_runtime_command(&self) -> &'static str {
        if matches!(self.selected_profile, OnboardingProfile::RemoteServer) {
            return "thinclaw run --no-onboard";
        }
        match self.runtime_ui_mode() {
            UiMode::Tui => "thinclaw tui",
            UiMode::Cli | UiMode::Auto => "thinclaw",
        }
    }

    pub(super) fn runtime_handoff_summary(&self) -> String {
        if matches!(self.selected_profile, OnboardingProfile::RemoteServer) {
            return if self.should_continue_to_runtime() {
                "ThinClaw will now continue into `thinclaw run --no-onboard` with the service-safe remote runtime settings from this run.".to_string()
            } else {
                "Settings are saved. Start the remote runtime with `thinclaw run --no-onboard` or install/start the OS service.".to_string()
            };
        }
        if self.should_continue_to_runtime() {
            format!(
                "ThinClaw will now continue into `{}` using the settings from this run.",
                self.primary_runtime_command()
            )
        } else {
            "Settings are saved. This pass stops here so you can launch runtime later on your own."
                .to_string()
        }
    }

    pub(super) fn what_next_commands(&self) -> Vec<String> {
        if matches!(self.selected_profile, OnboardingProfile::RemoteServer) {
            return vec![
                "Service-safe runtime: thinclaw run --no-onboard".to_string(),
                "Install OS service: thinclaw service install".to_string(),
                "Start OS service: thinclaw service start".to_string(),
                "Show WebUI access: thinclaw gateway access".to_string(),
                "Show full token URL: thinclaw gateway access --show-token".to_string(),
                "Remote diagnostics: thinclaw doctor --profile remote".to_string(),
                "Reopen remote onboarding: thinclaw onboard --profile remote".to_string(),
            ];
        }
        let mut commands = vec![
            format!("Primary runtime: {}", self.primary_runtime_command()),
            "Standard CLI runtime: thinclaw".to_string(),
            "Full-screen TUI runtime: thinclaw tui".to_string(),
            "Reopen onboarding: thinclaw onboard".to_string(),
            "Revisit channels only: thinclaw onboard --channels-only".to_string(),
        ];

        if self.config.pause_after_completion {
            commands.push("Topic guide: thinclaw onboard --guide".to_string());
        }

        commands
    }

    fn build_plan(&self) -> WizardPlan {
        use WizardPhaseId as Phase;
        use WizardStepId as Step;

        let mut phases = Vec::new();
        let mut steps = Vec::new();

        let mut push_step = |id, phase_id, title, description, why_this_matters, recommended| {
            steps.push(StepDescriptor {
                id,
                phase_id,
                title,
                description,
                why_this_matters,
                recommended,
            });
        };

        if self.config.channels_only {
            phases.push(WizardPhase {
                id: Phase::ChannelsContinuity,
                step_ids: vec![Step::Channels, Step::ChannelVerification],
            });
            phases.push(WizardPhase {
                id: Phase::Finish,
                step_ids: vec![Step::Summary],
            });

            push_step(
                Step::Channels,
                Phase::ChannelsContinuity,
                "Channel Configuration",
                "Choose where ThinClaw should receive and send messages.",
                "A working channel is what turns configuration into a usable agent.",
                Some("Start with one channel you can verify right now."),
            );
            push_step(
                Step::ChannelVerification,
                Phase::ChannelsContinuity,
                "Channel Verification",
                "Run safe checks against every enabled channel and capture any gaps.",
                "It is better to leave with one confirmed route than several unverified ones.",
                Some("Treat verification as the release gate for channel setup."),
            );
            push_step(
                Step::Summary,
                Phase::Finish,
                "Finish",
                "Review readiness, deferred tasks, and what happens next.",
                "The goal is a confident handoff into normal startup, not more guesswork.",
                None,
            );

            return WizardPlan { phases, steps };
        }

        if let Some(topic) = self.config.guide_topic {
            let step_ids = match topic {
                GuideTopic::Menu => vec![Step::Summary],
                GuideTopic::Ai => vec![
                    Step::InferenceProvider,
                    Step::ModelSelection,
                    Step::SmartRouting,
                    Step::FallbackProviders,
                    Step::Embeddings,
                ],
                GuideTopic::Channels => vec![
                    Step::Channels,
                    Step::ChannelContinuity,
                    Step::ChannelVerification,
                    Step::Notifications,
                ],
                GuideTopic::Agent => {
                    vec![
                        Step::CliSkin,
                        Step::AgentIdentity,
                        Step::Timezone,
                        Step::WebUi,
                    ]
                }
                GuideTopic::Tools => vec![
                    Step::ToolApproval,
                    Step::DockerSandbox,
                    Step::Extensions,
                    Step::ClaudeCode,
                    Step::CodexCode,
                ],
                GuideTopic::Automation => vec![Step::Routines, Step::Skills, Step::Heartbeat],
                GuideTopic::Runtime => {
                    vec![Step::Database, Step::Security, Step::Observability]
                }
            };

            let phase_id = match topic {
                GuideTopic::Menu => Phase::Finish,
                GuideTopic::Ai => Phase::AiStack,
                GuideTopic::Channels => Phase::ChannelsContinuity,
                GuideTopic::Agent => Phase::WelcomeProfile,
                GuideTopic::Tools => Phase::CapabilitiesAutomation,
                GuideTopic::Automation => Phase::CapabilitiesAutomation,
                GuideTopic::Runtime => Phase::CoreRuntime,
            };
            phases.push(WizardPhase {
                id: phase_id,
                step_ids,
            });
            phases.push(WizardPhase {
                id: Phase::Finish,
                step_ids: vec![Step::Summary],
            });
        } else {
            phases.push(WizardPhase {
                id: Phase::WelcomeProfile,
                step_ids: vec![Step::CliSkin, Step::Profile, Step::AgentIdentity],
            });
            phases.push(WizardPhase {
                id: Phase::AiStack,
                step_ids: vec![Step::InferenceProvider, Step::ModelSelection],
            });
            phases.push(WizardPhase {
                id: Phase::ChannelsContinuity,
                step_ids: vec![Step::Channels, Step::ChannelVerification],
            });
            phases.push(WizardPhase {
                id: Phase::CapabilitiesAutomation,
                step_ids: vec![Step::ToolApproval, Step::DockerSandbox, Step::CodingWorkers],
            });
            phases.push(WizardPhase {
                id: Phase::ExperienceOperations,
                step_ids: vec![Step::WebUi],
            });
            phases.push(WizardPhase {
                id: Phase::Finish,
                step_ids: vec![Step::Summary],
            });
        }

        push_step(
            Step::CliSkin,
            Phase::WelcomeProfile,
            "Choose Your Cockpit Skin",
            "Pick the skin you want onboarding, the CLI, and the default web experience to use.",
            "The first visual choice sets the tone for the whole operator experience.",
            Some("Pick the one that feels easiest to read for a long session."),
        );
        push_step(
            Step::Profile,
            Phase::WelcomeProfile,
            "Choose Your Setup Lane",
            "Pick a profile to prefill practical defaults for your environment.",
            "Profiles speed up setup without taking away your ability to review each section.",
            Some("Balanced is the best default for most operators."),
        );
        push_step(
            Step::Database,
            Phase::CoreRuntime,
            "Storage Foundation",
            "Review where ThinClaw stores settings, history, and runtime state.",
            "This storage path underpins everything else in onboarding.",
            Some("libSQL + local file is the fastest reliable path for day one."),
        );
        push_step(
            Step::Security,
            Phase::CoreRuntime,
            "Secret Protection",
            "Review how API keys and sensitive values are protected.",
            "Trust boundaries should be explicit before provider credentials are stored.",
            Some("Use your OS secure store when available."),
        );
        push_step(
            Step::InferenceProvider,
            Phase::AiStack,
            "Primary Model Provider",
            "Choose the provider ThinClaw should rely on for its primary advisor model.",
            "This choice impacts quality, latency, auth, and operating cost.",
            Some("Start with one reliable provider, then add fallback later."),
        );
        push_step(
            Step::ModelSelection,
            Phase::AiStack,
            "Advisor Model (Primary)",
            "Choose the stronger primary model used for strategic guidance and high-quality reasoning.",
            "This model defines the quality ceiling for everyday operation.",
            None,
        );
        push_step(
            Step::SmartRouting,
            Phase::AiStack,
            "Executor Model (Fast)",
            "Choose the fast execution model used in advisor/executor routing.",
            "A strong executor keeps everyday work responsive while the advisor stays available for escalation.",
            Some("Pick a fast model that lives next to your primary provider when possible."),
        );
        push_step(
            Step::FallbackProviders,
            Phase::AiStack,
            "Resilience Fallbacks",
            "Add secondary providers so routing can recover when the primary path is unavailable.",
            "Fallbacks improve uptime and reduce single-provider risk.",
            Some("Add one fallback after the primary route is confirmed."),
        );
        push_step(
            Step::Embeddings,
            Phase::AiStack,
            "Memory & Semantic Search",
            "Configure embeddings so ThinClaw can search memory semantically.",
            "Good embeddings improve recall quality and reduce repetitive prompting.",
            Some("Enable this once local or remote embeddings are actually reachable."),
        );
        push_step(
            Step::AgentIdentity,
            Phase::IdentityPresence,
            "Agent Name & Personality",
            "Set the agent name and the personality pack that seeds the canonical home soul.",
            "Identity details shape trust and consistency across channels.",
            None,
        );
        push_step(
            Step::Timezone,
            Phase::IdentityPresence,
            "Timezone",
            "Confirm the local timezone for schedules and time-aware logic.",
            "Timezone errors cause confusing routine timing and alert windows.",
            None,
        );
        push_step(
            Step::Channels,
            Phase::ChannelsContinuity,
            "Primary Channel",
            "Choose the main channel users should use to reach ThinClaw and configure only what is needed for that path.",
            "Channels are the interface where users will actually meet the agent.",
            Some("Pick only channels you can verify today."),
        );
        push_step(
            Step::ChannelContinuity,
            Phase::ChannelsContinuity,
            "Cross-Channel Session Continuity",
            "Review how direct sessions synchronize across channels and devices.",
            "Understanding continuity prevents confusion when conversations move channels.",
            None,
        );
        push_step(
            Step::ChannelVerification,
            Phase::ChannelsContinuity,
            "Channel Verification",
            "Run non-destructive checks for the selected channel and capture any follow-ups.",
            "Known gaps are manageable; hidden gaps break trust in production.",
            Some("Leave onboarding with at least one fully verified path."),
        );
        push_step(
            Step::Notifications,
            Phase::ChannelsContinuity,
            "Notification Preferences",
            "Choose where proactive alerts and routine results should be delivered.",
            "Useful automation depends on a destination users actually watch.",
            Some("Pick a verified channel whenever possible."),
        );
        push_step(
            Step::Extensions,
            Phase::CapabilitiesAutomation,
            "Tools & Extensions",
            "Select capability bundles and optional tools from the registry.",
            "Tooling determines what ThinClaw can do beyond chat responses.",
            Some("Use the Balanced bundle unless you need strict minimalism."),
        );
        push_step(
            Step::ToolApproval,
            Phase::CapabilitiesAutomation,
            "Autonomy Level",
            "Choose how much local autonomy ThinClaw has when running tools on your machine.",
            "Autonomy level defines the default operator trust posture on day one.",
            Some("Standard keeps approvals on. Autonomous and Full Autonomous enable local tools."),
        );
        push_step(
            Step::DockerSandbox,
            Phase::CapabilitiesAutomation,
            "Worker Sandbox",
            "Decide whether ThinClaw should isolate worker processes such as coding delegates in Docker.",
            "Early boundary choices reduce surprise and security drift later.",
            Some(
                "Keep the worker sandbox on unless you already know you do not want container isolation.",
            ),
        );
        push_step(
            Step::CodingWorkers,
            Phase::CapabilitiesAutomation,
            "Coding Workers",
            "Optionally enable Claude Code and Codex after the sandbox is configured.",
            "Coding workers add power, but only matter if you want delegated coding help right away.",
            Some("Leave them off unless you already know you want coding delegates today."),
        );
        push_step(
            Step::ClaudeCode,
            Phase::CapabilitiesAutomation,
            "Claude Code Sandbox",
            "Configure optional Claude Code worker integration.",
            "Only required if your workflow depends on Claude sandbox execution.",
            None,
        );
        push_step(
            Step::CodexCode,
            Phase::CapabilitiesAutomation,
            "Codex Sandbox",
            "Configure optional Codex CLI worker integration.",
            "Only required if your workflow depends on Codex sandbox execution.",
            None,
        );
        push_step(
            Step::Routines,
            Phase::CapabilitiesAutomation,
            "Routines",
            "Enable or defer scheduled automation tasks.",
            "Routines are optional at launch but powerful once channels are stable.",
            None,
        );
        push_step(
            Step::Skills,
            Phase::CapabilitiesAutomation,
            "Skills",
            "Enable reusable capability packs for specialized behavior.",
            "Skills increase adaptability without modifying core runtime code.",
            Some("Keep skills enabled unless you need a minimal locked-down install."),
        );
        push_step(
            Step::Heartbeat,
            Phase::CapabilitiesAutomation,
            "Background Tasks",
            "Choose whether ThinClaw runs periodic background heartbeat tasks.",
            "Heartbeat adds value after alerts and channels are fully configured.",
            Some("Keep it off on day one unless notification delivery is already verified."),
        );
        push_step(
            Step::WebUi,
            Phase::ExperienceOperations,
            "Web UI",
            "Tune the operator-facing dashboard experience.",
            "Clear UI defaults reduce operator friction and support load.",
            None,
        );
        push_step(
            Step::Observability,
            Phase::ExperienceOperations,
            "Observability",
            "Decide how much runtime telemetry and diagnostics should be emitted.",
            "Good visibility helps debugging without flooding operators with noise.",
            Some("Start lean, then raise observability once core flows are stable."),
        );
        push_step(
            Step::Summary,
            Phase::Finish,
            "Finish",
            "Review readiness, deferred tasks, and the bootstrap handoff into normal startup.",
            "A strong finish gives operators confidence to launch immediately.",
            None,
        );

        let mut filtered_steps = Vec::new();
        let allowed_ids: std::collections::BTreeSet<_> = phases
            .iter()
            .flat_map(|phase| phase.step_ids.iter().copied())
            .collect();
        for descriptor in steps {
            if allowed_ids.contains(&descriptor.id) {
                filtered_steps.push(descriptor);
            }
        }

        WizardPlan {
            phases,
            steps: filtered_steps,
        }
    }

    fn reset_plan_state(&mut self) {
        self.plan = Some(self.build_plan());
        self.step_statuses.clear();
        self.followups.clear();
        self.verified_channels.clear();
        self.quick_primary_channel = None;
        self.settings.onboarding_followups.clear();
        if let Some(plan) = &self.plan {
            for step in &plan.steps {
                self.step_statuses.insert(step.id, StepStatus::Pending);
            }
        }
    }

    fn persist_followups(&mut self) {
        self.settings.onboarding_followups = self
            .followups
            .iter()
            .map(|item| OnboardingFollowup {
                id: item.id.clone(),
                title: item.title.clone(),
                category: item.category,
                status: item.status,
                instructions: item.instructions.clone(),
                action_hint: item.action_hint.clone(),
            })
            .collect();
    }

    fn checkpoint(&self) -> WizardCheckpoint {
        WizardCheckpoint {
            settings: self.settings.clone(),
            #[cfg(feature = "postgres")]
            db_pool: self.db_pool.clone(),
            #[cfg(feature = "libsql")]
            db_backend: self.db_backend.clone(),
            secrets_crypto: self.secrets_crypto.clone(),
            llm_api_key: self.llm_api_key.clone(),
            selected_profile: self.selected_profile,
            step_statuses: self.step_statuses.clone(),
            followups: self.followups.clone(),
            verified_channels: self.verified_channels.clone(),
            quick_primary_channel: self.quick_primary_channel.clone(),
            generated_env_master_key: self.generated_env_master_key.clone(),
        }
    }

    fn restore_checkpoint(&mut self, checkpoint: WizardCheckpoint) {
        self.settings = checkpoint.settings;
        #[cfg(feature = "postgres")]
        {
            self.db_pool = checkpoint.db_pool;
        }
        #[cfg(feature = "libsql")]
        {
            self.db_backend = checkpoint.db_backend;
        }
        self.secrets_crypto = checkpoint.secrets_crypto;
        self.llm_api_key = checkpoint.llm_api_key;
        self.selected_profile = checkpoint.selected_profile;
        self.step_statuses = checkpoint.step_statuses;
        self.followups = checkpoint.followups;
        self.verified_channels = checkpoint.verified_channels;
        self.quick_primary_channel = checkpoint.quick_primary_channel;
        self.generated_env_master_key = checkpoint.generated_env_master_key;
    }

    async fn run_cli_flow(&mut self) -> Result<(), SetupError> {
        let _prompt_mode = push_prompt_ui_mode(PromptRenderMode::Cli);
        set_runtime_cli_skin_override(self.settings.agent.cli_skin.clone());
        let mode_label = match current_prompt_ui_mode() {
            PromptRenderMode::Cli => "cli",
            PromptRenderMode::Tui => "tui",
        };
        print_header("ThinClaw Humanist Cockpit");
        print_info(
            "You will move through focused phases with calm recommendations and inline readiness checks.",
        );
        print_info(
            "Progress is saved as you go, so you can pause and resume without redoing the stable parts.",
        );
        print_info(&format!("Cockpit mode: {mode_label}"));
        crate::setup::prompts::print_blank_line();
        self.run_planned_flow(None).await
    }

    async fn run_tui_flow(&mut self) -> Result<(), SetupError> {
        let _prompt_mode = push_prompt_ui_mode(PromptRenderMode::Tui);
        set_runtime_cli_skin_override(self.settings.agent.cli_skin.clone());
        clear_tui_prompt_messages();
        clear_tui_prompt_context();
        let plan = self
            .plan
            .clone()
            .ok_or_else(|| SetupError::Config("Onboarding plan was not initialized".to_string()))?;
        let mut shell = tui_shell::OnboardingTuiShell::new(plan);
        self.run_planned_flow(Some(&mut shell)).await?;
        shell.show_completion(self)?;
        clear_tui_prompt_context();
        Ok(())
    }

    async fn run_planned_flow(
        &mut self,
        shell: Option<&mut tui_shell::OnboardingTuiShell>,
    ) -> Result<(), SetupError> {
        let mut plan = self
            .plan
            .clone()
            .ok_or_else(|| SetupError::Config("Onboarding plan was not initialized".to_string()))?;
        let mut total_steps = plan.total_steps();
        let mut last_phase = None;
        let mut index = 0usize;

        while index < total_steps {
            let descriptor = plan.steps[index].clone();
            if shell.is_some() {
                clear_tui_prompt_messages();
                set_tui_prompt_context(TuiPromptContext {
                    phase_title: Some(descriptor.phase_id.title().to_string()),
                    phase_description: Some(descriptor.phase_id.description().to_string()),
                    step_progress: Some(format!("Step {}/{}", index + 1, total_steps)),
                    description: Some(descriptor.description.to_string()),
                    why_this_matters: Some(descriptor.why_this_matters.to_string()),
                    recommended: descriptor.recommended.map(ToOwned::to_owned),
                });
            }
            if shell.is_none() && last_phase != Some(descriptor.phase_id) {
                print_phase_banner(
                    descriptor.phase_id.title(),
                    descriptor.phase_id.description(),
                );
            }
            last_phase = Some(descriptor.phase_id);

            let checkpoint = self.checkpoint();
            self.step_statuses
                .insert(descriptor.id, StepStatus::InProgress);

            if shell.is_none() {
                print_step(index + 1, total_steps, descriptor.title);
                print_info(descriptor.description);
                print_info(descriptor.why_this_matters);
                if let Some(recommended) = descriptor.recommended {
                    print_success(&format!("Recommended: {}", recommended));
                }
                crate::setup::prompts::print_blank_line();
            }

            let status = match self.execute_step(descriptor.id).await {
                Ok(status) => status,
                Err(SetupError::Io(error)) if shell.is_some() && is_back_navigation(&error) => {
                    self.restore_checkpoint(checkpoint);
                    let rerouted = if self.should_reopen_tui_menus_on_back(index, shell.is_some()) {
                        if self.is_guide_mode() {
                            self.reroute_tui_back_from_first_guided_step().await?
                        } else {
                            self.reroute_tui_back_from_first_quick_step().await?
                        }
                    } else {
                        false
                    };
                    if rerouted {
                        plan = self.plan.clone().ok_or_else(|| {
                            SetupError::Config(
                                "Onboarding plan was not initialized after back navigation"
                                    .to_string(),
                            )
                        })?;
                        total_steps = plan.total_steps();
                        index = 0;
                        last_phase = None;
                        continue;
                    }
                    index = index.saturating_sub(1);
                    last_phase = None;
                    continue;
                }
                Err(error) => return Err(error),
            };
            self.step_statuses.insert(descriptor.id, status);
            self.persist_followups();

            if self.should_persist_step(descriptor.id) {
                self.persist_after_step().await;
            }

            if shell.is_some() {
                match status {
                    StepStatus::NeedsAttention => {
                        print_warning("This step left follow-up work queued for later.");
                    }
                    StepStatus::Skipped => {
                        print_info("This step was skipped for now.");
                    }
                    _ => {}
                }
            }

            index += 1;
        }

        Ok(())
    }

    fn should_reopen_tui_menus_on_back(&self, index: usize, has_shell: bool) -> bool {
        has_shell && index == 0 && !self.config.channels_only
    }

    async fn reroute_tui_back_from_first_guided_step(&mut self) -> Result<bool, SetupError> {
        let original_guide_topic = self.config.guide_topic;
        if !self.is_guide_mode() || !self.should_reopen_tui_menus_on_back(0, true) {
            return Ok(false);
        }

        self.config.guide_topic = Some(GuideTopic::Menu);

        loop {
            match self.resolve_guide_topic_prompt() {
                Ok(()) => break,
                Err(SetupError::Io(error)) if is_back_navigation(&error) => {
                    self.config.guide_topic = None;
                    match self.resolve_setup_entry_prompt(self.resolved_ui_mode) {
                        Ok(()) => {
                            if self.config.guide_topic == Some(GuideTopic::Menu) {
                                continue;
                            }
                            break;
                        }
                        Err(SetupError::Io(error)) if is_back_navigation(&error) => {
                            self.config.guide_topic = original_guide_topic;
                            return Ok(false);
                        }
                        Err(error) => {
                            self.config.guide_topic = original_guide_topic;
                            return Err(error);
                        }
                    }
                }
                Err(error) => {
                    self.config.guide_topic = original_guide_topic;
                    return Err(error);
                }
            }
        }

        self.apply_run_mode_defaults().await?;
        self.reset_plan_state();
        Ok(true)
    }

    /// Re-show the Quick vs. Advanced entry prompt when the user presses
    /// Ctrl+B on the very first plan step in quick-setup mode.
    async fn reroute_tui_back_from_first_quick_step(&mut self) -> Result<bool, SetupError> {
        if self.is_guide_mode() || !self.should_reopen_tui_menus_on_back(0, true) {
            return Ok(false);
        }

        // Re-present the "Quick Setup vs. Advanced Setup" entry prompt.
        // If the user presses Ctrl+B again at that prompt, treat it as a
        // no-op (stay on current plan) since there is nothing further back.
        match self.resolve_setup_entry_prompt(self.resolved_ui_mode) {
            Ok(()) => {
                // User may have switched to Advanced, which sets guide_topic.
                // If they picked Advanced, show the topic menu too.
                if self.config.guide_topic == Some(GuideTopic::Menu) {
                    self.resolve_guide_topic_prompt()?;
                }
            }
            Err(SetupError::Io(error)) if is_back_navigation(&error) => {
                // Nowhere further back to go — stay on the current plan.
                return Ok(false);
            }
            Err(error) => return Err(error),
        }

        self.apply_run_mode_defaults().await?;
        self.reset_plan_state();
        Ok(true)
    }

    fn should_persist_step(&self, step_id: WizardStepId) -> bool {
        !matches!(
            step_id,
            WizardStepId::Profile | WizardStepId::ChannelContinuity | WizardStepId::Summary
        )
    }

    async fn execute_step(&mut self, step_id: WizardStepId) -> Result<StepStatus, SetupError> {
        match step_id {
            WizardStepId::CliSkin => {
                self.step_cli_skin()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Profile => {
                self.step_profile()?;
                self.apply_profile_defaults();
                Ok(StepStatus::Completed)
            }
            WizardStepId::Database => {
                self.step_database().await?;
                let step1_settings = self.settings.clone();
                self.try_load_existing_settings().await;
                self.settings.merge_from(&step1_settings);
                self.apply_profile_defaults();
                Ok(StepStatus::Completed)
            }
            WizardStepId::Security => {
                self.step_security().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::InferenceProvider => {
                if self.config.skip_auth {
                    self.step_provider_review_skip_auth().await?;
                } else {
                    self.step_inference_provider().await?;
                }
                Ok(StepStatus::Completed)
            }
            WizardStepId::ModelSelection => {
                self.step_model_selection().await?;
                if self.is_quick_setup() {
                    self.apply_quick_embeddings_defaults();
                    self.step_smart_routing().await?;
                }
                Ok(StepStatus::Completed)
            }
            WizardStepId::SmartRouting => {
                self.step_smart_routing().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::FallbackProviders => {
                self.step_fallback_providers().await?;
                if self.config.skip_auth {
                    self.ensure_remote_provider_followup();
                } else {
                    self.ensure_onboarding_provider_api_key().await?;
                }
                Ok(StepStatus::Completed)
            }
            WizardStepId::Embeddings => {
                self.step_embeddings()?;
                Ok(if self.settings.embeddings.enabled {
                    StepStatus::Completed
                } else {
                    StepStatus::NeedsAttention
                })
            }
            WizardStepId::AgentIdentity => {
                self.step_agent_identity()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Timezone => {
                self.step_timezone()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Channels => {
                if self.config.channels_only {
                    self.reconnect_existing_db().await?;
                }
                if self.is_quick_setup() {
                    self.step_primary_channel_quick().await?;
                } else {
                    self.step_channels().await?;
                }
                Ok(StepStatus::Completed)
            }
            WizardStepId::ChannelContinuity => {
                self.step_channel_continuity()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::ChannelVerification => {
                let issues = self.step_channel_verification().await?;
                if self.is_quick_setup() {
                    self.apply_quick_notification_defaults();
                }
                Ok(if issues == 0 {
                    StepStatus::Completed
                } else {
                    StepStatus::NeedsAttention
                })
            }
            WizardStepId::Notifications => {
                self.step_notification_preferences().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Extensions => {
                self.step_extensions().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::DockerSandbox => {
                self.step_docker_sandbox().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::CodingWorkers => {
                self.step_coding_workers().await?;
                Ok(
                    if self.settings.claude_code_enabled || self.settings.codex_code_enabled {
                        StepStatus::Completed
                    } else {
                        StepStatus::Skipped
                    },
                )
            }
            WizardStepId::ClaudeCode => {
                self.step_claude_code().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::CodexCode => {
                self.step_codex_code().await?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::ToolApproval => {
                self.step_tool_approval()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Routines => {
                self.step_routines()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Skills => {
                self.step_skills()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Heartbeat => {
                self.step_heartbeat()?;
                Ok(if self.settings.heartbeat.enabled {
                    StepStatus::Completed
                } else {
                    StepStatus::Skipped
                })
            }
            WizardStepId::WebUi => {
                self.step_web_ui()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Observability => {
                self.step_observability()?;
                Ok(StepStatus::Completed)
            }
            WizardStepId::Summary => {
                self.save_and_summarize().await?;
                Ok(if self.followups.is_empty() {
                    StepStatus::Completed
                } else {
                    StepStatus::NeedsAttention
                })
            }
        }
    }

    fn readiness_summary(&self) -> ReadinessSummary {
        let ready_now = self
            .step_statuses
            .values()
            .filter(|status| matches!(status, StepStatus::Completed))
            .count();
        let needs_attention = self
            .step_statuses
            .values()
            .filter(|status| matches!(status, StepStatus::NeedsAttention))
            .count();
        let followups = self.followups.len();

        let headline = if followups == 0 && needs_attention == 0 {
            "Launch-ready".to_string()
        } else if followups > 0 || needs_attention > 0 {
            "Attention queued".to_string()
        } else {
            "Bringing systems online".to_string()
        };

        ReadinessSummary {
            ready_now,
            needs_attention,
            followups,
            headline,
        }
    }

    fn validation_items(&self) -> Vec<ValidationItem> {
        let mut items = Vec::new();

        if self.settings.database_backend.is_some() {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "Core runtime".to_string(),
                detail: self
                    .settings
                    .database_backend
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Error,
                title: "Core runtime".to_string(),
                detail: "Storage still needs to be configured before ThinClaw can fully launch."
                    .to_string(),
            });
        }

        if self.settings.llm_backend.is_some() && self.settings.selected_model.is_some() {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "AI stack".to_string(),
                detail: format!(
                    "{} / {}",
                    self.settings.llm_backend.as_deref().unwrap_or("unknown"),
                    self.settings
                        .selected_model
                        .as_deref()
                        .unwrap_or("unselected")
                ),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Error,
                title: "AI stack".to_string(),
                detail:
                    "Primary provider or model still needs review before the agent is fully ready."
                        .to_string(),
            });
        }

        let enabled_channels = self.configured_channel_names();
        if enabled_channels.is_empty() {
            items.push(ValidationItem {
                level: ValidationLevel::Warning,
                title: "Channels".to_string(),
                detail: "Only the built-in terminal path is confirmed right now.".to_string(),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "Channels".to_string(),
                detail: enabled_channels.join(", "),
            });
        }

        if !self.followups.is_empty() {
            items.push(ValidationItem {
                level: ValidationLevel::Warning,
                title: "Follow-ups".to_string(),
                detail: format!(
                    "{} follow-up item(s) are queued for after launch.",
                    self.followups.len()
                ),
            });
        }

        items
    }

    fn add_followup(&mut self, draft: FollowupDraft) {
        if let Some(existing) = self.followups.iter_mut().find(|item| item.id == draft.id) {
            *existing = draft;
        } else {
            self.followups.push(draft);
        }
    }

    fn remove_followup(&mut self, id: &str) {
        self.followups.retain(|item| item.id != id);
    }

    fn resolve_guide_topic_prompt(&mut self) -> Result<(), SetupError> {
        if self.config.guide_topic != Some(GuideTopic::Menu) {
            return Ok(());
        }

        print_info("Choose the settings topic you want to revisit.");
        let options = [
            "AI & Models",
            "Channels & Notifications",
            "Agent & Experience",
            "Tools & Safety",
            "Automation & Skills",
            "Runtime & Diagnostics",
        ];
        let choice = select_one("Guided settings topic", &options).map_err(SetupError::Io)?;
        self.config.guide_topic = Some(match choice {
            1 => GuideTopic::Channels,
            2 => GuideTopic::Agent,
            3 => GuideTopic::Tools,
            4 => GuideTopic::Automation,
            5 => GuideTopic::Runtime,
            _ => GuideTopic::Ai,
        });
        Ok(())
    }

    fn resolve_setup_entry_prompt(&mut self, ui_mode: UiMode) -> Result<(), SetupError> {
        if !matches!(ui_mode, UiMode::Cli | UiMode::Tui)
            || self.config.channels_only
            || self.config.guide_topic.is_some()
            || self.config.profile.is_some()
        {
            return Ok(());
        }

        print_info(
            "Choose whether you want the streamlined default path or the guided advanced lane.",
        );
        let options = [
            "Quick Setup     - reduced day-one path that gets ThinClaw running fast",
            "Advanced Setup  - choose a topic and tune more deeply before launch",
        ];
        let choice = select_one("Setup mode", &options).map_err(SetupError::Io)?;
        if choice == 1 {
            self.config.guide_topic = Some(GuideTopic::Menu);
            print_info(
                "Advanced Setup keeps Quick Setup short and opens the guided topic menu instead.",
            );
        } else {
            print_success("Quick Setup selected. ThinClaw will stay on the reduced day-one path.");
        }

        Ok(())
    }

    async fn prepare_run_mode(&mut self, ui_mode: UiMode) -> Result<(), SetupError> {
        self.resolve_setup_entry_prompt(ui_mode)?;
        self.resolve_guide_topic_prompt()?;
        self.apply_run_mode_defaults().await?;

        Ok(())
    }

    async fn apply_run_mode_defaults(&mut self) -> Result<(), SetupError> {
        if let Some(profile) = self.config.profile {
            self.selected_profile = profile;
            self.apply_profile_defaults();
        }

        if self.is_quick_setup() {
            self.auto_configure_quick_runtime_defaults().await?;
        } else if self.config.channels_only || self.is_guide_mode() {
            let _ = self.reconnect_existing_db().await;
        }

        Ok(())
    }

    fn step_profile(&mut self) -> Result<(), SetupError> {
        if let Some(profile) = self.config.profile {
            self.selected_profile = profile;
            print_success(&format!(
                "Using the {} profile from --profile.",
                self.selected_profile.title()
            ));
            print_info(self.selected_profile.description());
            return Ok(());
        }

        let options = [
            "Balanced            - calm defaults for most first runs",
            "Local & Private     - prefer local models and fewer external services",
            "Builder & Coding    - bias for tools, coding, and stronger routing",
            "Channel-First       - prioritize reachability and notification setup",
            "Remote / SSH Host   - safe service runtime with WebUI access via SSH tunnel",
            "Custom / Advanced   - start neutral and tune each major choice directly",
        ];
        print_info("Choose the lane that best matches the system you want to leave setup with.");
        let choice = select_one("Choose your setup lane", &options).map_err(SetupError::Io)?;
        self.selected_profile = match choice {
            1 => OnboardingProfile::LocalAndPrivate,
            2 => OnboardingProfile::BuilderAndCoding,
            3 => OnboardingProfile::ChannelFirst,
            4 => OnboardingProfile::RemoteServer,
            5 => OnboardingProfile::CustomAdvanced,
            _ => OnboardingProfile::Balanced,
        };

        if matches!(self.selected_profile, OnboardingProfile::CustomAdvanced) {
            print_success(
                "Using the Custom / Advanced profile. ThinClaw will keep profile-driven defaults light so you can make each major choice directly.",
            );
        } else {
            print_success(&format!(
                "Using the {} profile. Recommendations are prefilled, and every relevant section still stays reviewable.",
                self.selected_profile.title()
            ));
        }
        print_info(self.selected_profile.description());
        Ok(())
    }

    fn apply_profile_defaults(&mut self) {
        match self.selected_profile {
            OnboardingProfile::Balanced => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                self.settings.providers.smart_routing_enabled = true;
                if self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly
                {
                    self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
                }
                self.settings.routines_enabled = true;
                if !self.settings.heartbeat.enabled {
                    self.settings.heartbeat.enabled = false;
                }
            }
            OnboardingProfile::LocalAndPrivate => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                if self.settings.llm_backend.is_none() {
                    self.settings.llm_backend = Some("ollama".to_string());
                }
                self.settings.providers.smart_routing_enabled = false;
                self.settings.providers.routing_mode = crate::settings::RoutingMode::PrimaryOnly;
                if !self.settings.embeddings.enabled {
                    self.settings.embeddings.provider = "ollama".to_string();
                    self.settings.embeddings.model = "nomic-embed-text".to_string();
                }
                self.settings.routines_enabled = true;
                self.settings.heartbeat.enabled = false;
            }
            OnboardingProfile::BuilderAndCoding => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                self.settings.providers.smart_routing_enabled = true;
                self.settings.providers.routing_mode =
                    crate::settings::RoutingMode::AdvisorExecutor;
                self.settings.providers.advisor_max_calls =
                    self.settings.providers.advisor_max_calls.max(4);
                self.settings.routines_enabled = true;
                self.settings.heartbeat.enabled = false;
            }
            OnboardingProfile::ChannelFirst => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                self.settings.providers.smart_routing_enabled = true;
                if self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly
                {
                    self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
                }
                self.settings.routines_enabled = true;
            }
            OnboardingProfile::RemoteServer => {
                self.settings.skills_enabled = true;
                self.settings.observability_backend = "none".to_string();
                self.settings.providers.smart_routing_enabled = true;
                if self.settings.providers.routing_mode == crate::settings::RoutingMode::PrimaryOnly
                {
                    self.settings.providers.routing_mode = crate::settings::RoutingMode::CheapSplit;
                }
                self.settings.routines_enabled = true;
                self.settings.heartbeat.enabled = false;
                self.settings.channels.cli_enabled = Some(false);
                self.settings.channels.gateway_enabled = Some(true);
                let gateway_host = self.remote_gateway_host_or_loopback().to_string();
                self.settings.channels.gateway_host = Some(gateway_host);
                self.settings.channels.gateway_port =
                    Some(self.settings.channels.gateway_port.unwrap_or(3000));
                self.ensure_gateway_auth_token();
                if self.settings.database_backend.is_none() {
                    self.settings.database_backend = Some("libsql".to_string());
                }
                if self.settings.libsql_path.is_none() {
                    self.settings.libsql_path = Some(
                        crate::config::default_libsql_path()
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
                if self.settings.secrets_master_key_source == crate::settings::KeySource::Env {
                    self.settings.secrets.allow_env_master_key = true;
                    self.settings.secrets.master_key_source =
                        crate::settings::SecretsMasterKeySource::Env;
                }
            }
            OnboardingProfile::CustomAdvanced => {}
        }
    }

    fn ensure_gateway_auth_token(&mut self) {
        let has_token = self
            .settings
            .channels
            .gateway_auth_token
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty());
        if has_token {
            return;
        }

        use rand::Rng;
        let token: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(48)
            .map(char::from)
            .collect();
        self.settings.channels.gateway_auth_token = Some(token);
    }

    fn remote_gateway_host_or_loopback(&self) -> &str {
        self.settings
            .channels
            .gateway_host
            .as_deref()
            .filter(|host| !host.trim().is_empty())
            .unwrap_or("127.0.0.1")
    }

    fn configured_channel_names(&self) -> Vec<String> {
        let mut channels = Vec::new();
        if self.settings.channels.http_enabled {
            channels.push("http".to_string());
        }
        if self.settings.channels.signal_enabled {
            channels.push("signal".to_string());
        }
        if self.settings.channels.discord_enabled {
            channels.push("discord".to_string());
        }
        if self.settings.channels.slack_enabled {
            channels.push("slack".to_string());
        }
        if self.settings.channels.nostr_enabled {
            channels.push("nostr".to_string());
        }
        if self.settings.channels.gmail_enabled {
            channels.push("gmail".to_string());
        }
        #[cfg(target_os = "macos")]
        if self.settings.channels.imessage_enabled {
            channels.push("imessage".to_string());
        }
        #[cfg(target_os = "macos")]
        if self.settings.channels.apple_mail_enabled {
            channels.push("apple_mail".to_string());
        }
        if self.settings.channels.bluebubbles_enabled {
            channels.push("bluebubbles".to_string());
        }
        channels.extend(self.settings.channels.wasm_channels.iter().cloned());
        channels
    }

    fn ensure_remote_provider_followup(&mut self) {
        let provider_needs_credentials = |provider_slug: &str| match provider_slug {
            "ollama" | "llama_cpp" => false,
            "openai_compatible" => {
                let base_url = self
                    .settings
                    .openai_compatible_base_url
                    .as_deref()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let looks_local = base_url.starts_with("http://localhost")
                    || base_url.starts_with("https://localhost")
                    || base_url.contains("127.0.0.1")
                    || base_url.contains("0.0.0.0");
                !looks_local
            }
            _ => true,
        };

        let primary_provider = self
            .settings
            .providers
            .primary
            .as_deref()
            .or(self.settings.llm_backend.as_deref());
        let primary_needs_credentials = primary_provider
            .map(provider_needs_credentials)
            .unwrap_or(false);
        let any_enabled_needs_credentials = self
            .settings
            .providers
            .enabled
            .iter()
            .any(|slug| provider_needs_credentials(slug));

        if !primary_needs_credentials && !any_enabled_needs_credentials {
            self.remove_followup("provider-auth");
            return;
        }

        self.add_followup(FollowupDraft {
            id: "provider-auth".to_string(),
            title: "Provide remote model credentials".to_string(),
            category: OnboardingFollowupCategory::Authentication,
            status: OnboardingFollowupStatus::Pending,
            instructions: "Skip-auth mode kept provider review non-secret. Add the relevant provider API key before relying on remote routing or failover.".to_string(),
            action_hint: Some("Set the provider env var or rerun `thinclaw onboard --ui cli` without --skip-auth.".to_string()),
        });
    }

    fn step_channel_continuity(&self) -> Result<(), SetupError> {
        print_info(
            "ThinClaw keeps one canonical direct-message session per linked principal across channels and devices.",
        );
        print_info(
            "That means the same person can continue a direct conversation from another channel without losing context.",
        );
        print_info(
            "Group threads stay isolated so public or shared spaces do not bleed into direct sessions.",
        );
        Ok(())
    }

    async fn step_channel_verification(&mut self) -> Result<usize, SetupError> {
        let mut issues = 0usize;
        self.verified_channels.clear();

        self.remove_followup("channel-verification");

        if self.configured_channel_names().is_empty() {
            if self.is_quick_setup() && self.quick_primary_channel.as_deref() == Some("web") {
                self.verified_channels.insert("web".to_string(), true);
                print_success("Web Dashboard selected as the verified quick-setup path.");
                return Ok(0);
            }

            self.add_followup(FollowupDraft {
                id: "channel-verification".to_string(),
                title: "No external channels verified yet".to_string(),
                category: OnboardingFollowupCategory::Verification,
                status: OnboardingFollowupStatus::Optional,
                instructions: "ThinClaw can still run locally, but no external messaging path is configured yet.".to_string(),
                action_hint: Some("Rerun `thinclaw onboard --channels-only` when you are ready to add a channel.".to_string()),
            });
            print_warning(
                "No external messaging channel is configured yet. ThinClaw will still work locally.",
            );
            return Ok(1);
        }

        let secrets = self.init_secrets_context().await.ok();

        if self.settings.channels.http_enabled {
            let host = self
                .settings
                .channels
                .http_host
                .as_deref()
                .unwrap_or("0.0.0.0");
            let port = self.settings.channels.http_port.unwrap_or(8080);
            let ready = !host.trim().is_empty() && port > 0;
            self.verified_channels.insert("http".to_string(), ready);
            if ready {
                print_success("HTTP channel configuration looks valid.");
            } else {
                issues += 1;
                print_warning("HTTP channel is enabled but host/port configuration is invalid.");
            }
        }

        if self.settings.channels.signal_enabled {
            let signal_ready = if let (Some(url), Some(account)) = (
                self.settings.channels.signal_http_url.as_deref(),
                self.settings.channels.signal_account.as_deref(),
            ) {
                if url.trim().is_empty() || account.trim().is_empty() {
                    false
                } else {
                    Self::verify_http_reachable(url).await
                }
            } else {
                false
            };
            self.verified_channels
                .insert("signal".to_string(), signal_ready);
            if signal_ready {
                print_success("Signal verification passed (configuration + reachability).");
            } else {
                issues += 1;
                print_warning(
                    "Signal verification failed. Check account configuration and signal-http-api reachability.",
                );
            }
        }

        if self.settings.channels.discord_enabled {
            let mut discord_token = self
                .settings
                .channels
                .discord_bot_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("DISCORD_BOT_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if discord_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("discord_bot_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    discord_token = Some(token);
                }
            }

            let discord_ready = if let Some(token) = discord_token {
                Self::verify_discord_auth(&token).await
            } else {
                false
            };
            self.verified_channels
                .insert("discord".to_string(), discord_ready);
            if discord_ready {
                print_success("Discord verification passed (bot token accepted).");
            } else {
                issues += 1;
                print_warning(
                    "Discord verification failed. Ensure the bot token is valid and reachable.",
                );
            }
        }

        if self.settings.channels.slack_enabled {
            let mut bot_token = self
                .settings
                .channels
                .slack_bot_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("SLACK_BOT_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if bot_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("slack_bot_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    bot_token = Some(token);
                }
            }

            let mut app_token = self
                .settings
                .channels
                .slack_app_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("SLACK_APP_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if app_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("slack_app_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    app_token = Some(token);
                }
            }

            let slack_ready = if let (Some(bot), Some(app)) = (bot_token, app_token) {
                Self::verify_slack_bot_auth(&bot).await && Self::verify_slack_app_auth(&app).await
            } else {
                false
            };
            self.verified_channels
                .insert("slack".to_string(), slack_ready);
            if slack_ready {
                print_success("Slack verification passed (bot + app tokens accepted).");
            } else {
                issues += 1;
                print_warning(
                    "Slack verification failed. Check bot/app tokens and workspace connectivity.",
                );
            }
        }

        if self.settings.channels.nostr_enabled {
            let relays = self
                .settings
                .channels
                .nostr_relays
                .clone()
                .unwrap_or_default();
            let nostr_ready = Self::verify_nostr_relays(&relays).await;
            self.verified_channels
                .insert("nostr".to_string(), nostr_ready);
            if nostr_ready {
                print_success("Nostr relay verification passed.");
            } else {
                issues += 1;
                print_warning(
                    "Nostr verification failed. Ensure at least one relay URL is valid and reachable.",
                );
            }
        }

        if self.settings.channels.gmail_enabled {
            let gmail_ready = self
                .settings
                .channels
                .gmail_project_id
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
                && self
                    .settings
                    .channels
                    .gmail_subscription_id
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty())
                && self
                    .settings
                    .channels
                    .gmail_topic_id
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty());
            self.verified_channels
                .insert("gmail".to_string(), gmail_ready);
            if gmail_ready {
                print_success("Gmail verification passed (required Pub/Sub fields present).");
            } else {
                issues += 1;
                print_warning(
                    "Gmail verification failed. Project, subscription, and topic are required.",
                );
            }
        }

        if self.settings.channels.bluebubbles_enabled {
            let bb_ready = if let Some(ref url) = self.settings.channels.bluebubbles_server_url {
                if url.trim().is_empty() {
                    false
                } else {
                    let ping_url = format!("{}/api/v1/ping", url.trim_end_matches('/'));
                    Self::verify_http_reachable(&ping_url).await
                }
            } else {
                false
            };
            self.verified_channels
                .insert("bluebubbles".to_string(), bb_ready);
            if bb_ready {
                print_success("BlueBubbles verification passed (server reachable).");
            } else {
                issues += 1;
                print_warning(
                    "BlueBubbles verification failed. Ensure the server URL is correct and reachable.",
                );
            }
        }

        for wasm_channel in &self.settings.channels.wasm_channels {
            let ready = dirs::home_dir()
                .map(|home| {
                    home.join(".thinclaw/channels")
                        .join(format!("{wasm_channel}.wasm"))
                })
                .is_some_and(|path| path.exists());
            self.verified_channels.insert(wasm_channel.clone(), ready);
            if ready {
                print_success(&format!(
                    "WASM channel '{}' is installed and discoverable.",
                    wasm_channel
                ));
            } else {
                issues += 1;
                print_warning(&format!(
                    "WASM channel '{}' is enabled but the wasm artifact is missing.",
                    wasm_channel
                ));
            }
        }

        if issues > 0 {
            self.add_followup(FollowupDraft {
                id: "channel-verification".to_string(),
                title: "Review incomplete channel configuration".to_string(),
                category: OnboardingFollowupCategory::Verification,
                status: OnboardingFollowupStatus::NeedsAttention,
                instructions: "At least one enabled channel is still missing required configuration details or verification signals.".to_string(),
                action_hint: Some("Use `thinclaw onboard --channels-only` to revisit the channel flow.".to_string()),
            });
            print_warning(&format!(
                "Channel verification found {} configuration gap(s). The completion screen will keep a follow-up for you.",
                issues
            ));
        } else {
            print_success(
                "Channel verification found at least one ready messaging path and no obvious configuration gaps.",
            );
        }

        Ok(issues)
    }

    async fn verify_http_reachable(url: &str) -> bool {
        let parsed = match url::Url::parse(url) {
            Ok(parsed) => parsed,
            Err(_) => return false,
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return false;
        }
        reqwest::Client::new()
            .get(parsed)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }

    async fn verify_discord_auth(bot_token: &str) -> bool {
        reqwest::Client::new()
            .get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bot {}", bot_token.trim()))
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    async fn verify_slack_bot_auth(bot_token: &str) -> bool {
        let response = reqwest::Client::new()
            .post("https://slack.com/api/auth.test")
            .bearer_auth(bot_token.trim())
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = response.json::<serde_json::Value>().await else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_slack_app_auth(app_token: &str) -> bool {
        let response = reqwest::Client::new()
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(app_token.trim())
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = response.json::<serde_json::Value>().await else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_nostr_relays(relays_csv: &str) -> bool {
        for relay in relays_csv
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let Ok(url) = url::Url::parse(relay) else {
                continue;
            };
            if !matches!(url.scheme(), "ws" | "wss") {
                continue;
            }
            let Some(host) = url.host_str() else {
                continue;
            };
            let port = url
                .port_or_known_default()
                .unwrap_or(if url.scheme() == "wss" { 443 } else { 80 });
            let reachable = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                tokio::net::lookup_host((host, port)),
            )
            .await
            .ok()
            .and_then(|lookup| lookup.ok())
            .is_some_and(|mut addrs| addrs.next().is_some());
            if reachable {
                return true;
            }
        }
        false
    }

    /// Run the setup wizard.
    ///
    /// Settings are persisted incrementally after each successful step so
    /// that progress is not lost if a later step fails. On re-run, existing
    /// settings are loaded from the database after Step 1 establishes a
    /// connection, so users don't have to re-enter everything.
    pub async fn run(&mut self) -> Result<(), SetupError> {
        let requested_ui_mode = self.config.ui_mode;
        let ui_mode = self.resolve_ui_mode();
        self.resolved_ui_mode = ui_mode;

        let prompt_mode = match ui_mode {
            UiMode::Tui => PromptRenderMode::Tui,
            UiMode::Cli | UiMode::Auto => PromptRenderMode::Cli,
        };
        {
            let _prompt_mode = push_prompt_ui_mode(prompt_mode);
            self.prepare_run_mode(ui_mode).await?;
        }
        self.reset_plan_state();

        match (requested_ui_mode, ui_mode) {
            (_, UiMode::Cli) => self.run_cli_flow().await,
            (UiMode::Auto, UiMode::Tui) => match self.run_tui_flow().await {
                Ok(()) => Ok(()),
                Err(SetupError::Cancelled) => Err(SetupError::Cancelled),
                Err(error) => {
                    print_warning(&format!(
                        "The onboarding TUI could not continue cleanly ({}). Falling back to the terminal wizard.",
                        error
                    ));
                    self.resolved_ui_mode = UiMode::Cli;
                    self.reset_plan_state();
                    self.run_cli_flow().await
                }
            },
            (_, UiMode::Tui) => self.run_tui_flow().await,
            (_, UiMode::Auto) => unreachable!("auto mode is resolved before onboarding begins"),
        }
    }

    /// Reconnect to the existing database and load settings.
    ///
    /// Used by channels-only mode (and future single-step modes) so that
    /// `init_secrets_context()` and `save_and_summarize()` have a live
    /// database connection and the wizard's `self.settings` reflects the
    /// previously saved configuration.
    async fn reconnect_existing_db(&mut self) -> Result<(), SetupError> {
        // Determine backend from env (set by bootstrap .env loaded in main).
        let backend = std::env::var("DATABASE_BACKEND").unwrap_or_else(|_| "postgres".to_string());

        // Try libsql first if that's the configured backend.
        #[cfg(feature = "libsql")]
        if backend == "libsql" || backend == "turso" || backend == "sqlite" {
            return self.reconnect_libsql().await;
        }

        // Try postgres (either explicitly configured or as default).
        #[cfg(feature = "postgres")]
        {
            let _ = &backend;
            return self.reconnect_postgres().await;
        }

        #[allow(unreachable_code)]
        Err(SetupError::Database(
            "No database configured. Run full setup first (thinclaw onboard).".to_string(),
        ))
    }

    /// Reconnect to an existing PostgreSQL database and load settings.
    #[cfg(feature = "postgres")]
    async fn reconnect_postgres(&mut self) -> Result<(), SetupError> {
        let url = std::env::var("DATABASE_URL").map_err(|_| {
            SetupError::Database(
                "DATABASE_URL not set. Run full setup first (thinclaw onboard).".to_string(),
            )
        })?;

        self.test_database_connection_postgres(&url).await?;
        self.settings.database_backend = Some("postgres".to_string());
        self.settings.database_url = Some(url.clone());

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref pool) = self.db_pool {
            let store = crate::history::Store::from_pool(pool.clone());
            if let Ok(map) = store.get_all_settings("default").await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("postgres".to_string());
                self.settings.database_url = Some(url);
            }
        }

        Ok(())
    }

    /// Reconnect to an existing libSQL database and load settings.
    #[cfg(feature = "libsql")]
    async fn reconnect_libsql(&mut self) -> Result<(), SetupError> {
        let path = std::env::var("LIBSQL_PATH").unwrap_or_else(|_| {
            crate::config::default_libsql_path()
                .to_string_lossy()
                .to_string()
        });
        let turso_url = std::env::var("LIBSQL_URL").ok();
        let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();

        self.test_database_connection_libsql(&path, turso_url.as_deref(), turso_token.as_deref())
            .await?;

        self.settings.database_backend = Some("libsql".to_string());
        self.settings.libsql_path = Some(path.clone());
        if let Some(ref url) = turso_url {
            self.settings.libsql_url = Some(url.clone());
        }

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref db) = self.db_backend {
            use crate::db::SettingsStore as _;
            if let Ok(map) = db.get_all_settings("default").await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("libsql".to_string());
                self.settings.libsql_path = Some(path);
                if let Some(url) = turso_url {
                    self.settings.libsql_url = Some(url);
                }
            }
        }

        Ok(())
    }
}

// Step implementations are split into sub-modules by concern.
mod agent;
mod automation;
mod channels_step;
mod contracts;
mod extensions;
pub(crate) mod helpers;
mod infrastructure;
mod llm;
mod persistence;
mod presentation;
mod sandbox;
mod summary;
mod tui_shell;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;

    use crate::channels::wasm::{ChannelCapabilitiesFile, available_channel_names};
    use crate::config::helpers::lock_env;

    use super::helpers::*;
    use super::*;

    #[test]
    fn test_wizard_creation() {
        let wizard = SetupWizard::new();
        assert!(!wizard.config.skip_auth);
        assert!(!wizard.config.channels_only);
        assert_eq!(wizard.config.ui_mode, UiMode::Auto);
    }

    #[test]
    fn test_wizard_with_config() {
        let config = SetupConfig {
            skip_auth: true,
            channels_only: false,
            ui_mode: UiMode::Cli,
            guide_topic: None,
            profile: None,
            pause_after_completion: false,
        };
        let wizard = SetupWizard::with_config(config);
        assert!(wizard.config.skip_auth);
    }

    #[test]
    fn test_default_onboarding_continues_into_runtime() {
        let wizard = SetupWizard::new();
        assert!(wizard.should_continue_to_runtime());
        assert_eq!(wizard.primary_runtime_command(), "thinclaw");
    }

    #[test]
    fn test_quick_setup_plan_uses_documented_twelve_steps() {
        let wizard = SetupWizard::new();
        let plan = wizard.build_plan();

        assert_eq!(plan.steps.len(), 12);
        assert!(
            !plan
                .steps
                .iter()
                .any(|step| step.id == WizardStepId::SmartRouting)
        );
        assert!(
            plan.steps
                .iter()
                .any(|step| step.id == WizardStepId::CodingWorkers)
        );
    }

    #[test]
    fn test_tui_back_reopens_menus_on_first_guided_step() {
        let mut wizard = SetupWizard::new();
        wizard.config.guide_topic = Some(GuideTopic::Ai);

        assert!(wizard.should_reopen_tui_menus_on_back(0, true));
    }

    #[test]
    fn test_tui_back_does_not_reopen_menus_after_first_step() {
        let mut wizard = SetupWizard::new();
        wizard.config.guide_topic = Some(GuideTopic::Ai);

        assert!(!wizard.should_reopen_tui_menus_on_back(1, true));
    }

    #[test]
    fn test_tui_back_reopens_menus_in_quick_setup() {
        let wizard = SetupWizard::new();

        // Quick setup should also allow Ctrl+B back to the entry prompt.
        assert!(wizard.should_reopen_tui_menus_on_back(0, true));
    }

    #[test]
    fn test_quick_notification_defaults_use_verified_telegram_owner() {
        let mut wizard = SetupWizard::new();
        wizard.quick_primary_channel = Some("telegram".to_string());
        wizard
            .verified_channels
            .insert("telegram".to_string(), true);
        wizard.settings.channels.telegram_owner_id = Some(684480568);

        wizard.apply_quick_notification_defaults();

        assert_eq!(
            wizard.settings.notifications.preferred_channel.as_deref(),
            Some("telegram")
        );
        assert_eq!(
            wizard.settings.notifications.recipient.as_deref(),
            Some("684480568")
        );
        assert!(wizard.settings.heartbeat.enabled);
        assert_eq!(
            wizard.settings.heartbeat.notify_channel.as_deref(),
            Some("telegram")
        );
    }

    #[tokio::test]
    async fn test_quick_web_channel_verification_is_ready() {
        let mut wizard = SetupWizard::new();
        wizard.quick_primary_channel = Some("web".to_string());

        let issues = wizard.step_channel_verification().await.unwrap();

        assert_eq!(issues, 0);
        assert_eq!(wizard.verified_channels.get("web"), Some(&true));
    }

    #[test]
    fn test_custom_advanced_profile_metadata() {
        assert_eq!(
            OnboardingProfile::CustomAdvanced.title(),
            "Custom / Advanced"
        );
        assert!(
            OnboardingProfile::CustomAdvanced
                .description()
                .contains("neutral baseline")
        );
    }

    #[test]
    fn test_custom_advanced_profile_preserves_existing_settings() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::CustomAdvanced;
        wizard.settings.skills_enabled = false;
        wizard.settings.observability_backend = "log".to_string();
        wizard.settings.providers.smart_routing_enabled = true;
        wizard.settings.providers.routing_mode = crate::settings::RoutingMode::Policy;
        wizard.settings.routines_enabled = false;
        wizard.settings.heartbeat.enabled = true;
        wizard.settings.llm_backend = Some("openai".to_string());

        wizard.apply_profile_defaults();

        assert!(!wizard.settings.skills_enabled);
        assert_eq!(wizard.settings.observability_backend, "log");
        assert!(wizard.settings.providers.smart_routing_enabled);
        assert_eq!(
            wizard.settings.providers.routing_mode,
            crate::settings::RoutingMode::Policy
        );
        assert!(!wizard.settings.routines_enabled);
        assert!(wizard.settings.heartbeat.enabled);
        assert_eq!(wizard.settings.llm_backend.as_deref(), Some("openai"));
    }

    #[test]
    fn test_remote_provider_followup_added_for_remote_primary() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("openai".to_string());

        wizard.ensure_remote_provider_followup();

        assert!(wizard.followups.iter().any(|f| f.id == "provider-auth"));
    }

    #[test]
    fn test_remote_provider_followup_skipped_for_local_only() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("openai".to_string());
        wizard.ensure_remote_provider_followup();
        assert!(wizard.followups.iter().any(|f| f.id == "provider-auth"));

        wizard.settings.providers.primary = Some("ollama".to_string());
        wizard.settings.providers.enabled = vec!["ollama".to_string()];
        wizard.ensure_remote_provider_followup();

        assert!(!wizard.followups.iter().any(|f| f.id == "provider-auth"));
    }

    #[test]
    fn test_remote_provider_followup_kept_for_remote_fallback() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("ollama".to_string());
        wizard.settings.providers.enabled = vec!["ollama".to_string(), "openai".to_string()];

        wizard.ensure_remote_provider_followup();

        assert!(wizard.followups.iter().any(|f| f.id == "provider-auth"));
    }

    #[test]
    #[cfg(feature = "postgres")]
    fn test_mask_password_in_url() {
        assert_eq!(
            mask_password_in_url("postgres://user:secret@localhost/db"),
            "postgres://user:****@localhost/db"
        );

        // URL without password
        assert_eq!(
            mask_password_in_url("postgres://localhost/db"),
            "postgres://localhost/db"
        );
    }

    #[test]
    fn test_capitalize_first() {
        assert_eq!(capitalize_first("telegram"), "Telegram");
        assert_eq!(capitalize_first("CAPS"), "CAPS");
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_mask_api_key() {
        assert_eq!(
            mask_api_key("sk-ant-api03-abcdef1234567890"),
            "sk-ant...7890"
        );
        assert_eq!(mask_api_key("short"), "shor...");
        assert_eq!(mask_api_key("exactly12ch"), "exac...");
        assert_eq!(mask_api_key("exactly12chr"), "exactl...2chr");
        assert_eq!(mask_api_key(""), "...");
        // Multi-byte chars should not panic
        assert_eq!(mask_api_key("日本語キー"), "日本語キ...");
    }

    #[tokio::test]
    async fn test_install_missing_bundled_channels_installs_telegram() {
        // WASM artifacts only exist in dev builds (not CI). Skip gracefully
        // rather than fail when the telegram channel hasn't been compiled.
        if !available_channel_names().contains(&"telegram") {
            eprintln!("skipping: telegram WASM artifacts not built");
            return;
        }

        let dir = tempdir().unwrap();
        let installed = HashSet::<String>::new();

        install_missing_bundled_channels(dir.path(), &installed)
            .await
            .unwrap();

        assert!(dir.path().join("telegram.wasm").exists());
        assert!(dir.path().join("telegram.capabilities.json").exists());
    }

    #[test]
    fn test_build_channel_options_includes_available_when_missing() {
        let discovered = Vec::new();
        let options = build_channel_options(&discovered);
        let available = available_channel_names();
        // All available (built) channels should appear
        for name in &available {
            assert!(
                options.contains(&name.to_string()),
                "expected '{}' in options",
                name
            );
        }
    }

    #[test]
    fn test_build_channel_options_dedupes_available() {
        let discovered = vec![(String::from("telegram"), ChannelCapabilitiesFile::default())];
        let options = build_channel_options(&discovered);
        // telegram should appear exactly once despite being both discovered and available
        assert_eq!(
            options.iter().filter(|n| *n == "telegram").count(),
            1,
            "telegram should not be duplicated"
        );
    }

    #[test]
    fn test_claude_code_key_enable_anthropic_provider_without_changing_primary() {
        let mut wizard = SetupWizard::new();
        wizard.settings.providers.primary = Some("openai".to_string());

        wizard.enable_anthropic_provider_for_claude_code_key();

        assert_eq!(wizard.settings.providers.primary.as_deref(), Some("openai"));
        assert!(
            wizard
                .settings
                .providers
                .enabled
                .iter()
                .any(|slug| slug == "anthropic")
        );

        let slots = wizard
            .settings
            .providers
            .provider_models
            .get("anthropic")
            .expect("anthropic slots should be created");
        assert_eq!(slots.primary.as_deref(), Some("claude-opus-4-7"));
        assert_eq!(slots.cheap.as_deref(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn test_builder_and_coding_profile_enforces_advisor_executor_defaults() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::BuilderAndCoding;
        wizard.settings.providers.advisor_max_calls = 1;

        wizard.apply_profile_defaults();

        assert!(wizard.settings.providers.smart_routing_enabled);
        assert_eq!(
            wizard.settings.providers.routing_mode,
            crate::settings::RoutingMode::AdvisorExecutor
        );
        assert_eq!(wizard.settings.providers.advisor_max_calls, 4);
    }

    #[test]
    fn test_remote_profile_applies_service_safe_gateway_defaults() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::RemoteServer;

        wizard.apply_profile_defaults();

        assert_eq!(wizard.settings.channels.cli_enabled, Some(false));
        assert_eq!(wizard.settings.channels.gateway_enabled, Some(true));
        assert_eq!(
            wizard.settings.channels.gateway_host.as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(wizard.settings.channels.gateway_port, Some(3000));
        assert!(
            wizard
                .settings
                .channels
                .gateway_auth_token
                .as_deref()
                .is_some_and(|token| token.len() >= 32)
        );
        assert_eq!(wizard.settings.database_backend.as_deref(), Some("libsql"));
    }

    #[test]
    fn test_cli_supplied_remote_profile_is_preselected() {
        let wizard = SetupWizard::with_config(SetupConfig {
            profile: Some(OnboardingProfile::RemoteServer),
            ..SetupConfig::default()
        });

        assert_eq!(wizard.selected_profile, OnboardingProfile::RemoteServer);
    }

    #[test]
    fn test_remote_bootstrap_env_writes_gateway_and_cli_keys() {
        let temp = tempdir().expect("temp thinclaw home");
        let _guard = EnvGuard::set("THINCLAW_HOME", temp.path().to_string_lossy().into_owned());
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::RemoteServer;
        wizard.apply_profile_defaults();
        wizard.settings.onboard_completed = true;

        wizard.write_bootstrap_env().expect("write bootstrap env");

        let env_path = temp.path().join(".env");
        let content = std::fs::read_to_string(env_path).expect("read bootstrap env");
        assert!(content.contains("GATEWAY_ENABLED=\"true\""));
        assert!(content.contains("GATEWAY_HOST=\"127.0.0.1\""));
        assert!(content.contains("GATEWAY_PORT=\"3000\""));
        assert!(content.contains("GATEWAY_AUTH_TOKEN=\""));
        assert!(content.contains("CLI_ENABLED=\"false\""));
    }

    #[tokio::test]
    async fn test_fetch_anthropic_models_static_fallback() {
        // With no API key, should return static defaults
        let _guard = EnvGuard::clear("ANTHROPIC_API_KEY");
        let models = fetch_anthropic_models(None).await;
        assert!(!models.is_empty());
        assert!(
            models.iter().any(|(id, _)| id.contains("claude")),
            "static defaults should include a Claude model"
        );
    }

    #[tokio::test]
    async fn test_fetch_openai_models_static_fallback() {
        let _guard = EnvGuard::clear("OPENAI_API_KEY");
        let models = fetch_openai_models(None).await;
        assert!(!models.is_empty());
        assert_eq!(models[0].0, "gpt-5.3-codex");
        assert!(
            models.iter().any(|(id, _)| id.contains("gpt")),
            "static defaults should include a GPT model"
        );
    }

    #[test]
    fn test_is_openai_chat_model_includes_gpt5_and_filters_non_chat_variants() {
        assert!(is_openai_chat_model("gpt-5"));
        assert!(is_openai_chat_model("gpt-5-mini-2026-01-01"));
        assert!(is_openai_chat_model("o3-2025-04-16"));
        assert!(!is_openai_chat_model("chatgpt-image-latest"));
        assert!(!is_openai_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_openai_chat_model("gpt-4o-mini-transcribe"));
        assert!(!is_openai_chat_model("text-embedding-3-large"));
    }

    #[test]
    fn test_sort_openai_models_prioritizes_best_models_first() {
        let mut models = vec![
            ("gpt-4o-mini".to_string(), "gpt-4o-mini".to_string()),
            ("gpt-5-mini".to_string(), "gpt-5-mini".to_string()),
            ("o3".to_string(), "o3".to_string()),
            ("gpt-4.1".to_string(), "gpt-4.1".to_string()),
            ("gpt-5".to_string(), "gpt-5".to_string()),
        ];

        sort_openai_models(&mut models);

        let ordered: Vec<String> = models.into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ordered,
            vec![
                "gpt-5".to_string(),
                "gpt-5-mini".to_string(),
                "o3".to_string(),
                "gpt-4.1".to_string(),
                "gpt-4o-mini".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_fetch_ollama_models_unreachable_fallback() {
        // Point at a port nothing listens on
        let models = fetch_ollama_models("http://127.0.0.1:1").await;
        assert!(!models.is_empty(), "should fall back to static defaults");
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_empty_dir() {
        let dir = tempdir().unwrap();
        let channels = discover_wasm_channels(dir.path()).await;
        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_nonexistent_dir() {
        let channels =
            discover_wasm_channels(std::path::Path::new("/tmp/thinclaw_nonexistent_dir")).await;
        assert!(channels.is_empty());
    }

    /// RAII guard that sets/clears an env var for the duration of a test.
    struct EnvGuard {
        _env_guard: std::sync::MutexGuard<'static, ()>,
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: String) -> Self {
            let env_guard = lock_env();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self {
                _env_guard: env_guard,
                key,
                original,
            }
        }

        fn clear(key: &'static str) -> Self {
            let env_guard = lock_env();
            let original = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self {
                _env_guard: env_guard,
                key,
                original,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref val) = self.original {
                    std::env::set_var(self.key, val);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}
