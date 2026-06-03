//! Memory hygiene configuration.

use thinclaw_platform::resolve_thinclaw_home;
use thinclaw_types::error::ConfigError;

use crate::helpers::{parse_bool_env, parse_optional_env};

/// Memory hygiene configuration.
///
/// Controls automatic cleanup of stale workspace documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HygieneConfig {
    /// Whether hygiene is enabled. Env: `MEMORY_HYGIENE_ENABLED` (default: true).
    pub enabled: bool,
    /// Days before `daily/` documents are deleted. Env: `MEMORY_HYGIENE_RETENTION_DAYS` (default: 30).
    pub retention_days: u32,
    /// Minimum hours between hygiene passes. Env: `MEMORY_HYGIENE_CADENCE_HOURS` (default: 12).
    pub cadence_hours: u32,
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: 30,
            cadence_hours: 12,
        }
    }
}

impl HygieneConfig {
    pub fn resolve() -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("MEMORY_HYGIENE_ENABLED", true)?,
            retention_days: parse_optional_env("MEMORY_HYGIENE_RETENTION_DAYS", 30)?,
            cadence_hours: parse_optional_env("MEMORY_HYGIENE_CADENCE_HOURS", 12)?,
        })
    }

    /// Convert to the workspace hygiene runtime config, resolving the state
    /// directory to the standard `~/.thinclaw` location.
    pub fn to_workspace_config(&self) -> thinclaw_workspace::hygiene::HygieneConfig {
        thinclaw_workspace::hygiene::HygieneConfig {
            enabled: self.enabled,
            retention_days: self.retention_days,
            cadence_hours: self.cadence_hours,
            state_dir: resolve_thinclaw_home(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::lock_env;

    #[test]
    fn resolve_defaults() {
        let _guard = lock_env();
        unsafe {
            std::env::remove_var("MEMORY_HYGIENE_ENABLED");
            std::env::remove_var("MEMORY_HYGIENE_RETENTION_DAYS");
            std::env::remove_var("MEMORY_HYGIENE_CADENCE_HOURS");
        }

        assert_eq!(
            HygieneConfig::resolve().expect("hygiene"),
            HygieneConfig::default()
        );
    }

    #[test]
    fn converts_to_workspace_config() {
        let config = HygieneConfig {
            enabled: false,
            retention_days: 7,
            cadence_hours: 3,
        };

        let workspace = config.to_workspace_config();
        assert!(!workspace.enabled);
        assert_eq!(workspace.retention_days, 7);
        assert_eq!(workspace.cadence_hours, 3);
        assert!(workspace.state_dir.ends_with(".thinclaw"));
    }
}
