use crate::config::helpers::{parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Safety configuration.
#[derive(Debug, Clone)]
pub struct SafetyConfig {
    pub max_output_length: usize,
    pub injection_check_enabled: bool,
}

impl SafetyConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            max_output_length: parse_optional_env(
                "SAFETY_MAX_OUTPUT_LENGTH",
                settings.safety.max_output_length,
            )?,
            injection_check_enabled: parse_bool_env(
                "SAFETY_INJECTION_CHECK_ENABLED",
                settings.safety.injection_check_enabled,
            )?,
        })
    }
}
