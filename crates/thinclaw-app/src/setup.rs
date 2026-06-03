//! Root-independent setup/onboarding planning contracts.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SetupWizardUiMode {
    #[default]
    Auto,
    Cli,
    Tui,
}

impl SetupWizardUiMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cli => "cli",
            Self::Tui => "tui",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupGuideTopic {
    Menu,
    Ai,
    Channels,
    Agent,
    Tools,
    Automation,
    Runtime,
}

impl SetupGuideTopic {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Menu => "Guided Settings",
            Self::Ai => "AI & Models",
            Self::Channels => "Channels & Notifications",
            Self::Agent => "Agent & Experience",
            Self::Tools => "Tools & Safety",
            Self::Automation => "Automation & Skills",
            Self::Runtime => "Runtime & Diagnostics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SetupOnboardingProfile {
    #[default]
    Balanced,
    LocalAndPrivate,
    BuilderAndCoding,
    ChannelFirst,
    RemoteServer,
    PiOsLite64,
    CustomAdvanced,
}

impl SetupOnboardingProfile {
    pub const fn title(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::LocalAndPrivate => "Local & Private",
            Self::BuilderAndCoding => "Builder & Coding",
            Self::ChannelFirst => "Channel-First",
            Self::RemoteServer => "Remote / SSH Host",
            Self::PiOsLite64 => "Pi OS Lite 64-bit",
            Self::CustomAdvanced => "Custom / Advanced",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Balanced => {
                "A steady first-flight profile: one reliable model path, smart routing, safe approvals, and minimal day-one friction."
            }
            Self::LocalAndPrivate => {
                "Bias toward local inference and a smaller trust surface, with conservative defaults and fewer outbound dependencies."
            }
            Self::BuilderAndCoding => {
                "Favor coding, tool use, and stronger planning with advisor/executor-style routing."
            }
            Self::ChannelFirst => {
                "Prioritize inbound and outbound channels so ThinClaw can meet you where you already work."
            }
            Self::RemoteServer => {
                "Run ThinClaw as a safe headless/service runtime on a Raspberry Pi, Mac Mini, VPS, or SSH-managed host with WebUI access through a tunnel by default."
            }
            Self::PiOsLite64 => {
                "Run ThinClaw on Raspberry Pi OS Lite as a headless remote service with WebUI access, Docker Chromium fallback, and desktop autonomy explicitly blocked."
            }
            Self::CustomAdvanced => {
                "Start from a neutral baseline with minimal profile assumptions so you can choose the stack, routing, and trust boundaries step by step."
            }
        }
    }

    pub const fn is_headless_remote(self) -> bool {
        matches!(self, Self::RemoteServer | Self::PiOsLite64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupRuntimeProfile {
    Remote,
    PiOsLite64,
}

impl SetupRuntimeProfile {
    pub const fn env_value(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::PiOsLite64 => "pi-os-lite-64",
        }
    }

    pub const fn is_headless(self) -> bool {
        true
    }
}

impl SetupOnboardingProfile {
    pub const fn runtime_profile(self) -> Option<SetupRuntimeProfile> {
        match self {
            Self::RemoteServer => Some(SetupRuntimeProfile::Remote),
            Self::PiOsLite64 => Some(SetupRuntimeProfile::PiOsLite64),
            _ => None,
        }
    }

    pub const fn runtime_profile_env_value(self) -> Option<&'static str> {
        match self.runtime_profile() {
            Some(profile) => Some(profile.env_value()),
            None => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SetupWizardPhaseId {
    WelcomeProfile,
    CoreRuntime,
    AiStack,
    IdentityPresence,
    ChannelsContinuity,
    CapabilitiesAutomation,
    ExperienceOperations,
    Finish,
}

impl SetupWizardPhaseId {
    pub const fn title(self) -> &'static str {
        match self {
            Self::WelcomeProfile => "Skin & Profile",
            Self::CoreRuntime => "Core Runtime",
            Self::AiStack => "AI Stack",
            Self::IdentityPresence => "Identity & Presence",
            Self::ChannelsContinuity => "Channels & Continuity",
            Self::CapabilitiesAutomation => "Capabilities & Automation",
            Self::ExperienceOperations => "Experience & Operations",
            Self::Finish => "Finish",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::WelcomeProfile => {
                "Choose the cockpit look first, then pick the onboarding lane that fits your setup."
            }
            Self::CoreRuntime => "Establish storage, secrets, and the base operating posture.",
            Self::AiStack => "Configure providers, models, routing, fallback, and memory search.",
            Self::IdentityPresence => "Confirm how the agent presents itself and keeps time.",
            Self::ChannelsContinuity => {
                "Enable channels, explain continuity, and verify what is truly launch-ready."
            }
            Self::CapabilitiesAutomation => {
                "Choose tools, trust boundaries, and background behavior with intention."
            }
            Self::ExperienceOperations => "Tune the operator cockpit and the visibility you want.",
            Self::Finish => "Review readiness, capture follow-ups, and hand off to runtime.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SetupWizardStepId {
    CliSkin,
    Profile,
    Database,
    Security,
    InferenceProvider,
    ModelSelection,
    SmartRouting,
    FallbackProviders,
    Embeddings,
    AgentIdentity,
    Timezone,
    Channels,
    ChannelContinuity,
    ChannelVerification,
    Notifications,
    Extensions,
    DockerSandbox,
    CodingWorkers,
    ClaudeCode,
    CodexCode,
    ToolApproval,
    Routines,
    Skills,
    Heartbeat,
    WebUi,
    Observability,
    Summary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupStepDescriptor {
    pub id: SetupWizardStepId,
    pub phase_id: SetupWizardPhaseId,
    pub title: &'static str,
    pub description: &'static str,
    pub why_this_matters: &'static str,
    pub recommended: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupWizardPhase {
    pub id: SetupWizardPhaseId,
    pub step_ids: Vec<SetupWizardStepId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupWizardPlan {
    pub phases: Vec<SetupWizardPhase>,
    pub steps: Vec<SetupStepDescriptor>,
}

impl SetupWizardPlan {
    pub fn total_steps(&self) -> usize {
        self.steps.len()
    }

    pub fn phase(&self, id: SetupWizardPhaseId) -> Option<&SetupWizardPhase> {
        self.phases.iter().find(|phase| phase.id == id)
    }

    pub fn phase_index(&self, id: SetupWizardPhaseId) -> Option<usize> {
        self.phases.iter().position(|phase| phase.id == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupStepStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
    NeedsAttention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupValidationLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupValidationItem {
    pub level: SetupValidationLevel,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupReadinessSummary {
    pub ready_now: usize,
    pub needs_attention: usize,
    pub followups: usize,
    pub headline: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SetupWizardPlanInput {
    pub channels_only: bool,
    pub guide_topic: Option<SetupGuideTopic>,
}

impl SetupWizardPlanInput {
    pub const fn is_guide_mode(self) -> bool {
        self.guide_topic.is_some()
    }

    pub const fn is_quick_setup(self) -> bool {
        !self.channels_only && !self.is_guide_mode()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupRuntimeCommandInput {
    pub profile: SetupOnboardingProfile,
    pub ui_mode: SetupWizardUiMode,
    pub continue_to_runtime: bool,
    pub pause_after_completion: bool,
}

pub fn setup_primary_runtime_command(input: SetupRuntimeCommandInput) -> &'static str {
    if input.profile.is_headless_remote() {
        return "thinclaw run --no-onboard";
    }
    match input.ui_mode {
        SetupWizardUiMode::Tui => "thinclaw tui",
        SetupWizardUiMode::Cli | SetupWizardUiMode::Auto => "thinclaw",
    }
}

pub fn setup_runtime_handoff_summary(input: SetupRuntimeCommandInput) -> String {
    if input.profile.is_headless_remote() {
        return if input.continue_to_runtime {
            format!(
                "ThinClaw will now continue into `thinclaw run --no-onboard` with the service-safe {} runtime settings from this run.",
                input.profile.title()
            )
        } else {
            "Settings are saved. Start the headless runtime with `thinclaw run --no-onboard` or install/start the OS service.".to_string()
        };
    }
    if input.continue_to_runtime {
        format!(
            "ThinClaw will now continue into `{}` using the settings from this run.",
            setup_primary_runtime_command(input)
        )
    } else {
        "Settings are saved. This pass stops here so you can launch runtime later on your own."
            .to_string()
    }
}

pub fn setup_what_next_commands(input: SetupRuntimeCommandInput) -> Vec<String> {
    if input.profile.is_headless_remote() {
        let mut commands = vec![
            "Service-safe runtime: thinclaw run --no-onboard".to_string(),
            "Install OS service: thinclaw service install".to_string(),
            "Start OS service: thinclaw service start".to_string(),
            "Show WebUI access: thinclaw gateway access".to_string(),
            "Show full token URL: thinclaw gateway access --show-token".to_string(),
        ];
        if input.profile == SetupOnboardingProfile::PiOsLite64 {
            commands.push("Pi diagnostics: thinclaw doctor --profile pi-os-lite-64".to_string());
            commands
                .push("Reopen Pi onboarding: thinclaw onboard --profile pi-os-lite-64".to_string());
        } else {
            commands.push("Remote diagnostics: thinclaw doctor --profile remote".to_string());
            commands
                .push("Reopen remote onboarding: thinclaw onboard --profile remote".to_string());
        }
        return commands;
    }
    let mut commands = vec![
        format!("Primary runtime: {}", setup_primary_runtime_command(input)),
        "Standard CLI runtime: thinclaw".to_string(),
        "Full-screen TUI runtime: thinclaw tui".to_string(),
        "Reopen onboarding: thinclaw onboard".to_string(),
        "Revisit channels only: thinclaw onboard --channels-only".to_string(),
    ];

    if input.pause_after_completion {
        commands.push("Topic guide: thinclaw onboard --guide".to_string());
    }

    commands
}

pub fn setup_wizard_plan(input: SetupWizardPlanInput) -> SetupWizardPlan {
    use SetupWizardPhaseId as Phase;
    use SetupWizardStepId as Step;

    let mut phases = Vec::new();
    let mut steps = Vec::new();

    let mut push_step = |id, phase_id, title, description, why_this_matters, recommended| {
        steps.push(SetupStepDescriptor {
            id,
            phase_id,
            title,
            description,
            why_this_matters,
            recommended,
        });
    };

    if input.channels_only {
        phases.push(SetupWizardPhase {
            id: Phase::ChannelsContinuity,
            step_ids: vec![Step::Channels, Step::ChannelVerification],
        });
        phases.push(SetupWizardPhase {
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

        return SetupWizardPlan { phases, steps };
    }

    if let Some(topic) = input.guide_topic {
        let step_ids = match topic {
            SetupGuideTopic::Menu => vec![Step::Summary],
            SetupGuideTopic::Ai => vec![
                Step::InferenceProvider,
                Step::ModelSelection,
                Step::SmartRouting,
                Step::FallbackProviders,
                Step::Embeddings,
            ],
            SetupGuideTopic::Channels => vec![
                Step::Channels,
                Step::ChannelContinuity,
                Step::ChannelVerification,
                Step::Notifications,
            ],
            SetupGuideTopic::Agent => {
                vec![
                    Step::CliSkin,
                    Step::AgentIdentity,
                    Step::Timezone,
                    Step::WebUi,
                ]
            }
            SetupGuideTopic::Tools => vec![
                Step::ToolApproval,
                Step::DockerSandbox,
                Step::Extensions,
                Step::ClaudeCode,
                Step::CodexCode,
            ],
            SetupGuideTopic::Automation => vec![Step::Routines, Step::Skills, Step::Heartbeat],
            SetupGuideTopic::Runtime => vec![Step::Database, Step::Security, Step::Observability],
        };

        let phase_id = match topic {
            SetupGuideTopic::Menu => Phase::Finish,
            SetupGuideTopic::Ai => Phase::AiStack,
            SetupGuideTopic::Channels => Phase::ChannelsContinuity,
            SetupGuideTopic::Agent => Phase::WelcomeProfile,
            SetupGuideTopic::Tools => Phase::CapabilitiesAutomation,
            SetupGuideTopic::Automation => Phase::CapabilitiesAutomation,
            SetupGuideTopic::Runtime => Phase::CoreRuntime,
        };
        phases.push(SetupWizardPhase {
            id: phase_id,
            step_ids,
        });
        phases.push(SetupWizardPhase {
            id: Phase::Finish,
            step_ids: vec![Step::Summary],
        });
    } else {
        phases.push(SetupWizardPhase {
            id: Phase::WelcomeProfile,
            step_ids: vec![Step::CliSkin, Step::Profile, Step::AgentIdentity],
        });
        phases.push(SetupWizardPhase {
            id: Phase::AiStack,
            step_ids: vec![Step::InferenceProvider, Step::ModelSelection],
        });
        phases.push(SetupWizardPhase {
            id: Phase::ChannelsContinuity,
            step_ids: vec![Step::Channels, Step::ChannelVerification],
        });
        phases.push(SetupWizardPhase {
            id: Phase::CapabilitiesAutomation,
            step_ids: vec![Step::ToolApproval, Step::DockerSandbox, Step::CodingWorkers],
        });
        phases.push(SetupWizardPhase {
            id: Phase::ExperienceOperations,
            step_ids: vec![Step::WebUi],
        });
        phases.push(SetupWizardPhase {
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

    let allowed_ids: BTreeSet<_> = phases
        .iter()
        .flat_map(|phase| phase.step_ids.iter().copied())
        .collect();
    let filtered_steps = steps
        .into_iter()
        .filter(|descriptor| allowed_ids.contains(&descriptor.id))
        .collect();

    SetupWizardPlan {
        phases,
        steps: filtered_steps,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapChannelInput {
    pub signal_http_url: Option<String>,
    pub signal_account: Option<String>,
    pub signal_allow_from: Option<String>,
    pub signal_allow_from_groups: Option<String>,
    pub signal_dm_policy: Option<String>,
    pub signal_group_policy: Option<String>,
    pub signal_group_allow_from: Option<String>,
    pub http_enabled: bool,
    pub http_host: Option<String>,
    pub http_port: Option<u16>,
    pub discord_enabled: bool,
    pub discord_bot_token: Option<String>,
    pub discord_guild_id: Option<String>,
    pub discord_allow_from: Option<String>,
    pub slack_enabled: bool,
    pub slack_bot_token: Option<String>,
    pub slack_app_token: Option<String>,
    pub slack_allow_from: Option<String>,
    pub nostr_enabled: bool,
    pub nostr_relays: Option<String>,
    pub nostr_owner_pubkey: Option<String>,
    pub nostr_social_dm_enabled: bool,
    pub nostr_allow_from: Option<String>,
    pub gmail_enabled: bool,
    pub gmail_project_id: Option<String>,
    pub gmail_subscription_id: Option<String>,
    pub gmail_topic_id: Option<String>,
    pub gmail_allowed_senders: Option<String>,
    pub imessage_enabled: bool,
    pub imessage_allow_from: Option<String>,
    pub imessage_poll_interval: Option<u64>,
    pub apple_mail_enabled: bool,
    pub apple_mail_allow_from: Option<String>,
    pub apple_mail_poll_interval: Option<u64>,
    pub apple_mail_unread_only: bool,
    pub apple_mail_mark_as_read: bool,
    pub bluebubbles_enabled: bool,
    pub bluebubbles_server_url: Option<String>,
    pub bluebubbles_password: Option<String>,
    pub bluebubbles_webhook_host: Option<String>,
    pub bluebubbles_webhook_port: Option<u16>,
    pub bluebubbles_allow_from: Option<String>,
    pub bluebubbles_send_read_receipts: Option<bool>,
    pub gateway_enabled: Option<bool>,
    pub gateway_host: Option<String>,
    pub gateway_port: Option<u16>,
    pub gateway_auth_token: Option<String>,
    pub cli_enabled: Option<bool>,
}

impl Default for SetupBootstrapChannelInput {
    fn default() -> Self {
        Self {
            signal_http_url: None,
            signal_account: None,
            signal_allow_from: None,
            signal_allow_from_groups: None,
            signal_dm_policy: None,
            signal_group_policy: None,
            signal_group_allow_from: None,
            http_enabled: false,
            http_host: None,
            http_port: None,
            discord_enabled: false,
            discord_bot_token: None,
            discord_guild_id: None,
            discord_allow_from: None,
            slack_enabled: false,
            slack_bot_token: None,
            slack_app_token: None,
            slack_allow_from: None,
            nostr_enabled: false,
            nostr_relays: None,
            nostr_owner_pubkey: None,
            nostr_social_dm_enabled: false,
            nostr_allow_from: None,
            gmail_enabled: false,
            gmail_project_id: None,
            gmail_subscription_id: None,
            gmail_topic_id: None,
            gmail_allowed_senders: None,
            imessage_enabled: false,
            imessage_allow_from: None,
            imessage_poll_interval: None,
            apple_mail_enabled: false,
            apple_mail_allow_from: None,
            apple_mail_poll_interval: None,
            apple_mail_unread_only: true,
            apple_mail_mark_as_read: true,
            bluebubbles_enabled: false,
            bluebubbles_server_url: None,
            bluebubbles_password: None,
            bluebubbles_webhook_host: None,
            bluebubbles_webhook_port: None,
            bluebubbles_allow_from: None,
            bluebubbles_send_read_receipts: None,
            gateway_enabled: None,
            gateway_host: None,
            gateway_port: None,
            gateway_auth_token: None,
            cli_enabled: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupBootstrapProviderInput {
    pub cheap_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapWebUiInput {
    pub skin: Option<String>,
    pub theme: String,
    pub accent_color: Option<String>,
    pub show_branding: bool,
}

impl Default for SetupBootstrapWebUiInput {
    fn default() -> Self {
        Self {
            skin: None,
            theme: "system".to_string(),
            accent_color: None,
            show_branding: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupBootstrapAgentInput {
    pub allow_local_tools: bool,
    pub workspace_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapEnvInput {
    pub database_backend: Option<String>,
    pub database_url: Option<String>,
    pub libsql_path: Option<String>,
    pub libsql_url: Option<String>,
    pub secrets_master_key: Option<String>,
    pub allow_env_master_key: bool,
    pub llm_backend: Option<String>,
    pub llm_base_url: Option<String>,
    pub ollama_base_url: Option<String>,
    pub onboard_completed: bool,
    pub runtime_profile: Option<SetupRuntimeProfile>,
    pub channels: SetupBootstrapChannelInput,
    pub providers: SetupBootstrapProviderInput,
    pub web_ui: SetupBootstrapWebUiInput,
    pub observability_backend: String,
    pub agent: SetupBootstrapAgentInput,
}

impl Default for SetupBootstrapEnvInput {
    fn default() -> Self {
        Self {
            database_backend: None,
            database_url: None,
            libsql_path: None,
            libsql_url: None,
            secrets_master_key: None,
            allow_env_master_key: false,
            llm_backend: None,
            llm_base_url: None,
            ollama_base_url: None,
            onboard_completed: false,
            runtime_profile: None,
            channels: SetupBootstrapChannelInput::default(),
            providers: SetupBootstrapProviderInput::default(),
            web_ui: SetupBootstrapWebUiInput::default(),
            observability_backend: "none".to_string(),
            agent: SetupBootstrapAgentInput::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapEnvVar {
    pub key: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupBootstrapEnvPlan {
    variables: Vec<SetupBootstrapEnvVar>,
}

impl SetupBootstrapEnvPlan {
    pub fn variables(&self) -> &[SetupBootstrapEnvVar] {
        &self.variables
    }

    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    fn push(&mut self, key: &'static str, value: impl Into<String>) {
        self.variables.push(SetupBootstrapEnvVar {
            key,
            value: value.into(),
        });
    }

    fn push_optional(&mut self, key: &'static str, value: &Option<String>) {
        if let Some(value) = value {
            self.push(key, value.clone());
        }
    }

    fn push_non_empty_optional(&mut self, key: &'static str, value: &Option<String>) {
        if let Some(value) = value
            && !value.is_empty()
        {
            self.push(key, value.clone());
        }
    }
}

pub fn setup_bootstrap_env_plan(input: &SetupBootstrapEnvInput) -> SetupBootstrapEnvPlan {
    let mut plan = SetupBootstrapEnvPlan::default();

    plan.push_optional("DATABASE_BACKEND", &input.database_backend);
    plan.push_optional("DATABASE_URL", &input.database_url);
    plan.push_optional("LIBSQL_PATH", &input.libsql_path);
    plan.push_optional("LIBSQL_URL", &input.libsql_url);
    plan.push_optional("SECRETS_MASTER_KEY", &input.secrets_master_key);
    if input.allow_env_master_key {
        plan.push("THINCLAW_ALLOW_ENV_MASTER_KEY", "1");
    }

    plan.push_optional("LLM_BACKEND", &input.llm_backend);
    plan.push_optional("LLM_BASE_URL", &input.llm_base_url);
    plan.push_optional("OLLAMA_BASE_URL", &input.ollama_base_url);

    if input.onboard_completed {
        plan.push("ONBOARD_COMPLETED", "true");
    }
    if let Some(profile) = input.runtime_profile {
        plan.push("THINCLAW_RUNTIME_PROFILE", profile.env_value());
        if profile.is_headless() {
            plan.push("THINCLAW_HEADLESS", "true");
        }
    }

    let channels = &input.channels;
    plan.push_optional("SIGNAL_HTTP_URL", &channels.signal_http_url);
    plan.push_optional("SIGNAL_ACCOUNT", &channels.signal_account);
    plan.push_optional("SIGNAL_ALLOW_FROM", &channels.signal_allow_from);
    plan.push_non_empty_optional(
        "SIGNAL_ALLOW_FROM_GROUPS",
        &channels.signal_allow_from_groups,
    );
    plan.push_optional("SIGNAL_DM_POLICY", &channels.signal_dm_policy);
    plan.push_optional("SIGNAL_GROUP_POLICY", &channels.signal_group_policy);
    plan.push_non_empty_optional("SIGNAL_GROUP_ALLOW_FROM", &channels.signal_group_allow_from);

    if channels.http_enabled {
        plan.push("HTTP_ENABLED", "true");
        plan.push_optional("HTTP_HOST", &channels.http_host);
        if let Some(port) = channels.http_port {
            plan.push("HTTP_PORT", port.to_string());
        }
    }

    if channels.discord_enabled {
        plan.push("DISCORD_ENABLED", "true");
        plan.push_optional("DISCORD_BOT_TOKEN", &channels.discord_bot_token);
    }
    plan.push_optional("DISCORD_GUILD_ID", &channels.discord_guild_id);
    plan.push_optional("DISCORD_ALLOW_FROM", &channels.discord_allow_from);

    if channels.slack_enabled {
        plan.push("SLACK_ENABLED", "true");
        plan.push_optional("SLACK_BOT_TOKEN", &channels.slack_bot_token);
        plan.push_optional("SLACK_APP_TOKEN", &channels.slack_app_token);
    }
    plan.push_optional("SLACK_ALLOW_FROM", &channels.slack_allow_from);

    if channels.nostr_enabled {
        plan.push("NOSTR_ENABLED", "true");
    }
    plan.push_optional("NOSTR_RELAYS", &channels.nostr_relays);
    plan.push_optional("NOSTR_OWNER_PUBKEY", &channels.nostr_owner_pubkey);
    if channels.nostr_social_dm_enabled {
        plan.push("NOSTR_SOCIAL_DM_ENABLED", "true");
    }
    plan.push_optional("NOSTR_ALLOW_FROM", &channels.nostr_allow_from);

    if channels.gmail_enabled {
        plan.push("GMAIL_ENABLED", "true");
    }
    plan.push_optional("GMAIL_PROJECT_ID", &channels.gmail_project_id);
    plan.push_optional("GMAIL_SUBSCRIPTION_ID", &channels.gmail_subscription_id);
    plan.push_optional("GMAIL_TOPIC_ID", &channels.gmail_topic_id);
    plan.push_optional("GMAIL_ALLOWED_SENDERS", &channels.gmail_allowed_senders);

    if channels.imessage_enabled {
        plan.push("IMESSAGE_ENABLED", "true");
    }
    plan.push_optional("IMESSAGE_ALLOW_FROM", &channels.imessage_allow_from);
    if let Some(interval) = channels.imessage_poll_interval {
        plan.push("IMESSAGE_POLL_INTERVAL", interval.to_string());
    }

    if channels.apple_mail_enabled {
        plan.push("APPLE_MAIL_ENABLED", "true");
    }
    plan.push_optional("APPLE_MAIL_ALLOW_FROM", &channels.apple_mail_allow_from);
    if let Some(interval) = channels.apple_mail_poll_interval {
        plan.push("APPLE_MAIL_POLL_INTERVAL", interval.to_string());
    }
    if !channels.apple_mail_unread_only {
        plan.push("APPLE_MAIL_UNREAD_ONLY", "false");
    }
    if !channels.apple_mail_mark_as_read {
        plan.push("APPLE_MAIL_MARK_AS_READ", "false");
    }

    if channels.bluebubbles_enabled {
        plan.push("BLUEBUBBLES_ENABLED", "true");
    }
    plan.push_optional("BLUEBUBBLES_SERVER_URL", &channels.bluebubbles_server_url);
    plan.push_optional("BLUEBUBBLES_PASSWORD", &channels.bluebubbles_password);
    plan.push_optional(
        "BLUEBUBBLES_WEBHOOK_HOST",
        &channels.bluebubbles_webhook_host,
    );
    if let Some(port) = channels.bluebubbles_webhook_port {
        plan.push("BLUEBUBBLES_WEBHOOK_PORT", port.to_string());
    }
    plan.push_optional("BLUEBUBBLES_ALLOW_FROM", &channels.bluebubbles_allow_from);
    if let Some(send_receipts) = channels.bluebubbles_send_read_receipts {
        plan.push("BLUEBUBBLES_SEND_READ_RECEIPTS", send_receipts.to_string());
    }

    if let Some(enabled) = channels.gateway_enabled {
        plan.push("GATEWAY_ENABLED", enabled.to_string());
    }
    plan.push_optional("GATEWAY_HOST", &channels.gateway_host);
    if let Some(port) = channels.gateway_port {
        plan.push("GATEWAY_PORT", port.to_string());
    }
    plan.push_optional("GATEWAY_AUTH_TOKEN", &channels.gateway_auth_token);
    if let Some(enabled) = channels.cli_enabled {
        plan.push("CLI_ENABLED", enabled.to_string());
    }

    plan.push_optional("LLM_CHEAP_MODEL", &input.providers.cheap_model);

    plan.push_optional("WEBCHAT_SKIN", &input.web_ui.skin);
    if input.web_ui.theme != "system" {
        plan.push("WEBCHAT_THEME", input.web_ui.theme.clone());
    }
    plan.push_optional("WEBCHAT_ACCENT_COLOR", &input.web_ui.accent_color);
    if !input.web_ui.show_branding {
        plan.push("WEBCHAT_SHOW_BRANDING", "false");
    }

    if input.observability_backend != "none" {
        plan.push("OBSERVABILITY_BACKEND", input.observability_backend.clone());
    }

    if input.agent.allow_local_tools {
        plan.push("ALLOW_LOCAL_TOOLS", "true");
    }
    plan.push_optional("WORKSPACE_MODE", &input.agent.workspace_mode);

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_for<'a>(plan: &'a SetupBootstrapEnvPlan, key: &str) -> Option<&'a str> {
        plan.variables()
            .iter()
            .find(|var| var.key == key)
            .map(|var| var.value.as_str())
    }

    #[test]
    fn default_input_has_empty_plan() {
        let plan = setup_bootstrap_env_plan(&SetupBootstrapEnvInput::default());

        assert!(plan.is_empty());
    }

    #[test]
    fn runtime_profile_writes_headless_markers_after_onboard_marker() {
        let input = SetupBootstrapEnvInput {
            onboard_completed: true,
            runtime_profile: Some(SetupRuntimeProfile::PiOsLite64),
            ..SetupBootstrapEnvInput::default()
        };

        let plan = setup_bootstrap_env_plan(&input);

        assert_eq!(value_for(&plan, "ONBOARD_COMPLETED"), Some("true"));
        assert_eq!(
            value_for(&plan, "THINCLAW_RUNTIME_PROFILE"),
            Some("pi-os-lite-64")
        );
        assert_eq!(value_for(&plan, "THINCLAW_HEADLESS"), Some("true"));
        let keys: Vec<&str> = plan.variables().iter().map(|var| var.key).collect();
        assert_eq!(
            keys,
            vec![
                "ONBOARD_COMPLETED",
                "THINCLAW_RUNTIME_PROFILE",
                "THINCLAW_HEADLESS"
            ]
        );
    }

    #[test]
    fn channel_mapping_preserves_existing_enabled_and_false_values() {
        let input = SetupBootstrapEnvInput {
            channels: SetupBootstrapChannelInput {
                signal_allow_from_groups: Some(String::new()),
                signal_group_allow_from: Some("group-a".to_string()),
                http_enabled: true,
                http_host: Some("0.0.0.0".to_string()),
                http_port: Some(8080),
                apple_mail_unread_only: false,
                apple_mail_mark_as_read: false,
                gateway_enabled: Some(false),
                cli_enabled: Some(false),
                ..SetupBootstrapChannelInput::default()
            },
            web_ui: SetupBootstrapWebUiInput {
                show_branding: false,
                ..SetupBootstrapWebUiInput::default()
            },
            ..SetupBootstrapEnvInput::default()
        };

        let plan = setup_bootstrap_env_plan(&input);

        assert_eq!(value_for(&plan, "HTTP_ENABLED"), Some("true"));
        assert_eq!(value_for(&plan, "HTTP_HOST"), Some("0.0.0.0"));
        assert_eq!(value_for(&plan, "HTTP_PORT"), Some("8080"));
        assert_eq!(value_for(&plan, "SIGNAL_ALLOW_FROM_GROUPS"), None);
        assert_eq!(value_for(&plan, "SIGNAL_GROUP_ALLOW_FROM"), Some("group-a"));
        assert_eq!(value_for(&plan, "APPLE_MAIL_UNREAD_ONLY"), Some("false"));
        assert_eq!(value_for(&plan, "APPLE_MAIL_MARK_AS_READ"), Some("false"));
        assert_eq!(value_for(&plan, "GATEWAY_ENABLED"), Some("false"));
        assert_eq!(value_for(&plan, "CLI_ENABLED"), Some("false"));
        assert_eq!(value_for(&plan, "WEBCHAT_SHOW_BRANDING"), Some("false"));
        assert_eq!(value_for(&plan, "WEBCHAT_THEME"), None);
    }

    #[test]
    fn quick_setup_plan_uses_documented_twelve_steps() {
        let plan = setup_wizard_plan(SetupWizardPlanInput::default());

        assert_eq!(plan.steps.len(), 12);
        assert!(
            !plan
                .steps
                .iter()
                .any(|step| step.id == SetupWizardStepId::SmartRouting)
        );
        assert!(
            plan.steps
                .iter()
                .any(|step| step.id == SetupWizardStepId::CodingWorkers)
        );
        assert_eq!(
            plan.phase(SetupWizardPhaseId::WelcomeProfile)
                .map(|phase| phase.step_ids.as_slice()),
            Some(
                [
                    SetupWizardStepId::CliSkin,
                    SetupWizardStepId::Profile,
                    SetupWizardStepId::AgentIdentity
                ]
                .as_slice()
            )
        );
    }

    #[test]
    fn guided_and_channels_only_plans_keep_expected_shape() {
        let ai_plan = setup_wizard_plan(SetupWizardPlanInput {
            channels_only: false,
            guide_topic: Some(SetupGuideTopic::Ai),
        });
        let ai_step_ids: Vec<_> = ai_plan.steps.iter().map(|step| step.id).collect();
        assert_eq!(
            ai_step_ids,
            vec![
                SetupWizardStepId::InferenceProvider,
                SetupWizardStepId::ModelSelection,
                SetupWizardStepId::SmartRouting,
                SetupWizardStepId::FallbackProviders,
                SetupWizardStepId::Embeddings,
                SetupWizardStepId::Summary,
            ]
        );

        let channels_plan = setup_wizard_plan(SetupWizardPlanInput {
            channels_only: true,
            guide_topic: Some(SetupGuideTopic::Ai),
        });
        let channel_step_ids: Vec<_> = channels_plan.steps.iter().map(|step| step.id).collect();
        assert_eq!(
            channel_step_ids,
            vec![
                SetupWizardStepId::Channels,
                SetupWizardStepId::ChannelVerification,
                SetupWizardStepId::Summary,
            ]
        );
    }

    #[test]
    fn profile_metadata_maps_headless_runtime_profiles() {
        assert_eq!(
            SetupOnboardingProfile::CustomAdvanced.title(),
            "Custom / Advanced"
        );
        assert!(
            SetupOnboardingProfile::CustomAdvanced
                .description()
                .contains("neutral baseline")
        );
        assert_eq!(
            SetupOnboardingProfile::RemoteServer.runtime_profile(),
            Some(SetupRuntimeProfile::Remote)
        );
        assert_eq!(
            SetupOnboardingProfile::PiOsLite64.runtime_profile_env_value(),
            Some("pi-os-lite-64")
        );
        assert!(SetupOnboardingProfile::RemoteServer.is_headless_remote());
        assert!(!SetupOnboardingProfile::Balanced.is_headless_remote());
    }

    #[test]
    fn runtime_command_policy_distinguishes_desktop_and_headless_profiles() {
        let desktop = SetupRuntimeCommandInput {
            profile: SetupOnboardingProfile::Balanced,
            ui_mode: SetupWizardUiMode::Tui,
            continue_to_runtime: true,
            pause_after_completion: true,
        };
        assert_eq!(setup_primary_runtime_command(desktop), "thinclaw tui");
        assert!(
            setup_runtime_handoff_summary(desktop).contains("`thinclaw tui`"),
            "desktop handoff should name the selected UI command"
        );
        assert!(
            setup_what_next_commands(desktop)
                .iter()
                .any(|command| command == "Topic guide: thinclaw onboard --guide")
        );

        let headless = SetupRuntimeCommandInput {
            profile: SetupOnboardingProfile::PiOsLite64,
            ui_mode: SetupWizardUiMode::Tui,
            continue_to_runtime: false,
            pause_after_completion: false,
        };
        assert_eq!(
            setup_primary_runtime_command(headless),
            "thinclaw run --no-onboard"
        );
        assert!(
            setup_runtime_handoff_summary(headless).contains("install/start the OS service"),
            "paused headless handoff should point at service startup"
        );
        assert!(
            setup_what_next_commands(headless)
                .iter()
                .any(|command| command == "Pi diagnostics: thinclaw doctor --profile pi-os-lite-64")
        );
    }
}
