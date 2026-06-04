use clap::ValueEnum;

pub use thinclaw_app::{
    SetupReadinessSummary as ReadinessSummary, SetupStepDescriptor as StepDescriptor,
    SetupStepStatus as StepStatus, SetupValidationItem as ValidationItem,
    SetupValidationLevel as ValidationLevel, SetupWizardPhase as WizardPhase,
    SetupWizardPhaseId as WizardPhaseId, SetupWizardPlan as WizardPlan,
    SetupWizardStepId as WizardStepId,
};

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
        self.app_mode().as_str()
    }

    pub fn app_mode(self) -> thinclaw_app::SetupWizardUiMode {
        match self {
            Self::Auto => thinclaw_app::SetupWizardUiMode::Auto,
            Self::Cli => thinclaw_app::SetupWizardUiMode::Cli,
            Self::Tui => thinclaw_app::SetupWizardUiMode::Tui,
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
        self.app_topic().title()
    }

    pub fn app_topic(self) -> thinclaw_app::SetupGuideTopic {
        match self {
            Self::Menu => thinclaw_app::SetupGuideTopic::Menu,
            Self::Ai => thinclaw_app::SetupGuideTopic::Ai,
            Self::Channels => thinclaw_app::SetupGuideTopic::Channels,
            Self::Agent => thinclaw_app::SetupGuideTopic::Agent,
            Self::Tools => thinclaw_app::SetupGuideTopic::Tools,
            Self::Automation => thinclaw_app::SetupGuideTopic::Automation,
            Self::Runtime => thinclaw_app::SetupGuideTopic::Runtime,
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
    #[value(name = "pi-os-lite-64", alias = "raspberry-pi-os-lite", alias = "pi")]
    PiOsLite64,
    #[value(name = "custom", alias = "custom-advanced")]
    CustomAdvanced,
}

impl OnboardingProfile {
    pub fn title(self) -> &'static str {
        self.app_profile().title()
    }

    pub fn description(self) -> &'static str {
        self.app_profile().description()
    }

    pub fn is_headless_remote(self) -> bool {
        self.app_profile().is_headless_remote()
    }

    pub fn runtime_profile_env_value(self) -> Option<&'static str> {
        self.app_profile().runtime_profile_env_value()
    }

    pub fn app_profile(self) -> thinclaw_app::SetupOnboardingProfile {
        match self {
            Self::Balanced => thinclaw_app::SetupOnboardingProfile::Balanced,
            Self::LocalAndPrivate => thinclaw_app::SetupOnboardingProfile::LocalAndPrivate,
            Self::BuilderAndCoding => thinclaw_app::SetupOnboardingProfile::BuilderAndCoding,
            Self::ChannelFirst => thinclaw_app::SetupOnboardingProfile::ChannelFirst,
            Self::RemoteServer => thinclaw_app::SetupOnboardingProfile::RemoteServer,
            Self::PiOsLite64 => thinclaw_app::SetupOnboardingProfile::PiOsLite64,
            Self::CustomAdvanced => thinclaw_app::SetupOnboardingProfile::CustomAdvanced,
        }
    }
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
