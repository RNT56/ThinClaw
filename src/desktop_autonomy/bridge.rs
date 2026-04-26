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

    pub(super) async fn ensure_sidecar_script(&self) -> Result<(), String> {
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

    pub(super) async fn bridge_call(
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
}
