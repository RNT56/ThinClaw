use super::*;

fn default_true() -> bool {
    true
}

fn default_desktop_emergency_stop_path() -> String {
    "~/.thinclaw/AUTONOMY_DISABLED".to_string()
}

fn default_desktop_max_concurrent_jobs() -> usize {
    1
}

fn default_desktop_action_timeout_secs() -> u64 {
    60
}

fn default_desktop_kill_switch_hotkey() -> String {
    "ctrl+option+command+period".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DesktopAutonomyProfile {
    #[default]
    Off,
    RecklessDesktop,
}

impl DesktopAutonomyProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::RecklessDesktop => "reckless_desktop",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DesktopDeploymentMode {
    #[default]
    WholeMachineAdmin,
    DedicatedUser,
}

impl DesktopDeploymentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WholeMachineAdmin => "whole_machine_admin",
            Self::DedicatedUser => "dedicated_user",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopAutonomySettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub profile: DesktopAutonomyProfile,
    #[serde(default)]
    pub deployment_mode: DesktopDeploymentMode,
    #[serde(default)]
    pub target_username: Option<String>,
    #[serde(default = "default_desktop_max_concurrent_jobs")]
    pub desktop_max_concurrent_jobs: usize,
    #[serde(default = "default_desktop_action_timeout_secs")]
    pub desktop_action_timeout_secs: u64,
    #[serde(default = "default_true")]
    pub capture_evidence: bool,
    #[serde(default = "default_desktop_emergency_stop_path")]
    pub emergency_stop_path: String,
    #[serde(default = "default_true")]
    pub pause_on_bootstrap_failure: bool,
    #[serde(default = "default_desktop_kill_switch_hotkey")]
    pub kill_switch_hotkey: String,
}

impl DesktopAutonomySettings {
    pub fn is_reckless_enabled(&self) -> bool {
        self.enabled && matches!(self.profile, DesktopAutonomyProfile::RecklessDesktop)
    }
}

impl Default for DesktopAutonomySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            profile: DesktopAutonomyProfile::Off,
            deployment_mode: DesktopDeploymentMode::WholeMachineAdmin,
            target_username: None,
            desktop_max_concurrent_jobs: default_desktop_max_concurrent_jobs(),
            desktop_action_timeout_secs: default_desktop_action_timeout_secs(),
            capture_evidence: true,
            emergency_stop_path: default_desktop_emergency_stop_path(),
            pause_on_bootstrap_failure: true,
            kill_switch_hotkey: default_desktop_kill_switch_hotkey(),
        }
    }
}
