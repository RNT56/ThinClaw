use super::*;

pub(super) fn load_json_file<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

pub(super) async fn load_json_file_async<T: DeserializeOwned>(
    path: PathBuf,
) -> Result<Option<T>, String> {
    match tokio::fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw)
            .map(Some)
            .map_err(|e| format!("failed to parse {}: {e}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("failed to read {}: {err}", path.display())),
    }
}

pub(super) fn bootstrap_report_checks(
    report: &AutonomyBootstrapReport,
) -> Vec<AutonomyCheckResult> {
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

pub(super) async fn run_command_check(name: &str, command: &mut Command) -> AutonomyCheckResult {
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

pub(super) async fn run_cmd(command: &mut Command) -> Result<String, String> {
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

pub(super) fn command_on_path(name: &str) -> bool {
    std::process::Command::new("sh")
        .arg("-lc")
        .arg(format!("command -v {name} >/dev/null 2>&1"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(super) fn python_module_on_path(module: &str) -> bool {
    std::process::Command::new("python3")
        .arg("-c")
        .arg(format!("import {module}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(super) fn permissions_report_passed(report: &serde_json::Value) -> bool {
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

pub(super) fn bridge_report_passed(report: &serde_json::Value) -> bool {
    report
        .get("ok")
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

pub(super) fn trim_failed_canaries(entries: &mut Vec<DateTime<Utc>>) {
    let cutoff = Utc::now() - chrono::Duration::hours(24);
    entries.retain(|ts| *ts >= cutoff);
}

pub(super) fn dedicated_bootstrap_blocking_reason(
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

pub(super) fn validate_numbers_payload(
    action: &str,
    payload: &serde_json::Value,
) -> Result<(), String> {
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

pub(super) fn generate_dedicated_user_secret() -> String {
    let mut rng = rand::thread_rng();
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    (0..24)
        .map(|_| {
            let idx = rng.gen_range(0..alphabet.len());
            alphabet[idx] as char
        })
        .collect()
}

pub(super) fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

pub(super) fn linux_user_home(username: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("getent")
        .arg("passwd")
        .arg(username)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let home = raw.trim_end().split(':').nth(5)?;
    if home.is_empty() {
        None
    } else {
        Some(PathBuf::from(home))
    }
}

pub(super) fn copy_fixture_path(src: &Path, dst: &Path) -> Result<(), String> {
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

pub(super) fn passed_check(
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

pub(super) fn failed_check(
    name: &str,
    detail: String,
    evidence: serde_json::Value,
) -> AutonomyCheckResult {
    AutonomyCheckResult {
        name: name.to_string(),
        passed: false,
        detail: Some(detail),
        evidence,
    }
}

#[cfg(unix)]
pub(super) fn create_symlink_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(src, dst).map_err(|e| {
        format!(
            "failed to create symlink {} -> {}: {e}",
            dst.display(),
            src.display()
        )
    })
}

#[cfg(windows)]
pub(super) fn create_symlink_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::os::windows::fs::symlink_dir(src, dst).map_err(|e| {
        format!(
            "failed to create symlink {} -> {}: {e}",
            dst.display(),
            src.display()
        )
    })
}

#[cfg(target_os = "macos")]
pub(super) fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
