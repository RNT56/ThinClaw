use clap::ValueEnum;

/// How the onboarding UI should be presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum UiMode {
    #[default]
    Auto,
    Cli,
    Tui,
}

impl UiMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cli => "cli",
            Self::Tui => "tui",
        }
    }
}

/// Topic-oriented guided setup entry points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum GuideTopic {
    Menu,
    Ai,
    Channels,
    Agent,
    Tools,
    Automation,
    Runtime,
}

impl GuideTopic {
    pub fn title(self) -> &'static str {
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

/// High-level onboarding intent profile used to prefill recommended defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum OnboardingProfile {
    #[value(name = "balanced")]
    #[default]
    Balanced,
    #[value(name = "local-private", alias = "local-and-private")]
    LocalAndPrivate,
    #[value(name = "builder-coding", alias = "builder-and-coding")]
    BuilderAndCoding,
    #[value(name = "channel-first")]
    ChannelFirst,
    #[value(name = "remote", alias = "remote-server")]
    RemoteServer,
    #[value(name = "custom", alias = "custom-advanced")]
    CustomAdvanced,
}

impl OnboardingProfile {
    pub fn title(self) -> &'static str {
        match self {
            Self::Balanced => "Balanced",
            Self::LocalAndPrivate => "Local & Private",
            Self::BuilderAndCoding => "Builder & Coding",
            Self::ChannelFirst => "Channel-First",
            Self::RemoteServer => "Remote / SSH Host",
            Self::CustomAdvanced => "Custom / Advanced",
        }
    }

    pub fn description(self) -> &'static str {
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
            Self::CustomAdvanced => {
                "Start from a neutral baseline with minimal profile assumptions so you can choose the stack, routing, and trust boundaries step by step."
            }
        }
    }
}

/// Major onboarding phases shown in the CLI and TUI wrappers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WizardPhaseId {
    WelcomeProfile,
    CoreRuntime,
    AiStack,
    IdentityPresence,
    ChannelsContinuity,
    CapabilitiesAutomation,
    ExperienceOperations,
    Finish,
}

impl WizardPhaseId {
    pub fn title(self) -> &'static str {
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

    pub fn description(self) -> &'static str {
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

/// Individual onboarding steps. Each step still maps onto existing wizard
/// business logic so the CLI and TUI wrappers cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WizardStepId {
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

/// Public step descriptor shared by CLI and TUI views.
#[derive(Debug, Clone)]
pub struct StepDescriptor {
    pub id: WizardStepId,
    pub phase_id: WizardPhaseId,
    pub title: &'static str,
    pub description: &'static str,
    pub why_this_matters: &'static str,
    pub recommended: Option<&'static str>,
}

/// Phase descriptor used for progress navigation.
#[derive(Debug, Clone)]
pub struct WizardPhase {
    pub id: WizardPhaseId,
    pub step_ids: Vec<WizardStepId>,
}

/// Planned onboarding flow for the current run mode.
#[derive(Debug, Clone)]
pub struct WizardPlan {
    pub phases: Vec<WizardPhase>,
    pub steps: Vec<StepDescriptor>,
}

impl WizardPlan {
    pub fn total_steps(&self) -> usize {
        self.steps.len()
    }

    pub fn phase(&self, id: WizardPhaseId) -> Option<&WizardPhase> {
        self.phases.iter().find(|phase| phase.id == id)
    }

    pub fn phase_index(&self, id: WizardPhaseId) -> Option<usize> {
        self.phases.iter().position(|phase| phase.id == id)
    }
}

/// Execution status for a planned step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Skipped,
    NeedsAttention,
}

/// Validation severity shown in the onboarding sidecar summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationLevel {
    Info,
    Warning,
    Error,
}

/// Renderable validation item emitted by the wizard.
#[derive(Debug, Clone)]
pub struct ValidationItem {
    pub level: ValidationLevel,
    pub title: String,
    pub detail: String,
}

/// Renderable readiness summary for the current onboarding state.
#[derive(Debug, Clone, Default)]
pub struct ReadinessSummary {
    pub ready_now: usize,
    pub needs_attention: usize,
    pub followups: usize,
    pub headline: String,
}

/// Ephemeral follow-up draft created during the wizard and later persisted into
/// `Settings.onboarding_followups`.
#[derive(Debug, Clone)]
pub struct FollowupDraft {
    pub id: String,
    pub title: String,
    pub category: crate::settings::OnboardingFollowupCategory,
    pub status: crate::settings::OnboardingFollowupStatus,
    pub instructions: String,
    pub action_hint: Option<String>,
}
