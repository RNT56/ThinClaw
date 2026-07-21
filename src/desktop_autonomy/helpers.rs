use super::*;

const MAX_AUTONOMY_STATE_FILE_BYTES: u64 = 16 * 1024 * 1024;

pub(super) fn autonomy_state_sidecar(path: &Path) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "autonomy state path has no valid filename".to_string())?;
    Ok(path.with_file_name(format!(".{name}.state-sidecar")))
}

pub(super) fn read_autonomy_file_sync(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read as _;

    let sidecar = autonomy_state_sidecar(path)?;
    thinclaw_platform::recover_file_pair_sync(path, &sidecar)
        .map_err(|error| format!("failed to recover {}: {error}", path.display()))?;
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_AUTONOMY_STATE_FILE_BYTES
    {
        return Err(format!(
            "autonomy state file {} is not a bounded regular file",
            path.display()
        ));
    }
    let _guard = thinclaw_platform::acquire_artifact_read_lock_sync(path)
        .map_err(|error| format!("failed to lock {}: {error}", path.display()))?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;
    if !opened_metadata.is_file()
        || opened_metadata.len() != metadata.len()
        || opened_metadata.len() > MAX_AUTONOMY_STATE_FILE_BYTES
    {
        return Err(format!("autonomy state file {} changed", path.display()));
    }
    let opened_len = usize::try_from(opened_metadata.len())
        .map_err(|_| "autonomy state file does not fit this platform".to_string())?;
    let mut bytes = Vec::with_capacity(opened_len);
    file.take(MAX_AUTONOMY_STATE_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    if bytes.len() != opened_len {
        return Err(format!("autonomy state file {} changed", path.display()));
    }
    Ok(Some(bytes))
}

pub(super) async fn read_autonomy_file(path: PathBuf) -> Result<Option<Vec<u8>>, String> {
    tokio::task::spawn_blocking(move || read_autonomy_file_sync(&path))
        .await
        .map_err(|error| format!("autonomy state reader panicked: {error}"))?
}

pub(super) async fn write_autonomy_file(path: PathBuf, bytes: Vec<u8>) -> Result<(), String> {
    if bytes.len() > MAX_AUTONOMY_STATE_FILE_BYTES as usize {
        return Err("autonomy state file exceeds its size limit".to_string());
    }
    let sidecar = autonomy_state_sidecar(&path)?;
    thinclaw_platform::publish_file_pair(
        path,
        sidecar,
        bytes,
        None,
        thinclaw_platform::ExistingPairPolicy::Replace,
    )
    .await
    .map_err(|error| format!("failed to publish autonomy state: {error}"))
}

pub(super) async fn remove_autonomy_file(path: PathBuf) -> Result<(), String> {
    let sidecar = autonomy_state_sidecar(&path)?;
    thinclaw_platform::remove_file_pair(path, sidecar)
        .await
        .map_err(|error| format!("failed to remove autonomy state: {error}"))
}

pub(super) fn load_json_file_sync<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let Some(raw) = read_autonomy_file_sync(path)? else {
        return Ok(None);
    };
    serde_json::from_slice(&raw)
        .map(Some)
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))
}

