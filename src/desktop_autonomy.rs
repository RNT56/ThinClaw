use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, RwLock as StdRwLock};

use chrono::{DateTime, Utc};
use rand::Rng;
use secrecy::ExposeSecret;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use uuid::Uuid;

use crate::agent::routine::{
    NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger, canonicalize_schedule_expr,
    next_schedule_fire_for_user,
};
use crate::config::{DatabaseConfig, DesktopAutonomyConfig};
use crate::db::Database;
use crate::tools::ToolProfile;

static GLOBAL_MANAGER: LazyLock<StdRwLock<Option<Arc<DesktopAutonomyManager>>>> =
    LazyLock::new(|| StdRwLock::new(None));

const MACOS_SIDECAR_FILENAME: &str = "ThinClawDesktopBridge.swift";
const WINDOWS_SIDECAR_FILENAME: &str = "ThinClawDesktopBridge.ps1";
const LINUX_SIDECAR_FILENAME: &str = "thinclaw_desktop_bridge.py";
#[cfg(target_os = "macos")]
const MACOS_SIDECAR_SOURCE: &str = include_str!("../swift/ThinClawDesktopBridge.swift");
#[cfg(not(target_os = "macos"))]
const MACOS_SIDECAR_SOURCE: &str = "";
#[cfg(target_os = "windows")]
const WINDOWS_SIDECAR_SOURCE: &str = include_str!("../desktop-sidecars/ThinClawDesktopBridge.ps1");
#[cfg(not(target_os = "windows"))]
const WINDOWS_SIDECAR_SOURCE: &str = "";
#[cfg(target_os = "linux")]
const LINUX_SIDECAR_SOURCE: &str = include_str!("../desktop-sidecars/thinclaw_desktop_bridge.py");
#[cfg(not(target_os = "linux"))]
const LINUX_SIDECAR_SOURCE: &str = "";
const DEFAULT_SESSION_ID: &str = "desktop-main-session";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DesktopBridgeBackend {
    MacOsSwift,
    WindowsPowerShell,
    LinuxPython,
    Unsupported,
}

