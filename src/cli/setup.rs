//! Canonical setup command parsing.

use clap::{Args, Subcommand, ValueEnum};

use crate::setup::{GuideTopic, OnboardingProfile, UiMode};

use super::ResetCommand;

#[derive(Args, Debug, Clone)]
pub struct SetupCommand {
    #[command(subcommand)]
    pub action: Option<SetupAction>,

    /// Continue into the local runtime after setup completes
    #[arg(long)]
    pub run: bool,

    /// Setup depth: quick recommended defaults or the complete advanced flow
    #[arg(long, value_enum, default_value_t = SetupModeArg::Quick)]
    pub mode: SetupModeArg,

    /// Skip provider authentication during this setup pass
    #[arg(long)]
    pub skip_provider_auth: bool,

    /// Deprecated alias for --skip-provider-auth
    #[arg(long = "skip-auth", hide = true)]
    pub legacy_skip_auth: bool,

    #[arg(long, value_enum, default_value_t = UiMode::Auto)]
    pub ui: UiMode,

    #[arg(long, value_enum)]
    pub profile: Option<OnboardingProfile>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupModeArg {
    Quick,
    Advanced,
}

impl From<SetupModeArg> for thinclaw_app::SetupMode {
    fn from(value: SetupModeArg) -> Self {
        match value {
            SetupModeArg::Quick => Self::Quick,
            SetupModeArg::Advanced => Self::Advanced,
        }
    }
}

#[derive(Subcommand, Debug, Clone)]
pub enum SetupAction {
    /// Edit one focused setup area
    Edit {
        #[arg(value_enum, default_value_t = GuideTopic::Menu)]
        topic: GuideTopic,
    },
    /// Reset selected ThinClaw-owned state
    Reset(ResetCommand),
}