pub(super) async fn load_json_file_async<T: DeserializeOwned>(
    path: PathBuf,
) -> Result<Option<T>, String> {
    let raw = read_autonomy_file(path.clone()).await?;
    raw.map(|bytes| {
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))
    })
    .transpose()
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
    let manifest: DesktopCanaryManifest = load_json_file_async(manifest_path.to_path_buf())
        .await?
        .ok_or_else(|| "canary manifest does not exist".to_string())?;

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
    const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);
    const COMMAND_OUTPUT_LIMIT: usize = 8 * 1024 * 1024;
    const ERROR_PREVIEW_LIMIT: usize = 32 * 1024;

    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_PAGER", "cat")
        .env("GIT_CONFIG_COUNT", "2")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env(
            "GIT_CONFIG_VALUE_0",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_CONFIG_KEY_1", "commit.gpgSign")
        .env("GIT_CONFIG_VALUE_1", "false");
    let output = thinclaw_platform::bounded_command_output(
        command,
        COMMAND_TIMEOUT,
        COMMAND_OUTPUT_LIMIT,
        COMMAND_OUTPUT_LIMIT,
    )
    .await
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr_bytes = output
            .stderr
            .get(..ERROR_PREVIEW_LIMIT)
            .unwrap_or(&output.stderr);
        let stdout_bytes = output
            .stdout
            .get(..ERROR_PREVIEW_LIMIT)
            .unwrap_or(&output.stdout);
        let stderr = String::from_utf8_lossy(stderr_bytes).trim().to_string();
        let stdout = String::from_utf8_lossy(stdout_bytes).trim().to_string();
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
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && thinclaw_platform::find_executable_in_path(name).is_some()
}

pub(super) async fn python_module_on_path(module: &str) -> bool {
    if module.is_empty()
        || !module.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
    {
        return false;
    }
    run_cmd(
        Command::new("python3")
            .arg("-c")
            .arg(format!("import {module}")),
    )
    .await
    .is_ok()
}

pub(super) fn permissions_report_passed(report: &serde_json::Value) -> bool {
    let Some(object) = report.as_object() else {
        return false;
    };
    if object
        .get("accessibility")
        .and_then(|value| value.as_bool())
        != Some(true)
        || object
            .get("screen_recording")
            .and_then(|value| value.as_bool())
            != Some(true)
    {
        return false;
    }

    let available = |key: &str, allowed: &[&str]| {
        object
            .get(key)
            .and_then(|value| value.as_str())
            .is_some_and(|value| allowed.contains(&value))
    };
    match object.get("platform").and_then(|value| value.as_str()) {
        Some("macos") => available("calendar", &["authorized"]),
        Some("windows") => {
            available("calendar", &["available"])
                && available("excel", &["available"])
                && available("word", &["available"])
                && available("ocr", &["available"])
        }
        Some("linux") => available("calendar", &["available"]) && available("ocr", &["available"]),
        _ => false,
    }
}

pub(super) fn bridge_report_passed(report: &serde_json::Value) -> bool {
    report.as_object().is_some() && report.get("ok").and_then(|value| value.as_bool()) == Some(true)
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
    let mut rng = rand::rng();
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    (0..24)
        .map(|_| {
            let idx = rng.random_range(0..alphabet.len());
            alphabet[idx] as char
        })
        .collect()
}

pub(super) fn shell_single_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

pub(super) fn linux_user_home(username: &str) -> Option<PathBuf> {
    if username.is_empty()
        || username.len() > 256
        || !username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    let bytes =
        thinclaw_platform::read_regular_file_bounded(Path::new("/etc/passwd"), 4 * 1024 * 1024)
            .ok()?;
    let raw = std::str::from_utf8(&bytes).ok()?;
    let home = raw.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == username)
            .then(|| fields.nth(4))
            .flatten()
    })?;
    let home = PathBuf::from(home);
    home.is_absolute().then_some(home)
}

pub(super) fn copy_fixture_path(src: &Path, dst: &Path) -> Result<(), String> {
    let mut budget = FixtureCopyBudget::default();
    copy_fixture_path_inner(src, dst, 0, &mut budget)
}

#[derive(Default)]
struct FixtureCopyBudget {
    entries: usize,
    bytes: u64,
}