impl DesktopBridgeBackend {
    fn current() -> Self {
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

    fn as_str(self) -> &'static str {
        match self {
            Self::MacOsSwift => "macos_swift",
            Self::WindowsPowerShell => "windows_powershell",
            Self::LinuxPython => "linux_python",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct DesktopBridgeSpec {
    backend: DesktopBridgeBackend,
    filename: &'static str,
    source: &'static str,
}

impl DesktopBridgeSpec {
    fn current() -> Self {
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
struct DesktopBootstrapPrerequisites {
    passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocking_reason: Option<String>,
    #[serde(default)]
    checks: Vec<AutonomyCheckResult>,
    #[serde(default)]
    notes: Vec<String>,
    #[serde(default)]
    evidence: serde_json::Value,
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
struct ActionReadinessSnapshot {
    action_ready: bool,
    session_ready: bool,
    blocking_reason: Option<String>,
    permission_summary: serde_json::Value,
    prerequisite_summary: serde_json::Value,
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
struct RuntimeState {
    paused: bool,
    pause_reason: Option<String>,
    bootstrap_passed: bool,
    last_bootstrap_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RolloutState {
    consecutive_failed_promotions: u32,
    failed_canaries: Vec<DateTime<Utc>>,
    code_auto_apply_paused: bool,
    pause_reason: Option<String>,
    last_promoted_build_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BuildManifest {
    build_id: String,
    user_id: String,
    proposal_id: String,
    title: String,
    created_at: DateTime<Utc>,
    promoted: bool,
    #[serde(default)]
    checks: Vec<AutonomyCheckResult>,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
struct DedicatedUserBootstrap {
    keychain_label: Option<String>,
    session_ready: bool,
    blocking_reason: Option<String>,
    created_user: bool,
    one_time_login_secret: Option<String>,
}

#[derive(Debug)]
pub struct DesktopSessionManager {
    max_concurrent_jobs: usize,
    sessions: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl DesktopSessionManager {
    pub fn new(max_concurrent_jobs: usize) -> Self {
        Self {
            max_concurrent_jobs: max_concurrent_jobs.max(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn acquire(&self, session_id: &str) -> Result<DesktopSessionLease, String> {
        let semaphore = {
            let mut guard = self.sessions.lock().await;
            guard
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_concurrent_jobs)))
                .clone()
        };
        let permit = semaphore
            .acquire_owned()
            .await
            .map_err(|_| "desktop session manager is shutting down".to_string())?;
        Ok(DesktopSessionLease {
            session_id: session_id.to_string(),
            _permit: permit,
        })
    }
}

#[derive(Debug)]
pub struct DesktopSessionLease {
    session_id: String,
    _permit: OwnedSemaphorePermit,
}

impl DesktopSessionLease {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

pub struct DesktopAutonomyManager {
    config: DesktopAutonomyConfig,
    database_config: Option<DatabaseConfig>,
    store: Option<Arc<dyn Database>>,
    session_manager: DesktopSessionManager,
    state_root: PathBuf,
    sidecar_script_path: PathBuf,
    runtime_state: RwLock<RuntimeState>,
}

impl DesktopAutonomyManager {
    pub fn new(
        config: DesktopAutonomyConfig,
        database_config: Option<DatabaseConfig>,
        store: Option<Arc<dyn Database>>,
    ) -> Self {
        let state_root = crate::platform::state_paths().home.join("autonomy");
        let bridge_spec = DesktopBridgeSpec::current();
        let runtime_state = load_json_file::<RuntimeState>(&state_root.join("runtime_state.json"))
            .unwrap_or_default();
        Self {
            database_config,
            store,
            session_manager: DesktopSessionManager::new(config.desktop_max_concurrent_jobs),
            sidecar_script_path: state_root.join(bridge_spec.filename),
            state_root,
            config,
            runtime_state: RwLock::new(runtime_state),
        }
    }

    pub fn config(&self) -> &DesktopAutonomyConfig {
        &self.config
    }

    pub fn is_reckless_enabled(&self) -> bool {
        self.config.is_reckless_enabled()
    }

    fn bridge_spec(&self) -> DesktopBridgeSpec {
        DesktopBridgeSpec::current()
    }

    fn bridge_backend(&self) -> DesktopBridgeBackend {
        self.bridge_spec().backend
    }

    fn platform_label(&self) -> &'static str {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => "macos",
            DesktopBridgeBackend::WindowsPowerShell => "windows",
            DesktopBridgeBackend::LinuxPython => "linux",
            DesktopBridgeBackend::Unsupported => "unsupported",
        }
    }

    fn provider_matrix(&self) -> serde_json::Value {
        let generic_ui = self.generic_ui_provider();
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => serde_json::json!({
                "calendar": "calendar",
                "numbers": "numbers",
                "pages": "pages",
                "generic_ui": "textedit",
                "launcher": "launch_agent",
            }),
            DesktopBridgeBackend::WindowsPowerShell => serde_json::json!({
                "calendar": "outlook",
                "numbers": "excel",
                "pages": "word",
                "generic_ui": generic_ui,
                "launcher": "scheduled_task",
            }),
            DesktopBridgeBackend::LinuxPython => serde_json::json!({
                "calendar": "evolution",
                "numbers": "libreoffice_calc",
                "pages": "libreoffice_writer",
                "generic_ui": generic_ui,
                "launcher": "desktop_autostart",
            }),
            DesktopBridgeBackend::Unsupported => serde_json::json!({}),
        }
    }

    fn fixture_extensions(&self) -> (&'static str, &'static str) {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => ("numbers", "pages"),
            DesktopBridgeBackend::WindowsPowerShell => ("xlsx", "docx"),
            DesktopBridgeBackend::LinuxPython => ("ods", "odt"),
            DesktopBridgeBackend::Unsupported => ("numbers", "pages"),
        }
    }

    fn generic_ui_provider(&self) -> String {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => "textedit".to_string(),
            DesktopBridgeBackend::WindowsPowerShell => "notepad".to_string(),
            DesktopBridgeBackend::LinuxPython => {
                if command_on_path("gedit") {
                    "gedit".to_string()
                } else if command_on_path("xdg-text-editor") {
                    "xdg-text-editor".to_string()
                } else {
                    "gedit".to_string()
                }
            }
            DesktopBridgeBackend::Unsupported => "generic-editor".to_string(),
        }
    }

    fn generic_ui_target(&self) -> (String, String) {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => {
                ("com.apple.TextEdit".to_string(), "TextEdit".to_string())
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                ("notepad.exe".to_string(), "Notepad".to_string())
            }
            DesktopBridgeBackend::LinuxPython => {
                let provider = self.generic_ui_provider();
                let label = if provider == "xdg-text-editor" {
                    "xdg-text-editor"
                } else {
                    "gedit"
                };
                (provider, label.to_string())
            }
            DesktopBridgeBackend::Unsupported => {
                ("generic-editor".to_string(), "generic-editor".to_string())
            }
        }
    }

    fn attach_runtime_evidence(
        &self,
        capability: &str,
        mut evidence: serde_json::Value,
    ) -> serde_json::Value {
        if !evidence.is_object() {
            evidence = serde_json::json!({});
        }
        if let Some(obj) = evidence.as_object_mut() {
            obj.insert(
                "platform".to_string(),
                serde_json::json!(self.platform_label()),
            );
            obj.insert(
                "bridge_backend".to_string(),
                serde_json::json!(self.bridge_backend().as_str()),
            );
            obj.insert("providers".to_string(), self.provider_matrix());
            obj.insert("capability".to_string(), serde_json::json!(capability));
            obj.insert(
                "deployment_mode".to_string(),
                serde_json::json!(self.config.deployment_mode.as_str()),
            );
        }
        evidence
    }

    fn runtime_passed_check(
        &self,
        name: &str,
        detail: Option<serde_json::Value>,
        evidence: serde_json::Value,
    ) -> AutonomyCheckResult {
        passed_check(name, detail, self.attach_runtime_evidence(name, evidence))
    }

    fn runtime_failed_check(
        &self,
        name: &str,
        detail: impl Into<String>,
        evidence: serde_json::Value,
    ) -> AutonomyCheckResult {
        failed_check(
            name,
            detail.into(),
            self.attach_runtime_evidence(name, evidence),
        )
    }

    pub fn default_session_id(&self) -> String {
        match self.config.target_username.as_deref() {
            Some(username) if !username.trim().is_empty() => format!("desktop-session:{username}"),
            _ => DEFAULT_SESSION_ID.to_string(),
        }
    }

    pub async fn acquire_session_lease(
        &self,
        session_id: Option<&str>,
    ) -> Result<DesktopSessionLease, String> {
        let resolved = session_id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(DEFAULT_SESSION_ID);
        self.session_manager.acquire(resolved).await
    }

    pub async fn ensure_can_run(&self) -> Result<(), String> {
        let readiness = self.evaluate_action_readiness(true).await;
        if readiness.action_ready {
            Ok(())
        } else {
            Err(readiness
                .blocking_reason
                .unwrap_or_else(|| "desktop autonomy is not ready".to_string()))
        }
    }

    pub fn emergency_stop_active(&self) -> bool {
        self.config.emergency_stop_path.exists()
    }

    pub async fn pause(&self, reason: Option<String>) {
        let snapshot = {
            let mut state = self.runtime_state.write().await;
            state.paused = true;
            state.pause_reason = reason;
            state.clone()
        };
        if let Err(err) = self.persist_runtime_state(&snapshot).await {
            tracing::warn!(error = %err, "failed to persist autonomy pause state");
        }
    }

    pub async fn resume(&self) -> Result<(), String> {
        if self.emergency_stop_active() {
            return Err(format!(
                "cannot resume while emergency stop file exists at {}",
                self.config.emergency_stop_path.display()
            ));
        }
        let snapshot = {
            let mut state = self.runtime_state.write().await;
            state.paused = false;
            state.pause_reason = None;
            state.clone()
        };
        self.persist_runtime_state(&snapshot).await?;
        Ok(())
    }

    pub async fn status(&self) -> AutonomyStatus {
        let state = self.runtime_state.read().await.clone();
        let rollout = self.load_rollout_state().await.unwrap_or_default();
        let readiness = self.evaluate_action_readiness(false).await;
        AutonomyStatus {
            enabled: self.config.enabled,
            profile: self.config.profile.as_str().to_string(),
            deployment_mode: self.config.deployment_mode.as_str().to_string(),
            target_username: self.config.target_username.clone(),
            paused: state.paused,
            pause_reason: state.pause_reason.clone(),
            bootstrap_passed: state.bootstrap_passed,
            emergency_stop_active: self.emergency_stop_active(),
            capture_evidence: self.config.capture_evidence,
            kill_switch_hotkey: self.config.kill_switch_hotkey.clone(),
            sidecar_script_path: self.sidecar_script_path.clone(),
            launch_agent_path: self.session_launcher_path().ok(),
            current_build_id: self.current_build_id(),
            last_bootstrap_at: state.last_bootstrap_at,
            last_error: state.last_error.clone(),
            code_auto_apply_paused: rollout.code_auto_apply_paused,
            session_ready: readiness.session_ready,
            action_ready: readiness.action_ready,
            blocking_reason: readiness.blocking_reason,
            permission_summary: readiness.permission_summary,
            prerequisite_summary: readiness.prerequisite_summary,
        }
    }

    pub async fn desktop_permission_status(&self) -> Result<serde_json::Value, String> {
        self.ensure_sidecar_script().await?;
        self.bridge_call("permissions", serde_json::json!({})).await
    }

    pub async fn apps_action(
        &self,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.domain_action("apps", action, payload).await
    }

    pub async fn ui_action(
        &self,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.domain_action("ui", action, payload).await
    }

    pub async fn screen_action(
        &self,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.domain_action("screen", action, payload).await
    }

    pub async fn calendar_action(
        &self,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.domain_action("calendar", action, payload).await
    }

    pub async fn numbers_action(
        &self,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        validate_numbers_payload(action, &payload)?;
        self.domain_action("numbers", action, payload).await
    }

    pub async fn pages_action(
        &self,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.domain_action("pages", action, payload).await
    }

    pub async fn bootstrap(&self) -> Result<AutonomyBootstrapReport, String> {
        self.ensure_dirs().await?;
        self.ensure_sidecar_script().await?;

        let mut health = self.bridge_call("health", serde_json::json!({})).await?;
        health = self.attach_runtime_evidence("bridge_health", health);
        let mut permissions = self.desktop_permission_status().await?;
        permissions = self.attach_runtime_evidence("permissions", permissions);
        let prerequisites = self.platform_bootstrap_prerequisites().await;
        if let Some(obj) = health.as_object_mut() {
            obj.insert(
                "prerequisites".to_string(),
                serde_json::to_value(&prerequisites).unwrap_or(serde_json::Value::Null),
            );
            obj.insert(
                "platform".to_string(),
                serde_json::json!(self.platform_label()),
            );
            obj.insert(
                "bridge_backend".to_string(),
                serde_json::json!(self.bridge_backend().as_str()),
            );
            obj.insert("providers".to_string(), self.provider_matrix());
        }
        let dedicated = self.prepare_dedicated_user_bootstrap().await?;
        let fixture_paths =
            if dedicated.blocking_reason.is_none() && prerequisites.blocking_reason.is_none() {
                self.ensure_canary_fixtures().await.unwrap_or_else(|err| {
                    tracing::warn!(error = %err, "desktop autonomy fixture bootstrap skipped");
                    DesktopFixturePaths::default()
                })
            } else {
                DesktopFixturePaths::default()
            };
        let seeded_skills = self.seed_default_skills().await?;
        let seeded_routines = self.seed_default_routines().await?;

        let mut notes = Vec::new();
        notes.extend(prerequisites.notes.clone());
        let mut blocking_reason = prerequisites
            .blocking_reason
            .clone()
            .or_else(|| dedicated.blocking_reason.clone());
        let session_ready = match self.config.deployment_mode {
            crate::settings::DesktopDeploymentMode::WholeMachineAdmin => true,
            crate::settings::DesktopDeploymentMode::DedicatedUser => dedicated.session_ready,
        };
        let mut passed = bridge_report_passed(&health)
            && permissions_report_passed(&permissions)
            && prerequisites.passed
            && blocking_reason.is_none();
        let mut launch_agent_written = false;
        let mut launch_agent_loaded = false;
        let mut launch_agent_path = None;

        if blocking_reason.as_deref() == Some("requires_privileged_bootstrap") {
            notes.push(
                "session launcher installation deferred until the dedicated user can be created"
                    .to_string(),
            );
        } else {
            match self.write_session_launcher().await {
                Ok(path) => {
                    launch_agent_written = true;
                    launch_agent_path = Some(path.clone());
                    if session_ready {
                        match self.activate_session_launcher(&path).await {
                            Ok(()) => {
                                launch_agent_loaded = true;
                            }
                            Err(err) => {
                                passed = false;
                                blocking_reason.get_or_insert_with(|| {
                                    "session_launcher_install_failed".to_string()
                                });
                                notes.push(format!("session launcher bootstrap skipped: {err}"));
                            }
                        }
                    } else {
                        notes.push(
                            "session launcher written but not loaded because the target GUI session is not ready"
                                .to_string(),
                        );
                    }
                }
                Err(err) => {
                    passed = false;
                    blocking_reason
                        .get_or_insert_with(|| "session_launcher_install_failed".to_string());
                    notes.push(format!("session launcher not written: {err}"));
                }
            }
        }

        let runtime_snapshot = {
            let mut state = self.runtime_state.write().await;
            state.bootstrap_passed = passed;
            state.last_bootstrap_at = Some(Utc::now());
            state.last_error = (!passed).then_some(
                "desktop autonomy bootstrap reported missing permissions or bridge health issues"
                    .to_string(),
            );
            if !passed && self.config.pause_on_bootstrap_failure {
                state.paused = true;
                state.pause_reason =
                    Some("bootstrap failed; desktop autonomy paused pending operator fix".into());
            }
            state.clone()
        };
        self.persist_runtime_state(&runtime_snapshot).await?;
        let report = AutonomyBootstrapReport {
            passed,
            health,
            permissions,
            seeded_skills,
            seeded_routines,
            launch_agent_path,
            launch_agent_written,
            launch_agent_loaded,
            fixture_paths,
            session_ready,
            blocking_reason,
            dedicated_user_keychain_label: dedicated.keychain_label,
            one_time_login_secret: dedicated.one_time_login_secret,
            notes,
        };
        self.persist_bootstrap_report(&report).await?;
        Ok(report)
    }

    pub async fn rollback(&self) -> Result<serde_json::Value, String> {
        self.ensure_dirs().await?;
        let current = self.current_build_id();
        let manifests = self.list_build_manifests().await?;
        let Some(target) = manifests
            .iter()
            .filter(|manifest| manifest.promoted)
            .find(|manifest| current.as_deref() != Some(manifest.build_id.as_str()))
        else {
            return Err("no previous promoted build available for rollback".to_string());
        };

        let build_dir = self.builds_dir().join(&target.build_id);
        self.promote_build(&build_dir).await?;

        let mut rollout = self.load_rollout_state().await.unwrap_or_default();
        rollout.last_promoted_build_id = Some(target.build_id.clone());
        self.save_rollout_state(&rollout).await?;

        if let Some(current_build_id) = current.as_deref() {
            let _ = self.record_rollback_observation(current_build_id).await;
        }

        Ok(serde_json::json!({
            "rolled_back": true,
            "build_id": target.build_id,
            "build_dir": build_dir,
        }))
    }

    pub async fn rollout_summary(&self) -> Result<AutonomyRolloutSummary, String> {
        self.ensure_dirs().await?;
        let rollout = self.load_rollout_state().await.unwrap_or_default();
        let manifests = self.list_build_manifests().await?;
        let current_build_id = self.current_build_id();
        let last_successful_build_id = manifests
            .iter()
            .find(|manifest| manifest.promoted)
            .map(|manifest| manifest.build_id.clone());
        let rollback_target_build_id = manifests
            .iter()
            .filter(|manifest| manifest.promoted)
            .find(|manifest| current_build_id.as_deref() != Some(manifest.build_id.as_str()))
            .map(|manifest| manifest.build_id.clone());

        Ok(AutonomyRolloutSummary {
            current_build_id,
            last_successful_build_id,
            rollback_target_build_id,
            code_auto_apply_paused: rollout.code_auto_apply_paused,
            pause_reason: rollout.pause_reason,
            consecutive_failed_promotions: rollout.consecutive_failed_promotions,
            failed_canary_count: rollout.failed_canaries.len(),
            recent_builds: manifests
                .into_iter()
                .take(8)
                .map(|manifest| AutonomyRolloutEntry {
                    build_id: manifest.build_id,
                    proposal_id: manifest.proposal_id,
                    title: manifest.title,
                    created_at: manifest.created_at,
                    promoted: manifest.promoted,
                    checks: manifest.checks,
                    metadata: manifest.metadata,
                })
                .collect(),
        })
    }

    pub async fn checks_summary(&self) -> Result<AutonomyChecksSummary, String> {
        let bootstrap_report = self.load_bootstrap_report().await?;
        let latest_canary_report = self.latest_canary_report().await?;
        Ok(AutonomyChecksSummary {
            bootstrap_checks: bootstrap_report
                .as_ref()
                .map(bootstrap_report_checks)
                .unwrap_or_default(),
            latest_canary_checks: latest_canary_report
                .map(|report| report.checks)
                .unwrap_or_default(),
            permission_report: bootstrap_report
                .map(|report| report.permissions)
                .unwrap_or(serde_json::Value::Null),
        })
    }

    pub async fn evidence_summary(&self) -> Result<AutonomyEvidenceSummary, String> {
        let bootstrap_report = self.load_bootstrap_report().await?;
        let latest_canary_report = self.latest_canary_report().await?;
        let mut recent_events = Vec::new();
        let last_bootstrap_at = self.runtime_state.read().await.last_bootstrap_at;

        if let Some(report) = bootstrap_report.as_ref() {
            recent_events.push(AutonomyEventItem {
                kind: if report.passed {
                    "bootstrap_passed".to_string()
                } else {
                    "bootstrap_failed".to_string()
                },
                message: report
                    .blocking_reason
                    .clone()
                    .unwrap_or_else(|| "desktop autonomy bootstrap completed".to_string()),
                timestamp: last_bootstrap_at,
            });
            recent_events.extend(report.notes.iter().map(|note| AutonomyEventItem {
                kind: "bootstrap_note".to_string(),
                message: note.clone(),
                timestamp: last_bootstrap_at,
            }));
        }

        let manifests = self.list_build_manifests().await.unwrap_or_default();
        recent_events.extend(manifests.iter().take(5).map(|manifest| AutonomyEventItem {
            kind: if manifest.promoted {
                "rollout_promoted".to_string()
            } else {
                "rollout_candidate".to_string()
            },
            message: format!("{} ({})", manifest.title, manifest.build_id),
            timestamp: Some(manifest.created_at),
        }));

        recent_events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        recent_events.truncate(12);

        Ok(AutonomyEvidenceSummary {
            seeded_routines: bootstrap_report
                .as_ref()
                .map(|report| report.seeded_routines.clone())
                .unwrap_or_default(),
            seeded_skills: bootstrap_report
                .as_ref()
                .map(|report| report.seeded_skills.clone())
                .unwrap_or_default(),
            latest_bootstrap_report: bootstrap_report,
            latest_canary_report,
            recent_events,
        })
    }

    pub async fn local_autorollout(
        &self,
        user_id: &str,
        proposal_id: Uuid,
        diff: &str,
        title: &str,
    ) -> Result<LocalAutorolloutOutcome, String> {
        self.ensure_dirs().await?;
        let mut rollout_state = self.load_rollout_state().await.unwrap_or_default();
        if rollout_state.code_auto_apply_paused {
            return Err(rollout_state
                .pause_reason
                .clone()
                .unwrap_or_else(|| "code auto-apply is paused".to_string()));
        }

        let managed_source = self.sync_managed_source_clone().await?;
        let build_id = format!(
            "{}-{}",
            Utc::now().format("%Y%m%d%H%M%S"),
            &proposal_id.to_string()[..8]
        );
        let build_dir = self.builds_dir().join(&build_id);
        let patch_path = build_dir.join("proposal.patch");

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("worktree")
                .arg("add")
                .arg("--detach")
                .arg(&build_dir)
                .arg("HEAD"),
        )
        .await?;

        tokio::fs::write(&patch_path, diff)
            .await
            .map_err(|e| format!("failed to write rollout patch: {e}"))?;

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&build_dir)
                .arg("apply")
                .arg("--check")
                .arg(&patch_path),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&build_dir)
                .arg("apply")
                .arg(&patch_path),
        )
        .await?;

        let mut checks = Vec::new();
        checks.push(
            run_command_check(
                "cargo check",
                Command::new("cargo").arg("check").current_dir(&build_dir),
            )
            .await,
        );
        checks.push(
            run_command_check(
                "cargo test desktop_autonomy",
                Command::new("cargo")
                    .arg("test")
                    .arg("desktop_autonomy")
                    .current_dir(&build_dir),
            )
            .await,
        );
        checks.push(
            run_command_check(
                "cargo build",
                Command::new("cargo").arg("build").current_dir(&build_dir),
            )
            .await,
        );
        let canary_manifest = self
            .write_canary_manifest(user_id, proposal_id, &build_id, &build_dir)
            .await?;
        let canary_report = self.run_canaries(&build_dir, &canary_manifest).await;
        let canary_report_path = canary_manifest.report_path.clone();
        checks.extend(canary_report.checks.clone());

        let all_passed = checks.iter().all(|check| check.passed);
        let mut metadata = serde_json::json!({
            "build_id": build_id,
            "user_id": user_id,
            "proposal_id": proposal_id,
            "checks": checks,
            "managed_source": managed_source,
            "canary_report_path": canary_report_path,
            "platform": self.platform_label(),
            "bridge_backend": self.bridge_backend().as_str(),
            "providers": self.provider_matrix(),
            "launcher_kind": self.provider_matrix().get("launcher").cloned().unwrap_or(serde_json::Value::Null),
            "publish_mode": "local_autorollout",
        });

        if all_passed {
            self.promote_build(&build_dir).await?;
            rollout_state.consecutive_failed_promotions = 0;
            rollout_state.last_promoted_build_id = Some(build_id.clone());
        } else {
            rollout_state.consecutive_failed_promotions += 1;
            rollout_state.failed_canaries.push(Utc::now());
        }

        trim_failed_canaries(&mut rollout_state.failed_canaries);
        if rollout_state.consecutive_failed_promotions >= 2
            || rollout_state.failed_canaries.len() >= 3
        {
            rollout_state.code_auto_apply_paused = true;
            rollout_state.pause_reason = Some(
                "code auto-rollout paused after repeated promotion/canary failures".to_string(),
            );
        }
        self.save_rollout_state(&rollout_state).await?;

        let manifest = BuildManifest {
            build_id: build_id.clone(),
            user_id: user_id.to_string(),
            proposal_id: proposal_id.to_string(),
            title: title.to_string(),
            created_at: Utc::now(),
            promoted: all_passed,
            checks: checks.clone(),
            metadata: metadata.clone(),
        };
        self.write_build_manifest(&build_id, &manifest).await?;
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert("promoted".to_string(), serde_json::json!(all_passed));
            obj.insert(
                "code_auto_apply_paused".to_string(),
                serde_json::json!(rollout_state.code_auto_apply_paused),
            );
        }

        if all_passed {
            self.trim_old_builds().await?;
        }

        Ok(LocalAutorolloutOutcome {
            build_id,
            build_dir,
            promoted: all_passed,
            checks,
            publish_metadata: metadata,
        })
    }

    async fn domain_action(
        &self,
        domain: &str,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.bridge_domain_action(domain, action, payload, true)
            .await
    }

    async fn bridge_domain_action(
        &self,
        domain: &str,
        action: &str,
        payload: serde_json::Value,
        enforce_runtime_guard: bool,
    ) -> Result<serde_json::Value, String> {
        if enforce_runtime_guard {
            self.ensure_can_run().await?;
        }
        self.ensure_sidecar_script().await?;

        let session_id = payload
            .get("session_id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| self.default_session_id());
        let _lease = self.acquire_session_lease(Some(&session_id)).await?;

        let mut body = payload;
        if !body.is_object() {
            body = serde_json::json!({});
        }
        if let Some(obj) = body.as_object_mut() {
            obj.insert("action".to_string(), serde_json::json!(action));
            obj.insert("session_id".to_string(), serde_json::json!(session_id));
            obj.insert(
                "capture_evidence".to_string(),
                serde_json::json!(self.config.capture_evidence),
            );
            obj.insert(
                "timeout_ms".to_string(),
                serde_json::json!(self.config.desktop_action_timeout_secs * 1000),
            );
        }

        self.bridge_call(domain, body).await
    }

    async fn ensure_dirs(&self) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.state_root)
            .await
            .map_err(|e| format!("failed to create autonomy state root: {e}"))?;
        tokio::fs::create_dir_all(self.builds_dir())
            .await
            .map_err(|e| format!("failed to create builds dir: {e}"))?;
        tokio::fs::create_dir_all(self.state_root.join("manifests"))
            .await
            .map_err(|e| format!("failed to create manifests dir: {e}"))?;
        tokio::fs::create_dir_all(self.fixtures_dir())
            .await
            .map_err(|e| format!("failed to create fixtures dir: {e}"))?;
        Ok(())
    }

