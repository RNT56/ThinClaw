//! Interactive setup wizard for ThinClaw.
//!
//! Provides a guided setup experience for ThinClaw's core runtime, AI stack,
//! channels, capabilities, and operator-facing experience.
//!
//! The onboarding flow now supports both:
//! - an upgraded prompt-based terminal wizard
//! - a `ratatui` onboarding shell selected via `--ui tui` or `--ui auto`
//!
//! # Example
//!
//! ```ignore
//! use thinclaw::setup::SetupWizard;
//!
//! let mut wizard = SetupWizard::new();
//! wizard.run().await?;
//! ```

mod channels;
mod prompts;
#[cfg(any(feature = "postgres", feature = "libsql"))]
mod wizard;

pub use channels::{
    ChannelSetupError, SecretsContext, setup_http, setup_telegram, setup_tunnel,
    validate_telegram_token,
};
pub use prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_phase_banner,
    print_step, print_success, print_warning, secret_input, select_many, select_one,
};
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub use wizard::{OnboardingProfile, SetupConfig, SetupWizard, UiMode};
