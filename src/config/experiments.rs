use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Optional experiments subsystem configuration.
#[derive(Debug, Clone)]
pub struct ExperimentsConfig {
    pub enabled: bool,
    pub max_concurrent_campaigns: usize,
    pub default_artifact_retention_days: u32,
    pub allow_remote_runners: bool,
    pub ui_visibility: String,
    pub default_promotion_mode: String,
}

impl Default for ExperimentsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_concurrent_campaigns: 1,
            default_artifact_retention_days: 30,
            allow_remote_runners: true,
            ui_visibility: "hidden_until_enabled".to_string(),
            default_promotion_mode: "branch_pr_draft".to_string(),
        }
    }
}

impl ExperimentsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("EXPERIMENTS_ENABLED", settings.experiments.enabled)?,
            max_concurrent_campaigns: parse_optional_env(
                "EXPERIMENTS_MAX_CONCURRENT_CAMPAIGNS",
                settings.experiments.max_concurrent_campaigns as usize,
            )?,
            default_artifact_retention_days: parse_optional_env(
                "EXPERIMENTS_ARTIFACT_RETENTION_DAYS",
                settings.experiments.default_artifact_retention_days,
            )?,
            allow_remote_runners: parse_bool_env(
                "EXPERIMENTS_ALLOW_REMOTE_RUNNERS",
                settings.experiments.allow_remote_runners,
            )?,
            ui_visibility: std::env::var("EXPERIMENTS_UI_VISIBILITY")
                .ok()
                .unwrap_or_else(|| settings.experiments.ui_visibility.clone()),
            default_promotion_mode: std::env::var("EXPERIMENTS_DEFAULT_PROMOTION_MODE")
                .ok()
                .unwrap_or_else(|| settings.experiments.default_promotion_mode.clone()),
        })
    }
}