    async fn ensure_sidecar_script(&self) -> Result<(), String> {
        self.ensure_dirs().await?;
        let spec = self.bridge_spec();
        if matches!(spec.backend, DesktopBridgeBackend::Unsupported) {
            return Err("desktop autonomy bridge is not supported on this platform".to_string());
        }
        let should_write = match tokio::fs::read_to_string(&self.sidecar_script_path).await {
            Ok(existing) => existing != spec.source,
            Err(_) => true,
        };
        if should_write {
            tokio::fs::write(&self.sidecar_script_path, spec.source)
                .await
                .map_err(|e| format!("failed to write desktop sidecar script: {e}"))?;
        }
        Ok(())
    }

    async fn bridge_call(
        &self,
        command: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let spec = self.bridge_spec();
        let mut child = match spec.backend {
            DesktopBridgeBackend::MacOsSwift => {
                let mut command_builder = Command::new("swift");
                command_builder.arg(&self.sidecar_script_path).arg(command);
                command_builder
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                let mut command_builder = Command::new("powershell");
                command_builder
                    .arg("-NoLogo")
                    .arg("-NoProfile")
                    .arg("-ExecutionPolicy")
                    .arg("Bypass")
                    .arg("-File")
                    .arg(&self.sidecar_script_path)
                    .arg(command);
                command_builder
            }
            DesktopBridgeBackend::LinuxPython => {
                let mut command_builder = Command::new("python3");
                command_builder.arg(&self.sidecar_script_path).arg(command);
                command_builder
            }
            DesktopBridgeBackend::Unsupported => {
                return Err("desktop autonomy bridge is not supported on this platform".into());
            }
        }
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn desktop sidecar: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            let input = serde_json::to_vec(&payload)
                .map_err(|e| format!("failed to encode bridge request: {e}"))?;
            stdin
                .write_all(&input)
                .await
                .map_err(|e| format!("failed to write bridge request: {e}"))?;
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(self.config.desktop_action_timeout_secs.max(5)),
            child.wait_with_output(),
        )
        .await
        .map_err(|_| "desktop sidecar timed out".to_string())?
        .map_err(|e| format!("failed to read desktop sidecar output: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                format!("desktop sidecar failed with {}", output.status)
            } else {
                format!("desktop sidecar failed: {stderr}")
            });
        }

        let value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("failed to decode desktop sidecar response: {e}"))?;
        if value.get("ok").and_then(|value| value.as_bool()) == Some(false) {
            return Err(value
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("desktop sidecar returned an error")
                .to_string());
        }
        Ok(value
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    async fn seed_default_skills(&self) -> Result<Vec<PathBuf>, String> {
        let skills_dir = crate::platform::state_paths()
            .skills_dir
            .join("desktop_autonomy");
        tokio::fs::create_dir_all(&skills_dir)
            .await
            .map_err(|e| format!("failed to create autonomy skills dir: {e}"))?;

        let templates = [
            (
                "desktop_recover_app.md",
                "# desktop_recover_app\n\nRecover a stuck desktop flow by refocusing the target app, dismissing blocking dialogs, reopening the document if needed, restarting the desktop sidecar as a last resort, and only then surfacing attention.\n",
            ),
            (
                "calendar_reconcile.md",
                "# calendar_reconcile\n\nUse `desktop_calendar_native` first. Prefer idempotent find-or-update behavior, verify the final event state, and capture before/after evidence when anything changed.\n",
            ),
            (
                "numbers_update_sheet.md",
                "# numbers_update_sheet\n\nUse `desktop_numbers_native` before generic UI actions. Verify cell reads after every write and prefer table/range operations over coordinate clicks.\n",
            ),
            (
                "pages_prepare_report.md",
                "# pages_prepare_report\n\nUse `desktop_pages_native` before fallback UI automation. Keep edits deterministic, verify exports exist, and preserve document formatting where possible.\n",
            ),
            (
                "daily_desktop_heartbeat.md",
                "# daily_desktop_heartbeat\n\nInspect the desktop autonomy status, confirm bootstrap health, check the emergency stop state, and queue or resume the next desktop routines only when the autonomy profile is healthy.\n",
            ),
        ];

        let mut written = Vec::new();
        for (name, content) in templates {
            let path = skills_dir.join(name);
            tokio::fs::write(&path, content).await.map_err(|e| {
                format!(
                    "failed to seed desktop skill template {}: {e}",
                    path.display()
                )
            })?;
            written.push(path);
        }
        Ok(written)
    }

    async fn seed_default_routines(&self) -> Result<Vec<String>, String> {
        let Some(store) = self.store.as_ref() else {
            return Ok(Vec::new());
        };

        let user_id = "default";
        let actor_id = "default";
        let mut created = Vec::new();
        let weekday_nine = canonicalize_schedule_expr("0 0 9 * * MON-FRI *")
            .map_err(|e| format!("failed to build default heartbeat schedule: {e}"))?;
        let heartbeat_next_fire =
            next_schedule_fire_for_user(&weekday_nine, user_id, None).unwrap_or(None);

        let routines = vec![
            Routine {
                id: Uuid::new_v4(),
                name: "desktop_recover_app".to_string(),
                description: "Recover a stuck desktop app flow by refocusing the app, dismissing blocking UI, reopening the working document, and only then escalating for attention.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Recover desktop app".to_string(),
                    description: "Use desktop_apps, desktop_ui, and desktop_screen to recover a stuck desktop application flow. Refocus the target app, dismiss obvious modal blockers, reopen the working document if needed, verify the UI is responsive again, and surface attention only if recovery fails.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_apps".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "calendar_reconcile".to_string(),
                description: "Open or inspect Calendar data, reconcile the requested changes, verify the event state, and preserve evidence for any modifications.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Reconcile Calendar".to_string(),
                    description: "Use desktop_calendar_native first, then desktop_ui and desktop_screen only if needed. Find the target events, apply the requested create/update/delete actions, verify the final event state, and record before/after evidence for any modifications.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_calendar_native".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "desktop_apps".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "numbers_update_sheet".to_string(),
                description: "Open a Numbers document, apply deterministic cell or formula changes, and verify the resulting sheet state.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Update Numbers sheet".to_string(),
                    description: "Use desktop_numbers_native before any fallback desktop_ui actions. Open the requested document, read the target cells, apply writes or formulas, verify the resulting values, and export or save only when requested.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_numbers_native".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "desktop_apps".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "pages_prepare_report".to_string(),
                description: "Open a Pages document, apply the requested textual edits, and verify the resulting export or document state.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Manual,
                action: RoutineAction::FullJob {
                    title: "Prepare Pages report".to_string(),
                    description: "Use desktop_pages_native first, then desktop_ui and desktop_screen only if needed. Open the requested document, make deterministic text edits, verify the final content, and export the result when requested.".to_string(),
                    max_iterations: 12,
                    allowed_tools: Some(vec![
                        "desktop_pages_native".to_string(),
                        "desktop_ui".to_string(),
                        "desktop_screen".to_string(),
                        "desktop_apps".to_string(),
                        "autonomy_control".to_string(),
                    ]),
                    allowed_skills: None,
                    tool_profile: Some(ToolProfile::Restricted),
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: None,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
            Routine {
                id: Uuid::new_v4(),
                name: "daily_desktop_heartbeat".to_string(),
                description: "Weekday desktop heartbeat that inspects autonomy health, checks the emergency stop state, and queues the next desktop routines only when the profile is healthy.".to_string(),
                user_id: user_id.to_string(),
                actor_id: actor_id.to_string(),
                enabled: true,
                trigger: Trigger::Cron {
                    schedule: weekday_nine.clone(),
                },
                action: RoutineAction::Heartbeat {
                    light_context: true,
                    prompt: Some("Inspect the reckless desktop autonomy status, confirm the bootstrap and permission state are healthy, check the emergency-stop file, and summarize whether desktop routines should continue running today. Queue or recommend any needed follow-up desktop work only when the autonomy profile is healthy.".to_string()),
                    include_reasoning: false,
                    active_start_hour: Some(8),
                    active_end_hour: Some(20),
                    target: "none".to_string(),
                    max_iterations: 8,
                    interval_secs: None,
                },
                guardrails: RoutineGuardrails::default(),
                notify: NotifyConfig::default(),
                policy: Default::default(),
                last_run_at: None,
                next_fire_at: heartbeat_next_fire,
                run_count: 0,
                consecutive_failures: 0,
                state: serde_json::json!({}),
                config_version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            },
        ];

        for routine in routines {
            let exists = store
                .get_routine_by_name_for_actor(user_id, actor_id, &routine.name)
                .await
                .map_err(|e| format!("failed to check routine {}: {e}", routine.name))?
                .is_some();
            if exists {
                continue;
            }
            store
                .create_routine(&routine)
                .await
                .map_err(|e| format!("failed to seed routine {}: {e}", routine.name))?;
            created.push(routine.name);
        }

        Ok(created)
    }

    fn fixtures_dir(&self) -> PathBuf {
        self.state_root.join("fixtures")
    }

    async fn platform_bootstrap_prerequisites(&self) -> DesktopBootstrapPrerequisites {
        let mut checks = Vec::new();
        let mut notes = Vec::new();
        let mut blocking_reason = None;

        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => {
                let app_checks = [
                    ("calendar_app", "/Applications/Calendar.app"),
                    ("numbers_app", "/Applications/Numbers.app"),
                    ("pages_app", "/Applications/Pages.app"),
                    ("textedit_app", "/Applications/TextEdit.app"),
                ];
                for (name, path) in app_checks {
                    let evidence = self.attach_runtime_evidence(
                        "bootstrap_prerequisite",
                        serde_json::json!({ "path": path }),
                    );
                    if Path::new(path).exists() {
                        checks.push(passed_check(name, None, evidence));
                    } else {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            name,
                            format!("required app missing at {path}"),
                            evidence,
                        ));
                    }
                }
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                let command_checks = [
                    (
                        "outlook_com",
                        "requires_supported_apps",
                        "try { $app = New-Object -ComObject Outlook.Application; if ($null -ne $app) { $app.Quit() }; exit 0 } catch { Write-Error $_; exit 1 }",
                    ),
                    (
                        "excel_com",
                        "requires_supported_apps",
                        "try { $app = New-Object -ComObject Excel.Application; if ($null -ne $app) { $app.Quit() }; exit 0 } catch { Write-Error $_; exit 1 }",
                    ),
                    (
                        "word_com",
                        "requires_supported_apps",
                        "try { $app = New-Object -ComObject Word.Application; if ($null -ne $app) { $app.Quit() }; exit 0 } catch { Write-Error $_; exit 1 }",
                    ),
                    (
                        "notepad_app",
                        "requires_supported_apps",
                        "if (Test-Path \"$env:WINDIR\\System32\\notepad.exe\") { exit 0 } else { exit 1 }",
                    ),
                ];
                let interactive_session = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "user_interactive": std::env::var("SESSIONNAME").ok(),
                        "username": std::env::var("USERNAME").ok(),
                    }),
                );
                if std::env::var("SESSIONNAME")
                    .ok()
                    .is_some_and(|name| !name.trim().is_empty())
                {
                    checks.push(passed_check(
                        "interactive_session",
                        None,
                        interactive_session,
                    ));
                } else {
                    blocking_reason.get_or_insert_with(|| "unsupported_display_stack".to_string());
                    checks.push(failed_check(
                        "interactive_session",
                        "Windows reckless desktop requires an interactive desktop session"
                            .to_string(),
                        interactive_session,
                    ));
                }
                for (name, reason, script) in command_checks {
                    let result = run_cmd(
                        Command::new("powershell")
                            .arg("-NoLogo")
                            .arg("-NoProfile")
                            .arg("-Command")
                            .arg(script),
                    )
                    .await;
                    let evidence = self.attach_runtime_evidence(
                        "bootstrap_prerequisite",
                        serde_json::json!({ "script": script }),
                    );
                    match result {
                        Ok(_) => checks.push(passed_check(name, None, evidence)),
                        Err(err) => {
                            blocking_reason.get_or_insert_with(|| reason.to_string());
                            checks.push(failed_check(name, err, evidence));
                        }
                    }
                }
            }
            DesktopBridgeBackend::LinuxPython => {
                let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
                let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
                let display_ok = std::env::var_os("DISPLAY").is_some()
                    && !session_type.eq_ignore_ascii_case("wayland")
                    && current_desktop
                        .split(':')
                        .any(|value| value.eq_ignore_ascii_case("gnome"));
                let display_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "display": std::env::var("DISPLAY").ok(),
                        "wayland_display": std::env::var("WAYLAND_DISPLAY").ok(),
                        "xdg_session_type": std::env::var("XDG_SESSION_TYPE").ok(),
                        "xdg_current_desktop": std::env::var("XDG_CURRENT_DESKTOP").ok(),
                    }),
                );
                if display_ok {
                    checks.push(passed_check("display_stack", None, display_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "unsupported_display_stack".to_string());
                    checks.push(failed_check(
                        "display_stack",
                        "Linux reckless desktop currently requires a logged-in GNOME on X11 session with DISPLAY set. KDE and Wayland are unsupported for this release; choose 'GNOME on Xorg' at login."
                            .to_string(),
                        display_evidence,
                    ));
                }
                let dbus_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "dbus_session_bus_address": std::env::var("DBUS_SESSION_BUS_ADDRESS").ok(),
                    }),
                );
                if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
                    checks.push(passed_check("dbus_session", None, dbus_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "requires_supported_apps".to_string());
                    checks.push(failed_check(
                        "dbus_session",
                        "Linux reckless desktop requires a live user D-Bus session for Evolution/EDS access"
                            .to_string(),
                        dbus_evidence,
                    ));
                }
                let app_checks = [
                    ("python3", "python3"),
                    ("libreoffice", "libreoffice"),
                    ("evolution", "evolution"),
                    ("gdbus", "gdbus"),
                    ("xdotool", "xdotool"),
                    ("wmctrl", "wmctrl"),
                ];
                for (name, command_name) in app_checks {
                    let evidence = self.attach_runtime_evidence(
                        "bootstrap_prerequisite",
                        serde_json::json!({ "command": command_name }),
                    );
                    match run_cmd(
                        Command::new("sh")
                            .arg("-lc")
                            .arg(format!("command -v {command_name}")),
                    )
                    .await
                    {
                        Ok(_) => checks.push(passed_check(name, None, evidence)),
                        Err(err) => {
                            blocking_reason
                                .get_or_insert_with(|| "requires_supported_apps".to_string());
                            checks.push(failed_check(name, err, evidence));
                        }
                    }
                }
                notes.push(
                    "Ubuntu/Debian desktop prerequisites: sudo apt install python3 python3-gi python3-pyatspi libreoffice libreoffice-script-provider-python evolution evolution-data-server-bin xdotool wmctrl tesseract-ocr gnome-screenshot scrot imagemagick at-spi2-core libglib2.0-bin"
                        .to_string(),
                );
                for (name, module) in [("pyatspi_module", "pyatspi"), ("pygobject_module", "gi")] {
                    match run_cmd(
                        Command::new("python3")
                            .arg("-c")
                            .arg(format!("import {module}")),
                    )
                    .await
                    {
                        Ok(_) => checks.push(passed_check(
                            name,
                            None,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({ "python_module": module }),
                            ),
                        )),
                        Err(err) => {
                            blocking_reason
                                .get_or_insert_with(|| "requires_supported_apps".to_string());
                            checks.push(failed_check(
                                name,
                                err,
                                self.attach_runtime_evidence(
                                    "bootstrap_prerequisite",
                                    serde_json::json!({ "python_module": module }),
                                ),
                            ));
                        }
                    }
                }
                match run_cmd(Command::new("python3").arg("-c").arg("import uno")).await {
                    Ok(_) => checks.push(passed_check(
                        "libreoffice_uno",
                        None,
                        self.attach_runtime_evidence(
                            "bootstrap_prerequisite",
                            serde_json::json!({ "python_module": "uno" }),
                        ),
                    )),
                    Err(err) => {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            "libreoffice_uno",
                            err,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({ "python_module": "uno" }),
                            ),
                        ));
                    }
                }
                match run_cmd(Command::new("sh").arg("-lc").arg("command -v tesseract")).await {
                    Ok(_) => checks.push(passed_check(
                        "ocr_tooling",
                        None,
                        self.attach_runtime_evidence(
                            "bootstrap_prerequisite",
                            serde_json::json!({ "command": "tesseract" }),
                        ),
                    )),
                    Err(err) => {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            "ocr_tooling",
                            err,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({ "command": "tesseract" }),
                            ),
                        ));
                    }
                }
                match run_cmd(
                    Command::new("sh")
                        .arg("-lc")
                        .arg("command -v gedit || command -v xdg-text-editor"),
                )
                .await
                {
                    Ok(_) => checks.push(passed_check(
                        "generic_editor",
                        None,
                        self.attach_runtime_evidence(
                            "bootstrap_prerequisite",
                            serde_json::json!({
                                "commands": ["gedit", "xdg-text-editor"],
                                "provider": self.generic_ui_provider(),
                            }),
                        ),
                    )),
                    Err(err) => {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            "generic_editor",
                            err,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({
                                    "commands": ["gedit", "xdg-text-editor"],
                                    "provider": self.generic_ui_provider(),
                                }),
                            ),
                        ));
                    }
                }
                let at_spi_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "at_spi_bus_address": std::env::var("AT_SPI_BUS_ADDRESS").ok(),
                        "gtk_modules": std::env::var("GTK_MODULES").ok(),
                    }),
                );
                if std::env::var_os("AT_SPI_BUS_ADDRESS").is_some()
                    || std::env::var_os("GTK_MODULES")
                        .is_some_and(|value| value.to_string_lossy().contains("gail"))
                {
                    checks.push(passed_check("accessibility_bus", None, at_spi_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "requires_supported_apps".to_string());
                    checks.push(failed_check(
                        "accessibility_bus",
                        "Linux reckless desktop requires an active AT-SPI accessibility session"
                            .to_string(),
                        at_spi_evidence,
                    ));
                }
            }
            DesktopBridgeBackend::Unsupported => {
                blocking_reason = Some("unsupported_display_stack".to_string());
                checks.push(failed_check(
                    "bridge_backend",
                    "desktop autonomy bridge is unsupported on this platform".to_string(),
                    self.attach_runtime_evidence("bootstrap_prerequisite", serde_json::json!({})),
                ));
            }
        }

        DesktopBootstrapPrerequisites {
            passed: checks.iter().all(|check| check.passed),
            blocking_reason,
            evidence: self.attach_runtime_evidence(
                "bootstrap_prerequisites",
                serde_json::json!({ "check_count": checks.len() }),
            ),
            checks,
            notes,
        }
    }

    async fn prepare_dedicated_user_bootstrap(&self) -> Result<DedicatedUserBootstrap, String> {
        if self.config.deployment_mode != crate::settings::DesktopDeploymentMode::DedicatedUser {
            return Ok(DedicatedUserBootstrap {
                session_ready: true,
                ..Default::default()
            });
        }

        let username = self
            .config
            .target_username
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                "dedicated_user deployment requires desktop_autonomy.target_username".to_string()
            })?;
        let keychain_label = format!("ThinClaw Desktop Autonomy/{username}");

        let mut bootstrap = DedicatedUserBootstrap {
            keychain_label: Some(keychain_label.clone()),
            ..Default::default()
        };

        if matches!(self.bridge_backend(), DesktopBridgeBackend::LinuxPython) {
            bootstrap.blocking_reason = Some("unsupported_deployment_mode".to_string());
            return Ok(bootstrap);
        }

        let exists = self.user_exists(&username).await?;
        if !exists {
            if !self.has_privileged_bootstrap().await {
                bootstrap.blocking_reason =
                    Some(dedicated_bootstrap_blocking_reason(false, false, false).to_string());
                return Ok(bootstrap);
            }

            if matches!(self.bridge_backend(), DesktopBridgeBackend::LinuxPython) {
                bootstrap.blocking_reason = Some("unsupported_deployment_mode".to_string());
                return Ok(bootstrap);
            }

            let password = generate_dedicated_user_secret();
            self.create_dedicated_user(&username, &password).await?;
            crate::platform::secure_store::store_api_key(&keychain_label, &password)
                .await
                .map_err(|e| format!("failed to store dedicated-user password in keychain: {e}"))?;
            bootstrap.created_user = true;
            bootstrap.one_time_login_secret = Some(password);
        }

        let session_subject = self.target_session_subject().await?;
        bootstrap.session_ready = self
            .gui_session_ready(&session_subject, Some(&username))
            .await;
        if !bootstrap.session_ready {
            bootstrap.blocking_reason =
                Some(dedicated_bootstrap_blocking_reason(true, true, false).to_string());
        }

        Ok(bootstrap)
    }

    async fn ensure_canary_fixtures(&self) -> Result<DesktopFixturePaths, String> {
        self.ensure_dirs().await?;
        let fixtures_dir = self.fixtures_dir();
        tokio::fs::create_dir_all(&fixtures_dir)
            .await
            .map_err(|e| format!("failed to create canary fixtures dir: {e}"))?;

        let (numbers_ext, pages_ext) = self.fixture_extensions();
        let numbers_doc = fixtures_dir.join(format!("canary.{numbers_ext}"));
        let pages_doc = fixtures_dir.join(format!("canary.{pages_ext}"));
        let textedit_doc = fixtures_dir.join("canary.txt");
        let export_dir = fixtures_dir.join("exports");
        tokio::fs::create_dir_all(&export_dir)
            .await
            .map_err(|e| format!("failed to create canary export dir: {e}"))?;
        if tokio::fs::metadata(&textedit_doc).await.is_err() {
            tokio::fs::write(&textedit_doc, "")
                .await
                .map_err(|e| format!("failed to create TextEdit canary fixture: {e}"))?;
        }

        let calendar_title = "ThinClaw Canary".to_string();
        self.bridge_domain_action(
            "calendar",
            "ensure_calendar",
            serde_json::json!({ "title": calendar_title }),
            false,
        )
        .await?;

        if tokio::fs::metadata(&numbers_doc).await.is_err() {
            self.bridge_domain_action(
                "numbers",
                "create_doc",
                serde_json::json!({ "path": numbers_doc }),
                false,
            )
            .await?;
        }

        if tokio::fs::metadata(&pages_doc).await.is_err() {
            self.bridge_domain_action(
                "pages",
                "create_doc",
                serde_json::json!({ "path": pages_doc }),
                false,
            )
            .await?;
        }

        Ok(DesktopFixturePaths {
            calendar_title: "ThinClaw Canary".to_string(),
            numbers_doc: Some(numbers_doc),
            pages_doc: Some(pages_doc),
            textedit_doc: Some(textedit_doc),
            export_dir: Some(export_dir),
        })
    }

    async fn write_canary_manifest(
        &self,
        _user_id: &str,
        proposal_id: Uuid,
        build_id: &str,
        build_dir: &Path,
    ) -> Result<DesktopCanaryManifest, String> {
        let live_fixtures = self.ensure_canary_fixtures().await?;
        let shadow_home = build_dir.join("shadow-home");
        let shadow_fixtures_dir = build_dir.join("canary-fixtures");
        let shadow_export_dir = shadow_fixtures_dir.join("exports");
        tokio::fs::create_dir_all(&shadow_home)
            .await
            .map_err(|e| format!("failed to create shadow home: {e}"))?;
        tokio::fs::create_dir_all(&shadow_export_dir)
            .await
            .map_err(|e| format!("failed to create canary export dir: {e}"))?;

        let (numbers_ext, pages_ext) = self.fixture_extensions();
        let numbers_doc = shadow_fixtures_dir.join(format!("canary.{numbers_ext}"));
        let pages_doc = shadow_fixtures_dir.join(format!("canary.{pages_ext}"));
        let textedit_doc = shadow_fixtures_dir.join("canary.txt");

        tokio::fs::create_dir_all(&shadow_fixtures_dir)
            .await
            .map_err(|e| format!("failed to create build fixture dir: {e}"))?;

        if let Some(source) = live_fixtures.numbers_doc.as_ref() {
            copy_fixture_path(source, &numbers_doc)
                .map_err(|e| format!("failed to copy Numbers canary fixture: {e}"))?;
        }
        if let Some(source) = live_fixtures.pages_doc.as_ref() {
            copy_fixture_path(source, &pages_doc)
                .map_err(|e| format!("failed to copy Pages canary fixture: {e}"))?;
        }
        if let Some(source) = live_fixtures.textedit_doc.as_ref() {
            copy_fixture_path(source, &textedit_doc)
                .map_err(|e| format!("failed to copy TextEdit canary fixture: {e}"))?;
        }

        let manifest = DesktopCanaryManifest {
            build_id: build_id.to_string(),
            proposal_id: proposal_id.to_string(),
            report_path: build_dir.join("canary-report.json"),
            shadow_home,
            session_id: self.default_session_id(),
            fixture_paths: DesktopFixturePaths {
                calendar_title: live_fixtures.calendar_title,
                numbers_doc: Some(numbers_doc),
                pages_doc: Some(pages_doc),
                textedit_doc: Some(textedit_doc),
                export_dir: Some(shadow_export_dir),
            },
        };
        let manifest_path = build_dir.join("canary-manifest.json");
        let raw = serde_json::to_string_pretty(&manifest)
            .map_err(|e| format!("failed to serialize canary manifest: {e}"))?;
        tokio::fs::write(&manifest_path, raw)
            .await
            .map_err(|e| format!("failed to write canary manifest: {e}"))?;
        Ok(manifest)
    }

    async fn run_canaries(
        &self,
        build_dir: &Path,
        manifest: &DesktopCanaryManifest,
    ) -> DesktopCanaryReport {
        match self
            .run_shadow_canary_process(&self.shadow_binary_path(build_dir), manifest)
            .await
        {
            Ok(report) => report,
            Err(err) => DesktopCanaryReport {
                build_id: manifest.build_id.clone(),
                generated_at: Utc::now(),
                passed: false,
                fixture_paths: manifest.fixture_paths.clone(),
                checks: vec![self.runtime_failed_check(
                    "shadow_canary_runner",
                    err,
                    serde_json::json!({
                        "binary": self.shadow_binary_path(build_dir),
                        "manifest": build_dir.join("canary-manifest.json"),
                    }),
                )],
            },
        }
    }

    async fn run_shadow_canary_process(
        &self,
        binary_path: &Path,
        manifest: &DesktopCanaryManifest,
    ) -> Result<DesktopCanaryReport, String> {
        let mut command = Command::new(binary_path);
        command.arg("autonomy-shadow-canary");
        command.arg("--manifest");
        command.arg(manifest.report_path.with_file_name("canary-manifest.json"));
        command.env("THINCLAW_HOME", &manifest.shadow_home);
        command.env("HOME", &manifest.shadow_home);
        command.env("USERPROFILE", &manifest.shadow_home);
        command.env("DESKTOP_AUTONOMY_ENABLED", "true");
        command.env(
            "DESKTOP_AUTONOMY_PROFILE",
            self.config.profile.as_str().to_string(),
        );
        command.env(
            "DESKTOP_AUTONOMY_DEPLOYMENT_MODE",
            self.config.deployment_mode.as_str().to_string(),
        );
        if let Some(username) = self.config.target_username.as_deref() {
            command.env("DESKTOP_AUTONOMY_TARGET_USERNAME", username);
        }
        command.env(
            "DESKTOP_AUTONOMY_MAX_CONCURRENT_JOBS",
            self.config.desktop_max_concurrent_jobs.to_string(),
        );
        command.env(
            "DESKTOP_AUTONOMY_ACTION_TIMEOUT_SECS",
            self.config.desktop_action_timeout_secs.to_string(),
        );
        command.env(
            "DESKTOP_AUTONOMY_CAPTURE_EVIDENCE",
            self.config.capture_evidence.to_string(),
        );
        command.env(
            "DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH",
            self.config.emergency_stop_path.as_os_str(),
        );
        if let Some(db) = self.database_config.as_ref() {
            self.apply_shadow_database_env(&mut command, db);
            if matches!(db.backend, crate::config::DatabaseBackend::LibSql) {
                command.env("LIBSQL_PATH", manifest.shadow_home.join("thinclaw.db"));
            }
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());

        let output = command
            .output()
            .await
            .map_err(|e| format!("failed to spawn shadow canary runner: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                format!("shadow canary runner exited with {}", output.status)
            } else {
                stderr
            });
        }

        serde_json::from_slice::<DesktopCanaryReport>(&output.stdout)
            .map_err(|e| format!("failed to decode shadow canary report: {e}"))
    }

    fn apply_shadow_database_env(&self, command: &mut Command, database: &DatabaseConfig) {
        command.env("DATABASE_BACKEND", database.backend.to_string());
        match database.backend {
            crate::config::DatabaseBackend::Postgres => {
                command.env("DATABASE_URL", database.url());
                command.env("DATABASE_POOL_SIZE", database.pool_size.to_string());
            }
            crate::config::DatabaseBackend::LibSql => {
                if let Some(path) = database.libsql_path.as_ref() {
                    command.env("LIBSQL_PATH", path);
                }
                if let Some(url) = database.libsql_url.as_ref() {
                    command.env("LIBSQL_URL", url);
                }
                if let Some(token) = database.libsql_auth_token.as_ref() {
                    command.env("LIBSQL_AUTH_TOKEN", token.expose_secret());
                }
            }
        }
    }

    fn shadow_binary_path(&self, build_dir: &Path) -> PathBuf {
        let exe = if cfg!(windows) {
            "thinclaw.exe"
        } else {
            "thinclaw"
        };
        build_dir.join("target").join("debug").join(exe)
    }

    async fn user_exists(&self, username: &str) -> Result<bool, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => match run_cmd(
                Command::new("dscl")
                    .arg(".")
                    .arg("-read")
                    .arg(format!("/Users/{username}")),
            )
            .await
            {
                Ok(_) => Ok(true),
                Err(err) if err.contains("eDSUnknownNodeName") => Ok(false),
                Err(err) => Err(err),
            },
            DesktopBridgeBackend::WindowsPowerShell => run_cmd(
                Command::new("cmd")
                    .arg("/C")
                    .arg(format!("net user {username}")),
            )
            .await
            .map(|_| true)
            .or_else(|err| {
                if err.contains("The user name could not be found") {
                    Ok(false)
                } else {
                    Err(err)
                }
            }),
            DesktopBridgeBackend::LinuxPython => {
                run_cmd(Command::new("id").arg("-u").arg(username))
                    .await
                    .map(|_| true)
                    .or_else(|err| {
                        if err.contains("no such user") {
                            Ok(false)
                        } else {
                            Err(err)
                        }
                    })
            }
            DesktopBridgeBackend::Unsupported => Ok(false),
        }
    }

    async fn has_privileged_bootstrap(&self) -> bool {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift | DesktopBridgeBackend::LinuxPython => run_cmd(
                Command::new("id").arg("-u"),
            )
            .await
            .map(|uid| uid.trim() == "0")
            .unwrap_or(false),
            DesktopBridgeBackend::WindowsPowerShell => run_cmd(
                Command::new("powershell")
                    .arg("-NoLogo")
                    .arg("-NoProfile")
                    .arg("-Command")
                    .arg(
                        "[bool](([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator))",
                    ),
            )
            .await
            .map(|value| value.trim().eq_ignore_ascii_case("true"))
            .unwrap_or(false),
            DesktopBridgeBackend::Unsupported => false,
        }
    }

    async fn create_dedicated_user(&self, username: &str, password: &str) -> Result<(), String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => {
                run_cmd(
                    Command::new("sysadminctl")
                        .arg("-addUser")
                        .arg(username)
                        .arg("-password")
                        .arg(password)
                        .arg("-home")
                        .arg(PathBuf::from("/Users").join(username)),
                )
                .await?;
                Ok(())
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                let escaped_password = password.replace('\'', "''");
                run_cmd(
                    Command::new("powershell")
                        .arg("-NoLogo")
                        .arg("-NoProfile")
                        .arg("-Command")
                        .arg(format!(
                            "$pw = ConvertTo-SecureString '{escaped_password}' -AsPlainText -Force; \
                             New-LocalUser -Name '{username}' -Password $pw -AccountNeverExpires -PasswordNeverExpires; \
                             Add-LocalGroupMember -Group 'Users' -Member '{username}'"
                        )),
                )
                .await?;
                Ok(())
            }
            DesktopBridgeBackend::LinuxPython => Err(
                "linux dedicated_user bootstrap is best-effort only and will not create users"
                    .to_string(),
            ),
            DesktopBridgeBackend::Unsupported => {
                Err("dedicated-user creation is unsupported on this platform".to_string())
            }
        }
    }

    async fn gui_session_ready(&self, session_subject: &str, username: Option<&str>) -> bool {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => {
                if run_cmd(
                    Command::new("launchctl")
                        .arg("print")
                        .arg(format!("gui/{session_subject}")),
                )
                .await
                .is_ok()
                {
                    return true;
                }

                let Some(username) = username else {
                    return false;
                };
                run_cmd(
                    Command::new("stat")
                        .arg("-f")
                        .arg("%Su")
                        .arg("/dev/console"),
                )
                .await
                .map(|owner| owner.trim() == username)
                .unwrap_or(false)
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                let user = username.unwrap_or(session_subject);
                run_cmd(Command::new("query").arg("user").arg(user))
                    .await
                    .is_ok()
            }
            DesktopBridgeBackend::LinuxPython => {
                let expected_user = username.unwrap_or(session_subject);
                let display_ready = std::env::var_os("DISPLAY").is_some();
                let current_user = std::env::var("USER")
                    .ok()
                    .or_else(|| std::env::var("LOGNAME").ok())
                    .unwrap_or_default();
                display_ready && current_user == expected_user
            }
            DesktopBridgeBackend::Unsupported => false,
        }
    }

    async fn write_session_launcher(&self) -> Result<PathBuf, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => self.write_launch_agent_plist().await,
            DesktopBridgeBackend::WindowsPowerShell => self.write_windows_session_launcher().await,
            DesktopBridgeBackend::LinuxPython => self.write_linux_session_launcher().await,
            DesktopBridgeBackend::Unsupported => {
                Err("session launcher installation is unsupported on this platform".to_string())
            }
        }
    }

    async fn activate_session_launcher(&self, launcher_path: &Path) -> Result<(), String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => self.load_launch_agent(launcher_path).await,
            DesktopBridgeBackend::WindowsPowerShell => {
                self.activate_windows_session_launcher(launcher_path).await
            }
            DesktopBridgeBackend::LinuxPython => {
                self.activate_linux_session_launcher(launcher_path).await
            }
            DesktopBridgeBackend::Unsupported => {
                Err("session launcher activation is unsupported on this platform".to_string())
            }
        }
    }

    async fn write_launch_agent_plist(&self) -> Result<PathBuf, String> {
        #[cfg(not(target_os = "macos"))]
        {
            Err("launch agent installation is only supported on macOS".to_string())
        }

        #[cfg(target_os = "macos")]
        {
            let plist_path = self.session_launcher_path()?;
            if let Some(parent) = plist_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("failed to create launch agent dir: {e}"))?;
            }
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let home = self.session_launcher_home()?;
            let logs_dir = home.join(".thinclaw").join("logs");
            tokio::fs::create_dir_all(&logs_dir)
                .await
                .map_err(|e| format!("failed to create autonomy logs dir: {e}"))?;
            let stdout = logs_dir.join("desktop-autonomy.stdout.log");
            let stderr = logs_dir.join("desktop-autonomy.stderr.log");
            let mut env_entries = vec![
                (
                    "HOME".to_string(),
                    xml_escape(home.to_string_lossy().as_ref()),
                ),
                (
                    "PATH".to_string(),
                    "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
                ),
                ("DESKTOP_AUTONOMY_ENABLED".to_string(), "true".to_string()),
                (
                    "DESKTOP_AUTONOMY_PROFILE".to_string(),
                    self.config.profile.as_str().to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_DEPLOYMENT_MODE".to_string(),
                    self.config.deployment_mode.as_str().to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_MAX_CONCURRENT_JOBS".to_string(),
                    self.config.desktop_max_concurrent_jobs.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_ACTION_TIMEOUT_SECS".to_string(),
                    self.config.desktop_action_timeout_secs.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_CAPTURE_EVIDENCE".to_string(),
                    self.config.capture_evidence.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH".to_string(),
                    xml_escape(self.config.emergency_stop_path.to_string_lossy().as_ref()),
                ),
                (
                    "DESKTOP_AUTONOMY_PAUSE_ON_BOOTSTRAP_FAILURE".to_string(),
                    self.config.pause_on_bootstrap_failure.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_KILL_SWITCH_HOTKEY".to_string(),
                    xml_escape(&self.config.kill_switch_hotkey),
                ),
            ];
            if let Some(username) = self.config.target_username.as_deref() {
                env_entries.push((
                    "DESKTOP_AUTONOMY_TARGET_USERNAME".to_string(),
                    xml_escape(username),
                ));
            }
            let environment_variables = env_entries
                .into_iter()
                .map(|(key, value)| format!("    <key>{key}</key>\n    <string>{value}</string>\n"))
                .collect::<String>();

            let plist = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
  <key>Label</key>\n\
  <string>{label}</string>\n\
  <key>ProgramArguments</key>\n\
  <array>\n\
    <string>{exe}</string>\n\
    <string>run</string>\n\
    <string>--no-onboard</string>\n\
  </array>\n\
  <key>RunAtLoad</key>\n\
  <true/>\n\
  <key>KeepAlive</key>\n\
  <true/>\n\
  <key>EnvironmentVariables</key>\n\
  <dict>\n\
{environment_variables}\
  </dict>\n\
  <key>StandardOutPath</key>\n\
  <string>{stdout}</string>\n\
  <key>StandardErrorPath</key>\n\
  <string>{stderr}</string>\n\
