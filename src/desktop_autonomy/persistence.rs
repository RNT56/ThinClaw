use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn evaluate_action_readiness(
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

    pub(super) async fn persist_runtime_state(&self, state: &RuntimeState) -> Result<(), String> {
        self.ensure_dirs().await?;
        let raw = serde_json::to_string_pretty(state)
            .map_err(|e| format!("failed to serialize runtime state: {e}"))?;
        tokio::fs::write(self.runtime_state_path(), raw)
            .await
            .map_err(|e| format!("failed to write runtime state: {e}"))
    }

    pub(super) async fn persist_bootstrap_report(
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

    pub(super) async fn load_bootstrap_report(
        &self,
    ) -> Result<Option<AutonomyBootstrapReport>, String> {
        load_json_file_async(self.bootstrap_report_path()).await
    }

    pub(super) async fn latest_canary_report(&self) -> Result<Option<DesktopCanaryReport>, String> {
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

    pub(super) fn builds_dir(&self) -> PathBuf {
        self.state_root.join("builds")
    }

    pub(super) fn runtime_state_path(&self) -> PathBuf {
        self.state_root.join("runtime_state.json")
    }

    pub(super) fn bootstrap_report_path(&self) -> PathBuf {
        self.state_root.join("bootstrap_report.json")
    }

    pub(super) fn rollout_state_path(&self) -> PathBuf {
        self.state_root.join("rollout_state.json")
    }

    pub(super) fn current_build_link(&self) -> PathBuf {
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
