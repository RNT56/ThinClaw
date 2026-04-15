use std::path::PathBuf;

use crate::config::helpers::{optional_env, parse_bool_env, parse_optional_env};
use crate::error::ConfigError;
use crate::settings::{Settings, SkillTapConfig, WellKnownSkillRegistryConfig};

/// Skills system configuration.
#[derive(Debug, Clone)]
pub struct SkillsConfig {
    /// Whether the skills system is enabled.
    pub enabled: bool,
    /// Directory containing user-placed skills (default: ~/.thinclaw/skills/).
    /// Skills here are loaded with `Trusted` trust level.
    pub local_dir: PathBuf,
    /// Directory containing registry-installed skills (default: ~/.thinclaw/installed_skills/).
    /// Skills here are loaded with `Installed` trust level and get read-only tool access.
    pub installed_dir: PathBuf,
    /// Maximum number of skills that can be active simultaneously.
    pub max_active_skills: usize,
    /// Maximum total context tokens allocated to skill prompts.
    pub max_context_tokens: usize,
    /// Additional GitHub taps used for skill federation.
    pub skill_taps: Vec<SkillTapConfig>,
    /// Additional `/.well-known/skills` registries.
    pub well_known_skill_registries: Vec<WellKnownSkillRegistryConfig>,
    /// Quarantine directory for externally sourced skills before installation.
    pub quarantine_dir: PathBuf,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            local_dir: default_skills_dir(),
            installed_dir: default_installed_skills_dir(),
            max_active_skills: 3,
            max_context_tokens: 4000,
            skill_taps: Vec::new(),
            well_known_skill_registries: Vec::new(),
            quarantine_dir: default_quarantine_dir(),
        }
    }
}

/// Get the default user skills directory (~/.thinclaw/skills/).
fn default_skills_dir() -> PathBuf {
    crate::platform::state_paths().skills_dir
}

/// Get the default installed skills directory (~/.thinclaw/installed_skills/).
fn default_installed_skills_dir() -> PathBuf {
    crate::platform::state_paths().installed_skills_dir
}

fn default_quarantine_dir() -> PathBuf {
    crate::platform::resolve_data_dir("skills_quarantine")
}

impl SkillsConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            enabled: parse_bool_env("SKILLS_ENABLED", settings.skills_enabled)?,
            local_dir: optional_env("SKILLS_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_skills_dir),
            installed_dir: optional_env("SKILLS_INSTALLED_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_installed_skills_dir),
            max_active_skills: parse_optional_env("SKILLS_MAX_ACTIVE", 3)?,
            max_context_tokens: parse_optional_env("SKILLS_MAX_CONTEXT_TOKENS", 4000)?,
            skill_taps: settings.skill_taps.clone(),
            well_known_skill_registries: settings.well_known_skill_registries.clone(),
            quarantine_dir: optional_env("SKILLS_QUARANTINE_DIR")?
                .map(PathBuf::from)
                .unwrap_or_else(default_quarantine_dir),
        })
    }
}