</dict>\n\
</plist>\n",
                label = self.launch_agent_label(),
                exe = xml_escape(exe.to_string_lossy().as_ref()),
                environment_variables = environment_variables,
                stdout = xml_escape(stdout.to_string_lossy().as_ref()),
                stderr = xml_escape(stderr.to_string_lossy().as_ref()),
            );
            tokio::fs::write(&plist_path, plist)
                .await
                .map_err(|e| format!("failed to write launch agent plist: {e}"))?;
            Ok(plist_path)
        }
    }

    async fn load_launch_agent(&self, plist_path: &Path) -> Result<(), String> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = plist_path;
            Err("launch agent bootstrap is only supported on macOS".to_string())
        }

        #[cfg(target_os = "macos")]
        {
            let uid = self.target_session_subject().await?;
            let _ = run_cmd(
                Command::new("launchctl")
                    .arg("bootout")
                    .arg(format!("gui/{uid}"))
                    .arg(plist_path),
            )
            .await;
            run_cmd(
                Command::new("launchctl")
                    .arg("bootstrap")
                    .arg(format!("gui/{uid}"))
                    .arg(plist_path),
            )
            .await?;
            run_cmd(
                Command::new("launchctl")
                    .arg("kickstart")
                    .arg("-k")
                    .arg(format!("gui/{uid}/{}", self.launch_agent_label())),
            )
            .await?;
            Ok(())
        }
    }

    async fn write_windows_session_launcher(&self) -> Result<PathBuf, String> {
        #[cfg(not(target_os = "windows"))]
        {
            Err("windows session launcher is only supported on Windows".to_string())
        }

        #[cfg(target_os = "windows")]
        {
            let launcher_path = self.session_launcher_path()?;
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let mut lines = vec![
                "@echo off".to_string(),
                "set DESKTOP_AUTONOMY_ENABLED=true".to_string(),
                format!(
                    "set DESKTOP_AUTONOMY_PROFILE={}",
                    self.config.profile.as_str()
                ),
                format!(
                    "set DESKTOP_AUTONOMY_DEPLOYMENT_MODE={}",
                    self.config.deployment_mode.as_str()
                ),
                format!(
                    "set DESKTOP_AUTONOMY_CAPTURE_EVIDENCE={}",
                    self.config.capture_evidence
                ),
                format!(
                    "set DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH={}",
                    self.config.emergency_stop_path.display()
                ),
            ];
            if let Some(username) = self.config.target_username.as_deref() {
                lines.push(format!("set DESKTOP_AUTONOMY_TARGET_USERNAME={username}"));
            }
            lines.push(format!("\"{}\" run --no-onboard", exe.display()));
            let script = format!("{}\r\n", lines.join("\r\n"));
            tokio::fs::write(&launcher_path, script)
                .await
                .map_err(|e| format!("failed to write windows session launcher: {e}"))?;
            Ok(launcher_path)
        }
    }

    async fn activate_windows_session_launcher(&self, launcher_path: &Path) -> Result<(), String> {
        #[cfg(not(target_os = "windows"))]
        {
            let _ = launcher_path;
            Err("windows session launcher activation is only supported on Windows".to_string())
        }

        #[cfg(target_os = "windows")]
        {
            let task_name = self.launch_agent_label();
            let launcher_command = format!("\"{}\"", launcher_path.display());
            let mut command = Command::new("schtasks");
            command
                .arg("/Create")
                .arg("/F")
                .arg("/TN")
                .arg(&task_name)
                .arg("/SC")
                .arg("ONLOGON")
                .arg("/TR")
                .arg(&launcher_command);

            if self.config.deployment_mode == crate::settings::DesktopDeploymentMode::DedicatedUser
            {
                let username = self
                    .config
                    .target_username
                    .as_deref()
                    .ok_or_else(|| "missing target username".to_string())?;
                command.arg("/RU").arg(username);
                if let Some(secret) = crate::platform::secure_store::get_api_key(&format!(
                    "ThinClaw Desktop Autonomy/{username}"
                ))
                .await
                {
                    command.arg("/RP").arg(secret);
                }
            }

            run_cmd(&mut command).await?;
            let _ = run_cmd(
                Command::new("schtasks")
                    .arg("/Run")
                    .arg("/TN")
                    .arg(&task_name),
            )
            .await;
            Ok(())
        }
    }

    async fn write_linux_session_launcher(&self) -> Result<PathBuf, String> {
        #[cfg(not(target_os = "linux"))]
        {
            Err("linux session launcher is only supported on Linux".to_string())
        }

        #[cfg(target_os = "linux")]
        {
            let launcher_path = self.session_launcher_path()?;
            if let Some(parent) = launcher_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("failed to create linux autostart dir: {e}"))?;
            }
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let home = self.session_launcher_home()?;
            let desktop_entry = format!(
                "[Desktop Entry]\nType=Application\nName=ThinClaw Desktop Autonomy\nComment=ThinClaw reckless desktop session launcher\nExec=\"{}\" run --no-onboard\nPath={}\nOnlyShowIn=GNOME;\nX-GNOME-Autostart-enabled=true\nTerminal=false\n",
                exe.display(),
                home.display(),
            );
            tokio::fs::write(&launcher_path, desktop_entry)
                .await
                .map_err(|e| format!("failed to write linux session launcher: {e}"))?;
            Ok(launcher_path)
        }
    }

    async fn activate_linux_session_launcher(&self, launcher_path: &Path) -> Result<(), String> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = launcher_path;
            Err("linux session launcher activation is only supported on Linux".to_string())
        }

        #[cfg(target_os = "linux")]
        {
            let raw = tokio::fs::read_to_string(launcher_path)
                .await
                .map_err(|e| format!("failed to read linux session launcher: {e}"))?;
            if !raw.contains("[Desktop Entry]")
                || !raw.contains("OnlyShowIn=GNOME;")
                || !raw.contains("run --no-onboard")
            {
                return Err(
                    "linux session launcher does not contain the required GNOME autostart entry"
                        .to_string(),
                );
            }
            Ok(())
        }
    }

    fn launch_agent_label(&self) -> String {
        format!(
            "com.thinclaw.desktop-autonomy.{}",
            self.config.deployment_mode.as_str()
        )
    }

    fn session_launcher_home(&self) -> Result<PathBuf, String> {
        match self.config.deployment_mode {
            crate::settings::DesktopDeploymentMode::WholeMachineAdmin => dirs::home_dir()
                .ok_or_else(|| "failed to resolve current user home directory".to_string()),
            crate::settings::DesktopDeploymentMode::DedicatedUser => {
                let username = self
                    .config
                    .target_username
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        "dedicated_user deployment requires desktop_autonomy.target_username"
                            .to_string()
                    })?;
                match self.bridge_backend() {
                    DesktopBridgeBackend::MacOsSwift => Ok(PathBuf::from("/Users").join(username)),
                    DesktopBridgeBackend::WindowsPowerShell => {
                        Ok(PathBuf::from(r"C:\Users").join(username))
                    }
                    DesktopBridgeBackend::LinuxPython => Ok(PathBuf::from("/home").join(username)),
                    DesktopBridgeBackend::Unsupported => {
                        Err("failed to resolve target home for unsupported platform".to_string())
                    }
                }
            }
        }
    }

    fn session_launcher_path(&self) -> Result<PathBuf, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => Ok(self
                .session_launcher_home()?
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{}.plist", self.launch_agent_label()))),
            DesktopBridgeBackend::WindowsPowerShell => Ok(self
                .state_root
                .join(format!("{}.cmd", self.launch_agent_label()))),
            DesktopBridgeBackend::LinuxPython => Ok(self
                .session_launcher_home()?
                .join(".config")
                .join("autostart")
                .join(format!("{}.desktop", self.launch_agent_label()))),
            DesktopBridgeBackend::Unsupported => {
                Err("session launcher path is unsupported on this platform".to_string())
            }
        }
    }

    async fn target_session_subject(&self) -> Result<String, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => match self.config.deployment_mode {
                crate::settings::DesktopDeploymentMode::WholeMachineAdmin => {
                    run_cmd(Command::new("id").arg("-u"))
                        .await
                        .map(|value| value.trim().to_string())
                }
                crate::settings::DesktopDeploymentMode::DedicatedUser => {
                    let username = self
                        .config
                        .target_username
                        .as_deref()
                        .ok_or_else(|| "missing target username".to_string())?;
                    let output = run_cmd(
                        Command::new("dscl")
                            .arg(".")
                            .arg("-read")
                            .arg(format!("/Users/{username}"))
                            .arg("UniqueID"),
                    )
                    .await?;
                    output
                        .split_whitespace()
                        .last()
                        .map(str::to_string)
                        .ok_or_else(|| "failed to parse dedicated user uid".to_string())
                }
            },
            DesktopBridgeBackend::WindowsPowerShell | DesktopBridgeBackend::LinuxPython => {
                match self.config.deployment_mode {
                    crate::settings::DesktopDeploymentMode::WholeMachineAdmin => {
                        std::env::var("USER")
                            .or_else(|_| std::env::var("USERNAME"))
                            .or_else(|_| std::env::var("LOGNAME"))
                            .map_err(|e| format!("failed to resolve interactive username: {e}"))
                    }
                    crate::settings::DesktopDeploymentMode::DedicatedUser => self
                        .config
                        .target_username
                        .clone()
                        .ok_or_else(|| "missing target username".to_string()),
                }
            }
            DesktopBridgeBackend::Unsupported => {
                Err("target session lookup is unsupported on this platform".to_string())
            }
        }
    }

    async fn sync_managed_source_clone(&self) -> Result<PathBuf, String> {
        let repo_root = std::env::current_dir().map_err(|e| format!("current_dir: {e}"))?;
        let managed_source = self.state_root.join("agent-src");
        if !managed_source.exists() {
            run_cmd(
                Command::new("git")
                    .arg("clone")
                    .arg("--no-hardlinks")
                    .arg(repo_root.as_os_str())
                    .arg(managed_source.as_os_str()),
            )
            .await?;
            return Ok(managed_source);
        }

        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("fetch")
                .arg("--all")
                .arg("--prune"),
        )
        .await?;
        let head_ref = run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&repo_root)
                .arg("rev-parse")
                .arg("HEAD"),
        )
        .await?;
        run_cmd(
            Command::new("git")
                .arg("-C")
                .arg(&managed_source)
                .arg("reset")
                .arg("--hard")
                .arg(head_ref.trim()),
        )
        .await?;
        Ok(managed_source)
    }

    pub async fn execute_canary_manifest(
        &self,
        manifest: &DesktopCanaryManifest,
    ) -> Result<DesktopCanaryReport, String> {
        let mut checks = Vec::new();
        checks.push(
            match self.bridge_call("health", serde_json::json!({})).await {
                Ok(result) => {
                    self.runtime_passed_check("bridge_health", Some(result.clone()), result)
                }
                Err(err) => {
                    self.runtime_failed_check("bridge_health", err, serde_json::Value::Null)
                }
            },
        );
        checks.push(match self.desktop_permission_status().await {
            Ok(result) => self.runtime_passed_check("permissions", Some(result.clone()), result),
            Err(err) => self.runtime_failed_check("permissions", err, serde_json::Value::Null),
        });
        checks.push(
            match self.apps_action("list", serde_json::json!({})).await {
                Ok(result) => self.runtime_passed_check("apps_list", None, result),
                Err(err) => self.runtime_failed_check("apps_list", err, serde_json::Value::Null),
            },
        );
        checks.push(self.run_calendar_crud_canary(manifest).await);
        checks.push(self.run_numbers_canary(manifest).await);
        checks.push(self.run_pages_canary(manifest).await);
        checks.push(self.run_textedit_canary(manifest).await);

        let report = DesktopCanaryReport {
            build_id: manifest.build_id.clone(),
            generated_at: Utc::now(),
            passed: checks.iter().all(|check| check.passed),
            fixture_paths: manifest.fixture_paths.clone(),
            checks,
        };
        let raw = serde_json::to_string_pretty(&report)
            .map_err(|e| format!("failed to serialize canary report: {e}"))?;
        tokio::fs::write(&manifest.report_path, raw)
            .await
            .map_err(|e| format!("failed to write canary report: {e}"))?;
        Ok(report)
    }

    async fn run_calendar_crud_canary(
        &self,
        manifest: &DesktopCanaryManifest,
    ) -> AutonomyCheckResult {
        let title = format!("ThinClaw Canary {}", Uuid::new_v4().simple());
        let updated_title = format!("{title} Updated");
        let start = (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        let end = (Utc::now() + chrono::Duration::minutes(65)).to_rfc3339();

        let result = async {
            let ensured = self
                .calendar_action(
                    "ensure_calendar",
                    serde_json::json!({ "title": manifest.fixture_paths.calendar_title }),
                )
                .await?;
            let created = self
                .calendar_action(
                    "create",
                    serde_json::json!({
                        "title": title,
                        "calendar": manifest.fixture_paths.calendar_title,
                        "start": start,
                        "end": end,
                        "notes": "ThinClaw desktop canary event",
                    }),
                )
                .await?;
            let event_id = created
                .get("id")
                .and_then(|value| value.as_str())
                .ok_or_else(|| "calendar create did not return an id".to_string())?;
            let found = self
                .calendar_action(
                    "find",
                    serde_json::json!({
                        "query": title,
                        "calendar": manifest.fixture_paths.calendar_title,
                    }),
                )
                .await?;
            self.calendar_action(
                "update",
                serde_json::json!({
                    "event_id": event_id,
                    "title": updated_title,
                }),
            )
            .await?;
            self.calendar_action("delete", serde_json::json!({ "event_id": event_id }))
                .await?;
            let after_delete = self
                .calendar_action(
                    "find",
                    serde_json::json!({
                        "query": updated_title,
                        "calendar": manifest.fixture_paths.calendar_title,
                    }),
                )
                .await?;
            Ok::<serde_json::Value, String>(serde_json::json!({
                "calendar": ensured,
                "created": created,
                "found_before_delete": found,
                "found_after_delete": after_delete,
            }))
        }
        .await;

        match result {
            Ok(evidence) => self.runtime_passed_check("calendar_crud", None, evidence),
            Err(err) => self.runtime_failed_check("calendar_crud", err, serde_json::Value::Null),
        }
    }

    async fn run_numbers_canary(&self, manifest: &DesktopCanaryManifest) -> AutonomyCheckResult {
        let Some(numbers_doc) = manifest.fixture_paths.numbers_doc.as_ref() else {
            return self.runtime_failed_check(
                "numbers_open_write_read_export",
                "missing Numbers fixture path".to_string(),
                serde_json::Value::Null,
            );
        };
        let Some(export_dir) = manifest.fixture_paths.export_dir.as_ref() else {
            return self.runtime_failed_check(
                "numbers_open_write_read_export",
                "missing Numbers export dir".to_string(),
                serde_json::Value::Null,
            );
        };
        let export_path = export_dir.join("numbers-canary.csv");
        let marker = format!("canary-{}", Uuid::new_v4().simple());

        let result = async {
            self.numbers_action("open_doc", serde_json::json!({ "path": numbers_doc }))
                .await?;
            self.numbers_action(
                "run_table_action",
                serde_json::json!({
                    "table": "Table 1",
                    "table_action": "clear_range",
                    "range": "A1:B4",
                }),
            )
            .await?;
            self.numbers_action(
                "run_table_action",
                serde_json::json!({
                    "table": "Table 1",
                    "table_action": "add_row_below",
                    "row_index": 1,
                }),
            )
            .await?;
            self.numbers_action(
                "write_range",
                serde_json::json!({
                    "table": "Table 1",
                    "cell": "A1",
                    "value": marker,
                }),
            )
            .await?;
            let read_back = self
                .numbers_action(
                    "read_range",
                    serde_json::json!({
                        "table": "Table 1",
                        "cell": "A1",
                    }),
                )
                .await?;
            let observed = read_back
                .get("value")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            if !observed.contains(&marker) {
                return Err(format!(
                    "Numbers read-back mismatch: expected marker {marker}"
                ));
            }
            self.numbers_action(
                "set_formula",
                serde_json::json!({
                    "table": "Table 1",
                    "cell": "B1",
                    "value": "=1+1",
                }),
            )
            .await?;
            self.numbers_action("export", serde_json::json!({ "export_path": export_path }))
                .await?;
            if tokio::fs::metadata(&export_path).await.is_err() {
                return Err(format!(
                    "Numbers export was not created at {}",
                    export_path.display()
                ));
            }
            Ok::<serde_json::Value, String>(serde_json::json!({
                "document": numbers_doc,
                "export_path": export_path,
                "read_back": read_back,
            }))
        }
        .await;

        match result {
            Ok(evidence) => {
                self.runtime_passed_check("numbers_open_write_read_export", None, evidence)
            }
            Err(err) => self.runtime_failed_check(
                "numbers_open_write_read_export",
                err,
                serde_json::Value::Null,
            ),
        }
    }

    async fn run_pages_canary(&self, manifest: &DesktopCanaryManifest) -> AutonomyCheckResult {
        let Some(pages_doc) = manifest.fixture_paths.pages_doc.as_ref() else {
            return self.runtime_failed_check(
                "pages_open_insert_find_export",
                "missing Pages fixture path".to_string(),
                serde_json::Value::Null,
            );
        };
        let Some(export_dir) = manifest.fixture_paths.export_dir.as_ref() else {
            return self.runtime_failed_check(
                "pages_open_insert_find_export",
                "missing Pages export dir".to_string(),
                serde_json::Value::Null,
            );
        };
        let export_path = export_dir.join("pages-canary.pdf");
        let marker = format!("ThinClaw Pages {}", Uuid::new_v4().simple());

        let result = async {
            self.pages_action("open_doc", serde_json::json!({ "path": pages_doc }))
                .await?;
            self.pages_action("insert_text", serde_json::json!({ "text": marker }))
                .await?;
            let found = self
                .pages_action("find", serde_json::json!({ "search": marker }))
                .await?;
            if found.get("found").and_then(|value| value.as_bool()) != Some(true) {
                return Err("Pages did not report the inserted marker".to_string());
            }
            self.pages_action("export", serde_json::json!({ "export_path": export_path }))
                .await?;
            if tokio::fs::metadata(&export_path).await.is_err() {
                return Err(format!(
                    "Pages export was not created at {}",
                    export_path.display()
                ));
            }
            Ok::<serde_json::Value, String>(serde_json::json!({
                "document": pages_doc,
                "export_path": export_path,
                "find_result": found,
            }))
        }
        .await;

        match result {
            Ok(evidence) => {
                self.runtime_passed_check("pages_open_insert_find_export", None, evidence)
            }
            Err(err) => self.runtime_failed_check(
                "pages_open_insert_find_export",
                err,
                serde_json::Value::Null,
            ),
        }
    }

    async fn run_textedit_canary(&self, manifest: &DesktopCanaryManifest) -> AutonomyCheckResult {
        let (app_id, app_label) = self.generic_ui_target();
        let textedit_target = manifest
            .fixture_paths
            .textedit_doc
            .clone()
            .unwrap_or_else(|| manifest.shadow_home.join("canary.txt"));
        let marker = format!("{app_label} Canary {}", Uuid::new_v4().simple());
        let result = async {
            self.apps_action("open", serde_json::json!({ "path": textedit_target }))
                .await?;
            self.apps_action("focus", serde_json::json!({ "bundle_id": app_id }))
                .await?;
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            self.ui_action(
                "type_text",
                serde_json::json!({
                    "bundle_id": app_id,
                    "text": marker,
                }),
            )
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let matches = self
                .screen_action("find_text", serde_json::json!({ "query": marker }))
                .await?;
            let found_any = matches
                .get("matches")
                .and_then(|value| value.as_array())
                .is_some_and(|items| !items.is_empty());
            if !found_any {
                return Err(format!(
                    "{app_label} fallback OCR could not find the typed marker"
                ));
            }
            Ok::<serde_json::Value, String>(matches)
        }
        .await;

        match result {
            Ok(evidence) => {
                self.runtime_passed_check("generic_ui_textedit_fallback", None, evidence)
            }
            Err(err) => self.runtime_failed_check(
                "generic_ui_textedit_fallback",
                err,
                serde_json::Value::Null,
            ),
        }
    }

    async fn promote_build(&self, build_dir: &Path) -> Result<(), String> {
        let current = self.current_build_link();
        let tmp_link = self.state_root.join("current.next");
        let _ = tokio::fs::remove_file(&tmp_link).await;
        create_symlink_dir(build_dir, &tmp_link)?;
        tokio::fs::rename(&tmp_link, &current)
            .await
            .map_err(|e| format!("failed to atomically promote build: {e}"))?;
        Ok(())
    }

    async fn trim_old_builds(&self) -> Result<(), String> {
        let manifests = self.list_build_manifests().await?;
        let current = self.current_build_id();
        let mut promoted: Vec<_> = manifests
            .into_iter()
            .filter(|manifest| manifest.promoted)
            .collect();
        promoted.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        for manifest in promoted.iter().skip(3) {
            if current.as_deref() == Some(manifest.build_id.as_str()) {
                continue;
            }
            let build_dir = self.builds_dir().join(&manifest.build_id);
            let _ = tokio::fs::remove_dir_all(build_dir).await;
        }
        Ok(())
    }

    async fn load_rollout_state(&self) -> Result<RolloutState, String> {
        match tokio::fs::read_to_string(self.rollout_state_path()).await {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| format!("failed to parse rollout state: {e}")),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(RolloutState::default()),
            Err(err) => Err(format!("failed to read rollout state: {err}")),
        }
    }

    async fn save_rollout_state(&self, state: &RolloutState) -> Result<(), String> {
        let raw = serde_json::to_string_pretty(state)
            .map_err(|e| format!("failed to serialize rollout state: {e}"))?;
        tokio::fs::write(self.rollout_state_path(), raw)
            .await
            .map_err(|e| format!("failed to write rollout state: {e}"))
    }

    async fn write_build_manifest(
        &self,
        build_id: &str,
        manifest: &BuildManifest,
    ) -> Result<(), String> {
        let path = self
            .state_root
            .join("manifests")
            .join(format!("{build_id}.json"));
        let raw = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("failed to serialize build manifest: {e}"))?;
        tokio::fs::write(path, raw)
            .await
            .map_err(|e| format!("failed to write build manifest: {e}"))
    }

    async fn list_build_manifests(&self) -> Result<Vec<BuildManifest>, String> {
        let mut results = Vec::new();
        let manifest_dir = self.state_root.join("manifests");
        if !manifest_dir.exists() {
            return Ok(results);
        }
        let mut entries = tokio::fs::read_dir(&manifest_dir)
            .await
            .map_err(|e| format!("failed to read manifest dir: {e}"))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("failed to iterate manifest dir: {e}"))?
        {
            let raw = tokio::fs::read_to_string(entry.path())
                .await
                .map_err(|e| format!("failed to read manifest {}: {e}", entry.path().display()))?;
            let manifest: BuildManifest = serde_json::from_str(&raw)
                .map_err(|e| format!("failed to parse manifest {}: {e}", entry.path().display()))?;
            results.push(manifest);
        }
        results.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(results)
    }

    async fn record_rollback_observation(&self, current_build_id: &str) -> Result<(), String> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };
        let manifests = self.list_build_manifests().await?;
        let Some(current_manifest) = manifests
            .iter()
            .find(|manifest| manifest.build_id == current_build_id)
        else {
            return Ok(());
        };
        let versions = store
            .list_learning_artifact_versions(
                &current_manifest.user_id,
                Some("code"),
                Some(current_build_id),
                5,
            )
            .await
            .map_err(|e| format!("failed to list code artifact versions for rollback: {e}"))?;
        let Some(version) = versions.first() else {
            return Ok(());
        };
        let rollback = crate::history::LearningRollbackRecord {
            id: Uuid::new_v4(),
            user_id: current_manifest.user_id.clone(),
            artifact_type: "code".to_string(),
            artifact_name: current_build_id.to_string(),
            artifact_version_id: Some(version.id),
            reason: "desktop autonomy rollback".to_string(),
            metadata: serde_json::json!({
                "build_id": current_build_id,
                "rolled_back_at": Utc::now(),
                "autonomy_control": true,
            }),
            created_at: Utc::now(),
        };
        store
            .insert_learning_rollback(&rollback)
            .await
            .map_err(|e| format!("failed to persist rollback record: {e}"))?;
        crate::agent::outcomes::observe_rollback(store, &rollback).await?;
        Ok(())
    }

    async fn evaluate_action_readiness(
        &self,
        refresh_permissions: bool,
    ) -> ActionReadinessSnapshot {
        let state = self.runtime_state.read().await.clone();
        let bootstrap_report = self.load_bootstrap_report().await.unwrap_or(None);
        let prerequisites = self.platform_bootstrap_prerequisites().await;
        let permission_summary = if refresh_permissions {
            self.desktop_permission_status()
                .await
                .unwrap_or_else(|err| {
                    serde_json::json!({
                        "ok": false,
                        "error": err,
                    })
                })
        } else {
            bootstrap_report
                .as_ref()
                .map(|report| report.permissions.clone())
                .unwrap_or(serde_json::Value::Null)
        };
        let permissions_ok =
            !permission_summary.is_null() && permissions_report_passed(&permission_summary);
        let session_ready = bootstrap_report
            .as_ref()
            .map(|report| report.session_ready)
            .unwrap_or(false);

        let blocking_reason = if !self.config.enabled {
            Some("desktop autonomy is disabled".to_string())
        } else if self.config.profile.as_str() != "reckless_desktop" {
            Some("desktop autonomy profile is not reckless_desktop".to_string())
        } else if self.emergency_stop_active() {
            Some(format!(
                "desktop autonomy is paused by emergency stop file at {}",
                self.config.emergency_stop_path.display()
            ))
        } else if state.paused {
            Some(
                state
                    .pause_reason
                    .clone()
                    .unwrap_or_else(|| "desktop autonomy is paused".to_string()),
            )
        } else if !state.bootstrap_passed {
            Some(
                bootstrap_report
                    .as_ref()
                    .and_then(|report| report.blocking_reason.clone())
                    .unwrap_or_else(|| "desktop autonomy bootstrap has not passed yet".to_string()),
            )
        } else if !session_ready {
            Some(
                bootstrap_report
                    .as_ref()
                    .and_then(|report| report.blocking_reason.clone())
                    .unwrap_or_else(|| "target desktop session is not ready".to_string()),
            )
        } else if !permissions_ok {
            Some("desktop autonomy permissions are not fully granted".to_string())
        } else if !prerequisites.passed {
            Some(prerequisites.blocking_reason.clone().unwrap_or_else(|| {
                "desktop autonomy platform prerequisites are blocking".to_string()
            }))
        } else {
            None
        };

        ActionReadinessSnapshot {
            action_ready: blocking_reason.is_none(),
            session_ready,
            blocking_reason,
            permission_summary,
            prerequisite_summary: serde_json::to_value(&prerequisites)
                .unwrap_or(serde_json::Value::Null),
        }
    }

    async fn persist_runtime_state(&self, state: &RuntimeState) -> Result<(), String> {
        self.ensure_dirs().await?;
        let raw = serde_json::to_string_pretty(state)
            .map_err(|e| format!("failed to serialize runtime state: {e}"))?;
        tokio::fs::write(self.runtime_state_path(), raw)
            .await
            .map_err(|e| format!("failed to write runtime state: {e}"))
    }

    async fn persist_bootstrap_report(
        &self,
        report: &AutonomyBootstrapReport,
    ) -> Result<(), String> {
        self.ensure_dirs().await?;
        let raw = serde_json::to_string_pretty(report)
            .map_err(|e| format!("failed to serialize bootstrap report: {e}"))?;
        tokio::fs::write(self.bootstrap_report_path(), raw)
            .await
            .map_err(|e| format!("failed to write bootstrap report: {e}"))
    }

    async fn load_bootstrap_report(&self) -> Result<Option<AutonomyBootstrapReport>, String> {
        load_json_file_async(self.bootstrap_report_path()).await
    }

    async fn latest_canary_report(&self) -> Result<Option<DesktopCanaryReport>, String> {
        let manifests = self.list_build_manifests().await?;
        for manifest in manifests {
            let Some(path) = manifest
                .metadata
                .get("canary_report_path")
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            if let Some(report) =
                load_json_file_async::<DesktopCanaryReport>(PathBuf::from(path)).await?
            {
                return Ok(Some(report));
            }
        }
        Ok(None)
    }

    fn builds_dir(&self) -> PathBuf {
        self.state_root.join("builds")
    }

    fn runtime_state_path(&self) -> PathBuf {
        self.state_root.join("runtime_state.json")
    }

    fn bootstrap_report_path(&self) -> PathBuf {
        self.state_root.join("bootstrap_report.json")
    }

    fn rollout_state_path(&self) -> PathBuf {
        self.state_root.join("rollout_state.json")
    }

    fn current_build_link(&self) -> PathBuf {
        self.state_root.join("current")
    }

    pub fn current_build_id(&self) -> Option<String> {
        let link = self.current_build_link();
        std::fs::read_link(link).ok().and_then(|path| {
            path.file_name()
                .map(|value| value.to_string_lossy().to_string())
        })
    }
}

