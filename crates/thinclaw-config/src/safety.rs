//! Safety subsystem configuration.

use std::path::PathBuf;

use thinclaw_settings::Settings;
use thinclaw_types::error::ConfigError;

use crate::helpers::{parse_bool_env, parse_optional_env, parse_string_env};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyConfig {
    pub max_output_length: usize,
    pub injection_check_enabled: bool,
    pub redact_pii_in_prompts: bool,
    pub smart_approval_mode: String,
    pub external_scanner_mode: String,
    pub external_scanner_path: Option<PathBuf>,
    pub external_scanner_require_verified: bool,
    pub allow_temp_paths: bool,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            max_output_length: 100_000,
            injection_check_enabled: true,
            redact_pii_in_prompts: true,
            smart_approval_mode: "off".to_string(),
            external_scanner_mode: "fail_closed".to_string(),
            external_scanner_path: None,
            external_scanner_require_verified: false,
            allow_temp_paths: false,
        }
    }
}

impl SafetyConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let external_scanner_path = parse_string_env(
            "SAFETY_EXTERNAL_SCANNER_PATH",
            settings
                .safety
                .external_scanner_path
                .as_ref()
                .map(|path: &PathBuf| path.display().to_string())
                .unwrap_or_default(),
        )?;

        Ok(Self {
            max_output_length: parse_optional_env(
                "SAFETY_MAX_OUTPUT_LENGTH",
                settings.safety.max_output_length,
            )?,
            injection_check_enabled: parse_bool_env(
                "SAFETY_INJECTION_CHECK_ENABLED",
                settings.safety.injection_check_enabled,
            )?,
            redact_pii_in_prompts: parse_bool_env(
                "SAFETY_REDACT_PII_IN_PROMPTS",
                settings.safety.redact_pii_in_prompts,
            )?,
            smart_approval_mode: parse_string_env(
                "SAFETY_SMART_APPROVAL_MODE",
                settings.safety.smart_approval_mode.clone(),
            )?,
            external_scanner_mode: parse_string_env(
                "SAFETY_EXTERNAL_SCANNER_MODE",
                settings.safety.external_scanner_mode.clone(),
            )?,
            external_scanner_require_verified: parse_bool_env(
                "SAFETY_EXTERNAL_SCANNER_REQUIRE_VERIFIED",
                settings.safety.external_scanner_require_verified,
            )?,
            allow_temp_paths: parse_bool_env(
                "SAFETY_ALLOW_TEMP_PATHS",
                settings.safety.allow_temp_paths,
            )?,
            external_scanner_path: if external_scanner_path.trim().is_empty() {
                None
            } else {
                Some(PathBuf::from(external_scanner_path))
            },
        })
    }
}

impl thinclaw_safety::SafetyConfigLike for SafetyConfig {
    fn max_output_length(&self) -> usize {
        self.max_output_length
    }

    fn injection_check_enabled(&self) -> bool {
        self.injection_check_enabled
    }

    fn redact_pii_in_prompts(&self) -> bool {
        self.redact_pii_in_prompts
    }
}