fn copy_fixture_path_inner(
    src: &Path,
    dst: &Path,
    depth: usize,
    budget: &mut FixtureCopyBudget,
) -> Result<(), String> {
    const MAX_FIXTURE_DEPTH: usize = 64;
    const MAX_FIXTURE_ENTRIES: usize = 20_000;
    const MAX_FIXTURE_FILE_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_FIXTURE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;

    if depth > MAX_FIXTURE_DEPTH {
        return Err("fixture package exceeds its nesting limit".to_string());
    }
    budget.entries = budget
        .entries
        .checked_add(1)
        .ok_or_else(|| "fixture entry count overflowed".to_string())?;
    if budget.entries > MAX_FIXTURE_ENTRIES {
        return Err("fixture package exceeds its entry limit".to_string());
    }

    let metadata = std::fs::symlink_metadata(src)
        .map_err(|e| format!("failed to inspect fixture {}: {e}", src.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "fixture {} is a symlink and cannot be copied",
            src.display()
        ));
    }
    if metadata.is_dir() {
        match std::fs::symlink_metadata(dst) {
            Ok(existing) if existing.file_type().is_symlink() || !existing.is_dir() => {
                return Err(format!(
                    "fixture destination {} is not a real directory",
                    dst.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(dst)
                    .map_err(|e| format!("failed to create fixture dir {}: {e}", dst.display()))?;
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect fixture destination {}: {error}",
                    dst.display()
                ));
            }
        }
        for entry in std::fs::read_dir(src)
            .map_err(|e| format!("failed to read fixture dir {}: {e}", src.display()))?
        {
            let entry = entry
                .map_err(|e| format!("failed to read fixture entry in {}: {e}", src.display()))?;
            let child_src = entry.path();
            let child_dst = dst.join(entry.file_name());
            copy_fixture_path_inner(&child_src, &child_dst, depth + 1, budget)?;
        }
        return Ok(());
    }
    if !metadata.is_file() || metadata.len() > MAX_FIXTURE_FILE_BYTES {
        return Err(format!(
            "fixture {} is not a bounded regular file",
            src.display()
        ));
    }
    budget.bytes = budget
        .bytes
        .checked_add(metadata.len())
        .ok_or_else(|| "fixture byte count overflowed".to_string())?;
    if budget.bytes > MAX_FIXTURE_TOTAL_BYTES {
        return Err("fixture package exceeds its total size limit".to_string());
    }
    let parent = dst
        .parent()
        .ok_or_else(|| "fixture destination has no parent".to_string())?;
    let parent_metadata = std::fs::symlink_metadata(parent).map_err(|error| {
        format!(
            "failed to inspect fixture parent {}: {error}",
            parent.display()
        )
    })?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(format!(
            "fixture destination parent {} is not a real directory",
            parent.display()
        ));
    }

    let mut source_options = std::fs::OpenOptions::new();
    source_options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        source_options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut source = source_options
        .open(src)
        .map_err(|error| format!("failed to open fixture {}: {error}", src.display()))?;
    let opened_metadata = source
        .metadata()
        .map_err(|error| format!("failed to re-inspect fixture {}: {error}", src.display()))?;
    if opened_metadata.len() != metadata.len() {
        return Err(format!("fixture {} changed while copying", src.display()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if opened_metadata.dev() != metadata.dev() || opened_metadata.ino() != metadata.ino() {
            return Err(format!("fixture {} changed while opening", src.display()));
        }
    }
    let mut destination_options = std::fs::OpenOptions::new();
    destination_options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        destination_options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW);
    }
    let mut destination = destination_options.open(dst).map_err(|error| {
        format!(
            "failed to create fixture destination {}: {error}",
            dst.display()
        )
    })?;
    let mut limited_source = std::io::Read::take(&mut source, metadata.len() + 1);
    let copy_result = std::io::copy(&mut limited_source, &mut destination).and_then(|copied| {
        if copied == metadata.len() {
            destination.sync_all()
        } else {
            Err(std::io::Error::other("fixture changed while copying"))
        }
    });
    if let Err(error) = copy_result {
        drop(destination);
        let _ = std::fs::remove_file(dst);
        return Err(format!(
            "failed to copy fixture {} -> {}: {error}",
            src.display(),
            dst.display()
        ));
    }
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