pub fn install_global_manager(manager: Option<Arc<DesktopAutonomyManager>>) {
    match GLOBAL_MANAGER.write() {
        Ok(mut guard) => *guard = manager,
        Err(poisoned) => *poisoned.into_inner() = manager,
    }
}

pub fn desktop_autonomy_manager() -> Option<Arc<DesktopAutonomyManager>> {
    match GLOBAL_MANAGER.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub fn reckless_desktop_active() -> bool {
    desktop_autonomy_manager()
        .as_ref()
        .is_some_and(|manager| manager.is_reckless_enabled())
}

fn load_json_file<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

async fn load_json_file_async<T: DeserializeOwned>(path: PathBuf) -> Result<Option<T>, String> {
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| format!("failed to parse {}: {e}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("failed to read {}: {err}", path.display())),
    }
}

fn bootstrap_report_checks(report: &AutonomyBootstrapReport) -> Vec<AutonomyCheckResult> {
    let mut checks = Vec::new();
    checks.push(if bridge_report_passed(&report.health) {
        passed_check(
            "bridge_health",
            Some(report.health.clone()),
            report.health.clone(),
        )
    } else {
        failed_check(
            "bridge_health",
            report
                .health
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("bridge health did not pass")
                .to_string(),
            report.health.clone(),
        )
    });
    checks.push(if permissions_report_passed(&report.permissions) {
        passed_check(
            "permissions",
            Some(report.permissions.clone()),
            report.permissions.clone(),
        )
    } else {
        failed_check(
            "permissions",
            "desktop permissions are not fully granted".to_string(),
            report.permissions.clone(),
        )
    });

    if let Some(prerequisites) = report.health.get("prerequisites") {
        let passed = prerequisites
            .get("passed")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        if passed {
            checks.push(passed_check(
                "platform_prerequisites",
                Some(prerequisites.clone()),
                prerequisites.clone(),
            ));
        } else {
            checks.push(failed_check(
                "platform_prerequisites",
                prerequisites
                    .get("blocking_reason")
                    .and_then(|value| value.as_str())
                    .unwrap_or("platform prerequisites are blocking")
                    .to_string(),
                prerequisites.clone(),
            ));
        }
    }

    checks.push(if report.session_ready {
        passed_check(
            "session_ready",
            Some(serde_json::json!({ "session_ready": true })),
            serde_json::json!({ "session_ready": true }),
        )
    } else {
        failed_check(
            "session_ready",
            report
                .blocking_reason
                .clone()
                .unwrap_or_else(|| "target desktop session is not ready".to_string()),
            serde_json::json!({ "session_ready": false }),
        )
    });
    checks
}

async fn run_command_check(name: &str, command: &mut Command) -> AutonomyCheckResult {
    match run_cmd(command).await {
        Ok(_) => AutonomyCheckResult {
            name: name.to_string(),
            passed: true,
            detail: None,
            evidence: serde_json::json!({ "command": name }),
        },
        Err(err) => AutonomyCheckResult {
            name: name.to_string(),
            passed: false,
            detail: Some(err),
            evidence: serde_json::json!({ "command": name }),
        },
    }
}

pub async fn run_shadow_canary_entrypoint(
    manifest_path: &Path,
) -> Result<DesktopCanaryReport, String> {
    let raw = tokio::fs::read_to_string(manifest_path)
        .await
        .map_err(|e| format!("failed to read canary manifest: {e}"))?;
    let manifest: DesktopCanaryManifest =
        serde_json::from_str(&raw).map_err(|e| format!("failed to parse canary manifest: {e}"))?;

    let settings = crate::settings::Settings::default();
    let desktop_config = DesktopAutonomyConfig::resolve(&settings).map_err(|e| e.to_string())?;
    let database_config = DatabaseConfig::resolve().ok();
    let store = if let Some(config) = database_config.as_ref() {
        Some(
            crate::db::connect_from_config(config)
                .await
                .map_err(|e| format!("failed to connect shadow canary database: {e}"))?,
        )
    } else {
        None
    };

    let manager = DesktopAutonomyManager::new(desktop_config, database_config, store);
    manager.execute_canary_manifest(&manifest).await
}

async fn run_cmd(command: &mut Command) -> Result<String, String> {
    let output = command
        .output()
        .await
        .map_err(|e| format!("failed to spawn command: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(if detail.is_empty() {
            format!("command exited with status {}", output.status)
        } else {
            detail
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn command_on_path(name: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn permissions_report_passed(report: &serde_json::Value) -> bool {
    let object = report.as_object().cloned().unwrap_or_default();
    object.values().all(|value| {
        value.as_bool().unwrap_or_else(|| {
            value
                .as_str()
                .map(|text| !matches!(text, "denied" | "false"))
                .unwrap_or(true)
        })
    })
}

fn bridge_report_passed(report: &serde_json::Value) -> bool {
    report
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

fn trim_failed_canaries(entries: &mut Vec<DateTime<Utc>>) {
    let cutoff = Utc::now() - chrono::Duration::hours(24);
    entries.retain(|ts| *ts >= cutoff);
}

fn dedicated_bootstrap_blocking_reason(
    user_exists: bool,
    privileged: bool,
    session_ready: bool,
) -> &'static str {
    if !user_exists && !privileged {
        "requires_privileged_bootstrap"
    } else if !session_ready {
        "needs_target_user_login"
    } else {
        ""
    }
}

fn validate_numbers_payload(action: &str, payload: &serde_json::Value) -> Result<(), String> {
    if action != "run_table_action" {
        return Ok(());
    }
    let obj = payload
        .as_object()
        .ok_or_else(|| "desktop_numbers_native payload must be an object".to_string())?;
    let table_action = obj
        .get("table_action")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "run_table_action requires payload.table_action".to_string())?;
    if obj
        .get("table")
        .and_then(|value| value.as_str())
        .is_none_or(|value| value.trim().is_empty())
    {
        return Err("run_table_action requires payload.table".to_string());
    }

    match table_action {
        "add_row_above" | "add_row_below" | "delete_row" => {
            if obj
                .get("row_index")
                .and_then(|value| value.as_i64())
                .is_none()
            {
                return Err(format!(
                    "run_table_action '{table_action}' requires payload.row_index"
                ));
            }
        }
        "add_column_before"
        | "add_column_after"
        | "delete_column"
        | "sort_column_ascending"
        | "sort_column_descending" => {
            if obj
                .get("column_index")
                .and_then(|value| value.as_i64())
                .is_none()
            {
                return Err(format!(
                    "run_table_action '{table_action}' requires payload.column_index"
                ));
            }
        }
        "clear_range" => {
            if obj
                .get("range")
                .and_then(|value| value.as_str())
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err("run_table_action 'clear_range' requires payload.range".to_string());
            }
        }
        other => {
            return Err(format!(
                "unsupported run_table_action '{other}' for desktop_numbers_native"
            ));
        }
    }

    Ok(())
}

fn generate_dedicated_user_secret() -> String {
    let mut rng = rand::thread_rng();
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    (0..24)
        .map(|_| {
            let idx = rng.gen_range(0..alphabet.len());
            alphabet[idx] as char
        })
        .collect()
}

fn copy_fixture_path(src: &Path, dst: &Path) -> Result<(), String> {
    let metadata = std::fs::metadata(src)
        .map_err(|e| format!("failed to inspect fixture {}: {e}", src.display()))?;
    if metadata.is_dir() {
        std::fs::create_dir_all(dst)
            .map_err(|e| format!("failed to create fixture dir {}: {e}", dst.display()))?;
        for entry in std::fs::read_dir(src)
            .map_err(|e| format!("failed to read fixture dir {}: {e}", src.display()))?
        {
            let entry = entry
                .map_err(|e| format!("failed to read fixture entry in {}: {e}", src.display()))?;
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_fixture_path(&child_src, &child_dst)?;
        }
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            format!(
                "failed to create fixture parent dir {}: {e}",
                parent.display()
            )
        })?;
    }
    std::fs::copy(src, dst).map_err(|e| {
        format!(
            "failed to copy fixture {} -> {}: {e}",
            src.display(),
            dst.display()
        )
    })?;
    Ok(())
}

fn passed_check(
    name: &str,
    detail: Option<serde_json::Value>,
    evidence: serde_json::Value,
) -> AutonomyCheckResult {
    AutonomyCheckResult {
        name: name.to_string(),
        passed: true,
        detail: detail.map(|value| value.to_string()),
        evidence,
    }
}

fn failed_check(name: &str, detail: String, evidence: serde_json::Value) -> AutonomyCheckResult {
    AutonomyCheckResult {
        name: name.to_string(),
        passed: false,
        detail: Some(detail),
        evidence,
    }
}

#[cfg(unix)]
fn create_symlink_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(src, dst).map_err(|e| {
        format!(
            "failed to create symlink {} -> {}: {e}",
            dst.display(),
            src.display()
        )
    })
}

