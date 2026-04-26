use std::path::PathBuf;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::{
    DesktopAutonomyProfile, DesktopAutonomySettings, DesktopDeploymentMode, Settings,
};

#[derive(Debug, Clone)]
pub struct DesktopAutonomyConfig {
    pub enabled: bool,
    pub profile: DesktopAutonomyProfile,
    pub deployment_mode: DesktopDeploymentMode,
    pub target_username: Option<String>,
    pub desktop_max_concurrent_jobs: usize,
    pub desktop_action_timeout_secs: u64,
    pub capture_evidence: bool,
    pub emergency_stop_path: PathBuf,
    pub pause_on_bootstrap_failure: bool,
    pub kill_switch_hotkey: String,
}

impl DesktopAutonomyConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let defaults = &settings.desktop_autonomy;
        let profile = resolve_profile(defaults)?;
        let enabled = parse_bool_env("DESKTOP_AUTONOMY_ENABLED", defaults.enabled)?;
        let deployment_mode = resolve_deployment_mode(defaults)?;
        let target_username = optional_env("DESKTOP_AUTONOMY_TARGET_USERNAME")?
            .or_else(|| defaults.target_username.clone());
        let emergency_stop_raw = optional_env("DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH")?
            .unwrap_or_else(|| defaults.emergency_stop_path.clone());

        Ok(Self {
            enabled,
            profile,
            deployment_mode,
            target_username,
            desktop_max_concurrent_jobs: parse_optional_env(
                "DESKTOP_AUTONOMY_MAX_CONCURRENT_JOBS",
                defaults.desktop_max_concurrent_jobs,
            )?,
            desktop_action_timeout_secs: parse_optional_env(
                "DESKTOP_AUTONOMY_ACTION_TIMEOUT_SECS",
                defaults.desktop_action_timeout_secs,
            )?,
            capture_evidence: parse_bool_env(
                "DESKTOP_AUTONOMY_CAPTURE_EVIDENCE",
                defaults.capture_evidence,
            )?,
            emergency_stop_path: crate::platform::expand_home_dir(&emergency_stop_raw),
            pause_on_bootstrap_failure: parse_bool_env(
                "DESKTOP_AUTONOMY_PAUSE_ON_BOOTSTRAP_FAILURE",
                defaults.pause_on_bootstrap_failure,
            )?,
            kill_switch_hotkey: optional_env("DESKTOP_AUTONOMY_KILL_SWITCH_HOTKEY")?
                .unwrap_or_else(|| defaults.kill_switch_hotkey.clone()),
        })
    }

    pub fn is_reckless_enabled(&self) -> bool {
        self.enabled && matches!(self.profile, DesktopAutonomyProfile::RecklessDesktop)
    }
}

fn resolve_profile(
    defaults: &DesktopAutonomySettings,
) -> Result<DesktopAutonomyProfile, ConfigError> {
    match optional_env("DESKTOP_AUTONOMY_PROFILE")? {
        Some(value) => parse_profile(&value),
        None => Ok(defaults.profile),
    }
}

fn resolve_deployment_mode(
    defaults: &DesktopAutonomySettings,
) -> Result<DesktopDeploymentMode, ConfigError> {
    match optional_env("DESKTOP_AUTONOMY_DEPLOYMENT_MODE")? {
        Some(value) => parse_deployment_mode(&value),
        None => Ok(defaults.deployment_mode),
    }
}

fn parse_profile(value: &str) -> Result<DesktopAutonomyProfile, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "off" => Ok(DesktopAutonomyProfile::Off),
        "reckless_desktop" | "reckless-desktop" => Ok(DesktopAutonomyProfile::RecklessDesktop),
        other => Err(ConfigError::InvalidValue {
            key: "DESKTOP_AUTONOMY_PROFILE".to_string(),
            message: format!(
                "unsupported desktop autonomy profile '{other}' (expected off or reckless_desktop)"
            ),
        }),
    }
}

fn parse_deployment_mode(value: &str) -> Result<DesktopDeploymentMode, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "whole_machine_admin" | "whole-machine-admin" => {
            Ok(DesktopDeploymentMode::WholeMachineAdmin)
        }
        "dedicated_user" | "dedicated-user" => Ok(DesktopDeploymentMode::DedicatedUser),
        other => Err(ConfigError::InvalidValue {
            key: "DESKTOP_AUTONOMY_DEPLOYMENT_MODE".to_string(),
            message: format!(
                "unsupported deployment mode '{other}' (expected whole_machine_admin or dedicated_user)"
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_defaults_from_settings() {
        let settings = Settings::default();
        let cfg = DesktopAutonomyConfig::resolve(&settings).expect("desktop autonomy config");
        assert!(!cfg.enabled);
        assert_eq!(cfg.profile, DesktopAutonomyProfile::Off);
        assert_eq!(
            cfg.deployment_mode,
            DesktopDeploymentMode::WholeMachineAdmin
        );
        assert!(
            cfg.emergency_stop_path
                .ends_with(".thinclaw/AUTONOMY_DISABLED")
        );
    }
}
