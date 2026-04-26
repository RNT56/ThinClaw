use super::*;
impl DesktopAutonomyManager {
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
}