#[cfg(windows)]
fn create_symlink_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_dir(src, dst).map_err(|e| {
        format!(
            "failed to create symlink {} -> {}: {e}",
            dst.display(),
            src.display()
        )
    })
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn session_manager_limits_single_concurrency() {
        let manager = DesktopSessionManager::new(1);
        let lease = manager.acquire("main").await.expect("first lease");
        assert_eq!(lease.session_id(), "main");
    }

    #[test]
    fn trim_failed_canaries_keeps_recent_entries() {
        let mut entries = vec![Utc::now() - chrono::Duration::hours(25), Utc::now()];
        trim_failed_canaries(&mut entries);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn bootstrap_reason_helper_covers_dedicated_user_branches() {
        assert_eq!(
            dedicated_bootstrap_blocking_reason(false, false, false),
            "requires_privileged_bootstrap"
        );
        assert_eq!(dedicated_bootstrap_blocking_reason(false, true, true), "");
        assert_eq!(
            dedicated_bootstrap_blocking_reason(true, true, false),
            "needs_target_user_login"
        );
        assert_eq!(dedicated_bootstrap_blocking_reason(true, true, true), "");
    }

    #[test]
    fn validate_numbers_payload_requires_normalized_fields() {
        let err = validate_numbers_payload(
            "run_table_action",
            &serde_json::json!({
                "table": "Table 1",
                "table_action": "add_column_after",
            }),
        )
        .expect_err("missing column_index should fail");
        assert!(err.contains("column_index"));
    }

    #[test]
    fn canary_manifest_and_report_round_trip() {
        let manifest = DesktopCanaryManifest {
            build_id: "build-123".to_string(),
            proposal_id: "proposal-123".to_string(),
            report_path: PathBuf::from("/tmp/canary-report.json"),
            shadow_home: PathBuf::from("/tmp/shadow-home"),
            session_id: "desktop-main-session".to_string(),
            fixture_paths: DesktopFixturePaths {
                calendar_title: "ThinClaw Canary".to_string(),
                numbers_doc: Some(PathBuf::from("/tmp/canary.numbers")),
                pages_doc: Some(PathBuf::from("/tmp/canary.pages")),
                textedit_doc: Some(PathBuf::from("/tmp/canary.txt")),
                export_dir: Some(PathBuf::from("/tmp/exports")),
            },
        };
        let encoded = serde_json::to_string(&manifest).expect("serialize manifest");
        let decoded: DesktopCanaryManifest =
            serde_json::from_str(&encoded).expect("deserialize manifest");
        assert_eq!(decoded.build_id, manifest.build_id);

        let report = DesktopCanaryReport {
            build_id: manifest.build_id.clone(),
            generated_at: Utc::now(),
            passed: true,
            fixture_paths: manifest.fixture_paths.clone(),
            checks: vec![passed_check(
                "bridge_health",
                None,
                serde_json::json!({"ok": true}),
            )],
        };
        let report_encoded = serde_json::to_string(&report).expect("serialize report");
        let report_decoded: DesktopCanaryReport =
            serde_json::from_str(&report_encoded).expect("deserialize report");
        assert!(report_decoded.passed);
        assert_eq!(report_decoded.checks.len(), 1);
    }

    #[test]
    fn copy_fixture_path_supports_package_directories() {
        let temp = tempdir().expect("tempdir");
        let src = temp.path().join("source.pages");
        let nested = src.join("Data");
        std::fs::create_dir_all(&nested).expect("create source package");
        std::fs::write(src.join("Index.xml"), "<doc />").expect("write package file");
        std::fs::write(nested.join("payload.txt"), "hello").expect("write nested file");

        let dst = temp.path().join("copy.pages");
        copy_fixture_path(&src, &dst).expect("copy package dir");

        assert!(dst.join("Index.xml").exists());
        assert_eq!(
            std::fs::read_to_string(dst.join("Data").join("payload.txt"))
                .expect("read copied nested file"),
            "hello"
        );
    }

    #[test]
    fn bootstrap_report_serializes_extended_fields() {
        let report = AutonomyBootstrapReport {
            passed: false,
            health: serde_json::json!({"ok": true}),
            permissions: serde_json::json!({"accessibility": false}),
            seeded_skills: vec![PathBuf::from("/tmp/skill.md")],
            seeded_routines: vec!["daily_desktop_heartbeat".to_string()],
            launch_agent_path: Some(PathBuf::from("/tmp/test.plist")),
            launch_agent_written: true,
            launch_agent_loaded: false,
            fixture_paths: DesktopFixturePaths {
                calendar_title: "ThinClaw Canary".to_string(),
                ..Default::default()
            },
            session_ready: false,
            blocking_reason: Some("needs_target_user_login".to_string()),
            dedicated_user_keychain_label: Some("ThinClaw Desktop Autonomy/tester".to_string()),
            one_time_login_secret: Some("secret".to_string()),
            notes: vec!["note".to_string()],
        };
        let encoded = serde_json::to_string(&report).expect("serialize bootstrap report");
        assert!(encoded.contains("needs_target_user_login"));
        let decoded: AutonomyBootstrapReport =
            serde_json::from_str(&encoded).expect("deserialize bootstrap report");
        assert_eq!(decoded.fixture_paths.calendar_title, "ThinClaw Canary");
        assert_eq!(decoded.one_time_login_secret.as_deref(), Some("secret"));
    }

    #[test]
    fn bridge_spec_matches_current_host_backend() {
        let spec = DesktopBridgeSpec::current();
        match spec.backend {
            DesktopBridgeBackend::MacOsSwift => {
                assert_eq!(spec.filename, MACOS_SIDECAR_FILENAME);
                assert!(spec.source.contains("ThinClawDesktopBridge"));
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                assert_eq!(spec.filename, WINDOWS_SIDECAR_FILENAME);
                assert!(spec.source.contains("Invoke-Numbers"));
            }
            DesktopBridgeBackend::LinuxPython => {
                assert_eq!(spec.filename, LINUX_SIDECAR_FILENAME);
                assert!(spec.source.contains("invoke_numbers"));
            }
            DesktopBridgeBackend::Unsupported => {
                assert!(spec.source.is_empty());
            }
        }
    }

    #[test]
    fn runtime_evidence_adds_platform_and_providers() {
        let manager = DesktopAutonomyManager::new(
            crate::config::DesktopAutonomyConfig {
                enabled: true,
                profile: crate::settings::DesktopAutonomyProfile::RecklessDesktop,
                deployment_mode: crate::settings::DesktopDeploymentMode::WholeMachineAdmin,
                target_username: None,
                desktop_max_concurrent_jobs: 1,
                desktop_action_timeout_secs: 30,
                capture_evidence: true,
                emergency_stop_path: PathBuf::from("/tmp/stop"),
                pause_on_bootstrap_failure: true,
                kill_switch_hotkey: "ctrl+option+command+period".to_string(),
            },
            None,
            None,
        );
        let evidence = manager.attach_runtime_evidence(
            "numbers_open_write_read_export",
            serde_json::json!({"export_path": "/tmp/out.csv"}),
        );
        assert_eq!(
            evidence.get("platform").and_then(|value| value.as_str()),
            Some(manager.platform_label())
        );
        assert!(evidence.get("providers").is_some());
        assert_eq!(
            evidence
                .get("bridge_backend")
                .and_then(|value| value.as_str()),
            Some(manager.bridge_backend().as_str())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shadow_canary_process_reads_fake_runner_output() {
        let temp = tempdir().expect("tempdir");
        let report_path = temp.path().join("canary-report.json");
        let binary_path = temp.path().join("fake-runner.sh");
        let manifest_path = report_path.with_file_name("canary-manifest.json");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" != \"autonomy-shadow-canary\" ]; then exit 2; fi\ncat <<'JSON'\n{{\"build_id\":\"build-123\",\"generated_at\":\"2026-01-01T00:00:00Z\",\"passed\":true,\"fixture_paths\":{{\"calendar_title\":\"ThinClaw Canary\"}},\"checks\":[{{\"name\":\"bridge_health\",\"passed\":true,\"evidence\":{{\"ok\":true}}}}]}}\nJSON\n"
        );
        std::fs::write(&binary_path, script).expect("write fake runner");
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&binary_path)
            .expect("metadata")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&binary_path, perms).expect("chmod");

        let manifest = DesktopCanaryManifest {
            build_id: "build-123".to_string(),
            proposal_id: "proposal-123".to_string(),
            report_path: report_path.clone(),
            shadow_home: temp.path().join("shadow-home"),
            session_id: "desktop-main-session".to_string(),
            fixture_paths: DesktopFixturePaths {
                calendar_title: "ThinClaw Canary".to_string(),
                ..Default::default()
            },
        };
        std::fs::write(
            &manifest_path,
            serde_json::to_string(&manifest).expect("serialize manifest"),
        )
        .expect("write manifest");

        let manager = DesktopAutonomyManager::new(
            crate::config::DesktopAutonomyConfig {
                enabled: true,
                profile: crate::settings::DesktopAutonomyProfile::RecklessDesktop,
                deployment_mode: crate::settings::DesktopDeploymentMode::WholeMachineAdmin,
                target_username: None,
                desktop_max_concurrent_jobs: 1,
                desktop_action_timeout_secs: 30,
                capture_evidence: true,
                emergency_stop_path: temp.path().join("stop"),
                pause_on_bootstrap_failure: true,
                kill_switch_hotkey: "ctrl+option+command+period".to_string(),
            },
            None,
            None,
        );
        let report = manager
            .run_shadow_canary_process(&binary_path, &manifest)
            .await
            .expect("fake canary report");
        assert!(report.passed);
        assert_eq!(report.build_id, "build-123");
    }
}
