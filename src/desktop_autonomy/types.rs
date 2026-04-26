use super::*;

pub(crate) static GLOBAL_MANAGER: LazyLock<StdRwLock<Option<Arc<DesktopAutonomyManager>>>> =
    LazyLock::new(|| StdRwLock::new(None));

pub(crate) const MACOS_SIDECAR_FILENAME: &str = "ThinClawDesktopBridge.swift";
pub(crate) const WINDOWS_SIDECAR_FILENAME: &str = "ThinClawDesktopBridge.ps1";
pub(crate) const LINUX_SIDECAR_FILENAME: &str = "thinclaw_desktop_bridge.py";
#[cfg(target_os = "macos")]
pub(crate) const MACOS_SIDECAR_SOURCE: &str =
    include_str!("../../swift/ThinClawDesktopBridge.swift");
#[cfg(not(target_os = "macos"))]
pub(crate) const MACOS_SIDECAR_SOURCE: &str = "";
#[cfg(target_os = "windows")]
pub(crate) const WINDOWS_SIDECAR_SOURCE: &str =
    include_str!("../../desktop-sidecars/ThinClawDesktopBridge.ps1");
#[cfg(not(target_os = "windows"))]
pub(crate) const WINDOWS_SIDECAR_SOURCE: &str = "";
#[cfg(target_os = "linux")]
pub(crate) const LINUX_SIDECAR_SOURCE: &str =
    include_str!("../../desktop-sidecars/thinclaw_desktop_bridge.py");
#[cfg(not(target_os = "linux"))]
pub(crate) const LINUX_SIDECAR_SOURCE: &str = "";
pub(crate) const DEFAULT_SESSION_ID: &str = "desktop-main-session";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DesktopBridgeBackend {
    MacOsSwift,
    WindowsPowerShell,
    LinuxPython,
    Unsupported,
}

