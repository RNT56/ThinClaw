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
pub use wizard::{GuideTopic, OnboardingProfile, SetupConfig, SetupWizard, UiMode};

/// Check if onboarding is needed and return the reason.
///
/// Returns `Some(reason)` when the operator should be prompted (or blocked)
/// because onboarding has not been completed yet, or `None` when the system
/// is ready to start normally.
#[cfg(any(feature = "postgres", feature = "libsql"))]
pub fn check_onboard_needed(toml_path: Option<&std::path::Path>, no_db: bool) -> Option<String> {
    use std::path::{Path, PathBuf};

    use crate::settings::Settings;

    if no_db {
        return None;
    }

    let mut settings = Settings::load();
    let resolved_toml_path: PathBuf = toml_path
        .map(PathBuf::from)
        .unwrap_or_else(Settings::default_toml_path);
    if let Ok(Some(toml_settings)) = Settings::load_toml(&resolved_toml_path) {
        settings.merge_from(&toml_settings);
    }

    let bootstrap_path = crate::bootstrap::thinclaw_env_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("bootstrap.json");
    let bootstrap_json: serde_json::Value = std::fs::read_to_string(&bootstrap_path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or(serde_json::Value::Null);

    let env_backend = std::env::var("DATABASE_BACKEND").ok();
    let settings_backend = settings.database_backend.clone();
    let configured_libsql = matches!(
        env_backend.as_deref().or(settings_backend.as_deref()),
        Some("libsql" | "sqlite" | "turso")
    );

    let has_db = std::env::var("DATABASE_URL").is_ok()
        || std::env::var("LIBSQL_PATH").is_ok()
        || std::env::var("LIBSQL_URL").is_ok()
        || settings.database_url.is_some()
        || settings.libsql_path.is_some()
        || settings.libsql_url.is_some()
        || bootstrap_json
            .get("database_url")
            .and_then(|value| value.as_str())
            .is_some()
        || configured_libsql
        || crate::config::default_libsql_path().exists();

    if !has_db {
        return Some("Database not configured".to_string());
    }

    let onboard_completed = std::env::var("ONBOARD_COMPLETED")
        .map(|v| v == "true")
        .unwrap_or(false)
        || settings.onboard_completed
        || bootstrap_json
            .get("onboard_completed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

    if onboard_completed {
        return None;
    }

    // No explicit completion marker — treat as first run
    Some("First run".to_string())
}
