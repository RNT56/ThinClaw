use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn domain_action(
        &self,
        domain: &str,
        action: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.bridge_domain_action(domain, action, payload, true)
            .await
    }

    pub(super) async fn bridge_domain_action(
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

    pub(super) async fn ensure_dirs(&self) -> Result<(), String> {
        for (label, path) in [
            ("autonomy state root", self.state_root.clone()),
            ("builds dir", self.builds_dir()),
            ("canaries dir", self.canaries_dir()),
            ("manifests dir", self.state_root.join("manifests")),
            ("fixtures dir", self.fixtures_dir()),
        ] {
            tokio::fs::create_dir_all(&path)
                .await
                .map_err(|error| format!("failed to create {label}: {error}"))?;
            let metadata = tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|error| format!("failed to inspect {label}: {error}"))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!("{label} is not a real directory"));
            }
        }
        Ok(())
    }

    pub(super) async fn ensure_sidecar_script(&self) -> Result<(), String> {
        self.ensure_dirs().await?;
        let spec = self.bridge_spec();
        if matches!(spec.backend, DesktopBridgeBackend::Unsupported) {
            return Err("desktop autonomy bridge is not supported on this platform".to_string());
        }
        let should_write = match read_autonomy_file(self.sidecar_script_path.clone()).await? {
            Some(existing) => existing != spec.source.as_bytes(),
            None => true,
        };
        if should_write {
            write_autonomy_file(
                self.sidecar_script_path.clone(),
                spec.source.as_bytes().to_vec(),
            )
            .await
            .map_err(|e| format!("failed to write desktop sidecar script: {e}"))?;
        }
        Ok(())
    }

    pub(super) async fn bridge_call(
        &self,
        command: &str,
        payload: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        const MAX_BRIDGE_INPUT_BYTES: usize = 2 * 1024 * 1024;
        const MAX_BRIDGE_STDOUT_BYTES: usize = 8 * 1024 * 1024;
        const MAX_BRIDGE_STDERR_BYTES: usize = 1024 * 1024;
        const MAX_ERROR_PREVIEW_BYTES: usize = 32 * 1024;

        let spec = self.bridge_spec();
        let mut child_command = match spec.backend {
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
        };
        let input = serde_json::to_vec(&payload)
            .map_err(|e| format!("failed to encode bridge request: {e}"))?;
        if input.len() > MAX_BRIDGE_INPUT_BYTES {
            return Err("desktop sidecar request exceeds its size limit".to_string());
        }
        let output = thinclaw_platform::bounded_command_output_with_input(
            &mut child_command,
            &input,
            std::time::Duration::from_secs(self.config.desktop_action_timeout_secs.max(5)),
            MAX_BRIDGE_STDOUT_BYTES,
            MAX_BRIDGE_STDERR_BYTES,
        )
        .await
        .map_err(|e| format!("desktop sidecar execution failed: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(
                output
                    .stderr
                    .get(..MAX_ERROR_PREVIEW_BYTES)
                    .unwrap_or(&output.stderr),
            )
            .trim()
            .to_string();
            return Err(if stderr.is_empty() {
                format!("desktop sidecar failed with {}", output.status)
            } else {
                format!("desktop sidecar failed: {stderr}")
            });
        }

        let value: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("failed to decode desktop sidecar response: {e}"))?;
        if value.get("ok").and_then(|value| value.as_bool()) != Some(true) {
            return Err(value
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or("desktop sidecar returned an invalid or failed response")
                .to_string());
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| "desktop sidecar response omitted result".to_string())
    }
}