impl DesktopBridgeBackend {
    pub(super) fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Self::MacOsSwift
        }

        #[cfg(target_os = "windows")]
        {
            Self::WindowsPowerShell
        }

        #[cfg(target_os = "linux")]
        {
            Self::LinuxPython
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        {
            Self::Unsupported
        }
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::MacOsSwift => "macos_swift",
            Self::WindowsPowerShell => "windows_powershell",
            Self::LinuxPython => "linux_python",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DesktopBridgeSpec {
    pub(super) backend: DesktopBridgeBackend,
    pub(super) filename: &'static str,
    pub(super) source: &'static str,
}

impl DesktopBridgeSpec {
    pub(super) fn current() -> Self {
        match DesktopBridgeBackend::current() {
            DesktopBridgeBackend::MacOsSwift => Self {
                backend: DesktopBridgeBackend::MacOsSwift,
                filename: MACOS_SIDECAR_FILENAME,
                source: MACOS_SIDECAR_SOURCE,
            },
            DesktopBridgeBackend::WindowsPowerShell => Self {
                backend: DesktopBridgeBackend::WindowsPowerShell,
                filename: WINDOWS_SIDECAR_FILENAME,
                source: WINDOWS_SIDECAR_SOURCE,
            },
            DesktopBridgeBackend::LinuxPython => Self {
                backend: DesktopBridgeBackend::LinuxPython,
                filename: LINUX_SIDECAR_FILENAME,
                source: LINUX_SIDECAR_SOURCE,
            },
            DesktopBridgeBackend::Unsupported => Self {
                backend: DesktopBridgeBackend::Unsupported,
                filename: "unsupported-sidecar.txt",
                source: "",
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct DesktopBootstrapPrerequisites {
    pub(super) passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) blocking_reason: Option<String>,
    #[serde(default)]
    pub(super) checks: Vec<AutonomyCheckResult>,
    #[serde(default)]
    pub(super) notes: Vec<String>,
    #[serde(default)]
    pub(super) evidence: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopOcrBlock {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(default)]
    pub bounds: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopSnapshot {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
    #[serde(default)]
    pub tree: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot_path: Option<PathBuf>,
    #[serde(default)]
    pub ocr_blocks: Vec<DesktopOcrBlock>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopActionRequest {
    pub session_id: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub modifiers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_change: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopActionResult {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_change: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_snapshot: Option<DesktopSnapshot>,
    #[serde(default)]
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyStatus {
    pub enabled: bool,
    pub profile: String,
    pub deployment_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_username: Option<String>,
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    pub bootstrap_passed: bool,
    pub emergency_stop_active: bool,
    pub capture_evidence: bool,
    pub kill_switch_hotkey: String,
    pub sidecar_script_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_agent_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_bootstrap_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub code_auto_apply_paused: bool,
    #[serde(default)]
    pub session_ready: bool,
    #[serde(default)]
    pub action_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub permission_summary: serde_json::Value,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub prerequisite_summary: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyBootstrapReport {
    pub passed: bool,
    pub health: serde_json::Value,
    pub permissions: serde_json::Value,
    #[serde(default)]
    pub seeded_skills: Vec<PathBuf>,
    #[serde(default)]
    pub seeded_routines: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_agent_path: Option<PathBuf>,
    #[serde(default)]
    pub launch_agent_written: bool,
    #[serde(default)]
    pub launch_agent_loaded: bool,
    #[serde(default)]
    pub fixture_paths: DesktopFixturePaths,
    #[serde(default)]
    pub session_ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dedicated_user_keychain_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one_time_login_secret: Option<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalAutorolloutOutcome {
    pub build_id: String,
    pub build_dir: PathBuf,
    pub promoted: bool,
    #[serde(default)]
    pub checks: Vec<AutonomyCheckResult>,
    #[serde(default)]
    pub publish_metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomyCheckResult {
    pub name: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub evidence: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutonomyEventItem {
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutonomyRolloutEntry {
    pub build_id: String,
    pub proposal_id: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub promoted: bool,
    #[serde(default)]
    pub checks: Vec<AutonomyCheckResult>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutonomyRolloutSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_target_build_id: Option<String>,
    pub code_auto_apply_paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    #[serde(default)]
    pub consecutive_failed_promotions: u32,
    #[serde(default)]
    pub failed_canary_count: usize,
    #[serde(default)]
    pub recent_builds: Vec<AutonomyRolloutEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutonomyChecksSummary {
    #[serde(default)]
    pub bootstrap_checks: Vec<AutonomyCheckResult>,
    #[serde(default)]
    pub latest_canary_checks: Vec<AutonomyCheckResult>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub permission_report: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutonomyEvidenceSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_bootstrap_report: Option<AutonomyBootstrapReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_canary_report: Option<DesktopCanaryReport>,
    #[serde(default)]
    pub recent_events: Vec<AutonomyEventItem>,
    #[serde(default)]
    pub seeded_routines: Vec<String>,
    #[serde(default)]
    pub seeded_skills: Vec<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActionReadinessSnapshot {
    pub(super) action_ready: bool,
    pub(super) session_ready: bool,
    pub(super) blocking_reason: Option<String>,
    pub(super) permission_summary: serde_json::Value,
    pub(super) prerequisite_summary: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopFixturePaths {
    #[serde(default)]
    pub calendar_title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbers_doc: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages_doc: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub textedit_doc: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopCanaryManifest {
    pub build_id: String,
    pub proposal_id: String,
    pub report_path: PathBuf,
    pub shadow_home: PathBuf,
    pub session_id: String,
    pub fixture_paths: DesktopFixturePaths,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesktopCanaryReport {
    pub build_id: String,
    pub generated_at: DateTime<Utc>,
    pub passed: bool,
    pub fixture_paths: DesktopFixturePaths,
    #[serde(default)]
    pub checks: Vec<AutonomyCheckResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RuntimeState {
    pub(super) paused: bool,
    pub(super) pause_reason: Option<String>,
    pub(super) bootstrap_passed: bool,
    pub(super) last_bootstrap_at: Option<DateTime<Utc>>,
    pub(super) last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct RolloutState {
    pub(super) consecutive_failed_promotions: u32,
    pub(super) failed_canaries: Vec<DateTime<Utc>>,
    pub(super) code_auto_apply_paused: bool,
    pub(super) pause_reason: Option<String>,
    pub(super) last_promoted_build_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BuildManifest {
    pub(super) build_id: String,
    pub(super) user_id: String,
    pub(super) proposal_id: String,
    pub(super) title: String,
    pub(super) created_at: DateTime<Utc>,
    pub(super) promoted: bool,
    #[serde(default)]
    pub(super) checks: Vec<AutonomyCheckResult>,
    #[serde(default)]
    pub(super) metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct DedicatedUserBootstrap {
    pub(super) keychain_label: Option<String>,
    pub(super) session_ready: bool,
    pub(super) blocking_reason: Option<String>,
    pub(super) created_user: bool,
    pub(super) one_time_login_secret: Option<String>,
}
