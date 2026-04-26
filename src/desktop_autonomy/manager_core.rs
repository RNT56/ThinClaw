use super::*;
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

    pub(super) fn bridge_spec(&self) -> DesktopBridgeSpec {
        DesktopBridgeSpec::current()
    }

    pub(super) fn bridge_backend(&self) -> DesktopBridgeBackend {
        self.bridge_spec().backend
    }

    pub(super) fn platform_label(&self) -> &'static str {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => "macos",
            DesktopBridgeBackend::WindowsPowerShell => "windows",
            DesktopBridgeBackend::LinuxPython => "linux",
            DesktopBridgeBackend::Unsupported => "unsupported",
        }
    }

    pub(super) fn provider_matrix(&self) -> serde_json::Value {
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

    pub(super) fn fixture_extensions(&self) -> (&'static str, &'static str) {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => ("numbers", "pages"),
            DesktopBridgeBackend::WindowsPowerShell => ("xlsx", "docx"),
            DesktopBridgeBackend::LinuxPython => ("ods", "odt"),
            DesktopBridgeBackend::Unsupported => ("numbers", "pages"),
        }
    }

    pub(super) fn generic_ui_provider(&self) -> String {
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

    pub(super) fn generic_ui_target(&self) -> (String, String) {
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

    pub(super) fn attach_runtime_evidence(
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

    pub(super) fn runtime_passed_check(
        &self,
        name: &str,
        detail: Option<serde_json::Value>,
        evidence: serde_json::Value,
    ) -> AutonomyCheckResult {
        passed_check(name, detail, self.attach_runtime_evidence(name, evidence))
    }

    pub(super) fn runtime_failed_check(
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
}
