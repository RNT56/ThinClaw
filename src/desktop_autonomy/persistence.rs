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
        write_autonomy_file(self.runtime_state_path(), raw.into_bytes())
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
        write_autonomy_file(self.bootstrap_report_path(), raw.into_bytes())
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
        if manifests.is_empty() {
            return Ok(None);
        }
        let canaries_root = self
            .canaries_dir()
            .canonicalize()
            .map_err(|error| format!("failed to resolve autonomy canaries directory: {error}"))?;
        for manifest in manifests {
            let Some(path) = manifest
                .metadata
                .get("canary_report_path")
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            let path = PathBuf::from(path);
            let expected_path = self
                .canaries_dir()
                .join(&manifest.build_id)
                .join("canary-report.json");
            if path != expected_path {
                return Err("build manifest references the wrong canary report".to_string());
            }
            let canonical_path = path
                .canonicalize()
                .map_err(|error| format!("failed to resolve canary report path: {error}"))?;
            if !canonical_path.starts_with(&canaries_root) {
                return Err(
                    "canary report path escapes the autonomy canaries directory".to_string()
                );
            }
            if let Some(report) =
                load_json_file_async::<DesktopCanaryReport>(canonical_path).await?
            {
                return Ok(Some(report));
            }
        }
        Ok(None)
    }

    pub(super) fn builds_dir(&self) -> PathBuf {
        self.state_root.join("builds")
    }

    pub(super) fn canaries_dir(&self) -> PathBuf {
        self.state_root.join("canaries")
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

    pub(super) fn current_build_id_checked(&self) -> Result<Option<String>, String> {
        let link = self.current_build_link();
        match std::fs::symlink_metadata(&link) {
            Ok(metadata) if !metadata.file_type().is_symlink() => {
                return Err("autonomy current-build path is not a symlink".to_string());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "failed to inspect autonomy current-build link: {error}"
                ));
            }
        }
        let canonical_build = link
            .canonicalize()
            .map_err(|error| format!("failed to resolve autonomy current-build link: {error}"))?;
        let binary = self.shadow_binary_path(&canonical_build);
        let canonical_build = super::rollout_helpers::validate_promotable_build_sync(
            &self.builds_dir(),
            &canonical_build,
            &binary,
        )?;
        let build_id = canonical_build
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "autonomy current build has no valid identifier".to_string())?;
        Ok(Some(build_id.to_string()))
    }

    pub fn current_build_id(&self) -> Option<String> {
        match self.current_build_id_checked() {
            Ok(build_id) => build_id,
            Err(error) => {
                tracing::error!(%error, "autonomy current-build link is invalid");
                None
            }
        }
    }
}
