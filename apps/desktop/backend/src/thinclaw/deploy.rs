//! Remote deployment over SSH.
//!
//! Deployment is intentionally fail-closed: the gateway is exposed only over
//! a Tailscale overlay, host keys use OpenSSH's accept-new TOFU policy, secrets
//! travel through stdin rather than process arguments, and the remote bundle is
//! installed transactionally at a stable path suitable for systemd.

use std::net::{IpAddr, Ipv4Addr};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};
use thinclaw_tools::execution::OwnedChild;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

const GATEWAY_PORT: u16 = 3000;
const MAX_TAILSCALE_KEY_BYTES: usize = 512;
const MAX_PROCESS_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
const MAX_CAPTURED_STDOUT_BYTES: usize = 64 * 1024;
const MAX_LOG_LINE_BYTES: usize = 16 * 1024;
const MAX_BUNDLE_FILE_BYTES: u64 = 2 * 1024 * 1024;
const SCP_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SETUP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SSH_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

struct DeployTarget {
    ip: IpAddr,
    user: String,
    tailscale_key: Zeroizing<String>,
}

impl DeployTarget {
    fn ssh_destination(&self) -> String {
        format!("{}@{}", self.user, self.ip)
    }

    fn scp_destination(&self, path: &str) -> String {
        match self.ip {
            IpAddr::V4(ip) => format!("{}@{}:{path}", self.user, ip),
            IpAddr::V6(ip) => format!("{}@[{}]:{path}", self.user, ip),
        }
    }
}

struct CommandOutput {
    stdout: String,
}

/// Connection details returned only to the renderer that initiated deployment.
///
/// The bearer token must never be broadcast through a process-global event:
/// another renderer listener could otherwise observe a credential that it did
/// not request.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct RemoteDeployResult {
    pub status: String,
    pub url: String,
    pub token: String,
    pub message: Option<String>,
    pub reachable: bool,
}

/// Deploy ThinClaw to a Linux host and return its connection details directly
/// to the caller. Only one deployment may run at a time because progress logs
/// are process-global rather than request-scoped.
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_deploy_remote(
    app: AppHandle,
    state: State<'_, super::commands::ThinClawManager>,
    ip: String,
    user: String,
    tailscale_key: Option<String>,
    enable_systemd: Option<bool>,
) -> Result<RemoteDeployResult, crate::thinclaw::bridge::BridgeError> {
    let _deployment_lease = state
        .deploy_lock
        .try_lock()
        .map_err(|_| "another remote deployment is already in progress".to_string())?;
    let target = validate_target(&ip, &user, tailscale_key)?;
    let bundle = prepare_deploy_bundle(&app)?;
    let token = Zeroizing::new(generate_secure_token());
    let deployment_id = uuid::Uuid::new_v4().simple().to_string();
    let remote_stage = format!("/tmp/thinclaw-deploy-upload-{deployment_id}");

    emit_log(&app, "=== ThinClaw Remote Deploy ===");
    emit_log(
        &app,
        &format!(
            "Target: {}@{} (host key verification: accept-new)",
            target.user, target.ip
        ),
    );
    emit_log(&app, "[1/4] Uploading the audited deployment bundle...");

    let mut scp_args = ssh_transport_options();
    scp_args.push("-r".into());
    scp_args.push("--".into());
    scp_args.push(bundle.path().to_string_lossy().into_owned());
    scp_args.push(target.scp_destination(&remote_stage));
    run_command_with_events(&app, "scp", &scp_args, None, SCP_TIMEOUT).await?;
    emit_log(&app, "[1/4] Deployment bundle uploaded.");

    emit_log(
        &app,
        "[2/4] Installing Docker, firewall rules, and the ThinClaw service...",
    );
    let remote_script = remote_install_script(
        &remote_stage,
        &deployment_id,
        enable_systemd.unwrap_or(false),
    );
    let remote_command = privilege_wrapped_shell_command(&remote_script);
    let mut ssh_args = ssh_transport_options();
    ssh_args.push("--".into());
    ssh_args.push(target.ssh_destination());
    ssh_args.push(remote_command);

    let stdin_payload =
        Zeroizing::new(format!("{}\n{}\n", &*token, &*target.tailscale_key).into_bytes());
    if let Err(error) = run_command_with_events(
        &app,
        "ssh",
        &ssh_args,
        Some(stdin_payload.as_slice()),
        SETUP_TIMEOUT,
    )
    .await
    {
        let message = format!("remote setup failed: {error}");
        emit_log(&app, &format!("[error] {message}"));
        return Err(crate::thinclaw::bridge::BridgeError::Runtime { message });
    }

    emit_log(
        &app,
        "[3/4] Discovering the private Tailscale gateway address...",
    );
    let tailscale_ip = query_tailscale_ip(&app, &target).await?;
    let gateway_url = format!("http://{tailscale_ip}:{GATEWAY_PORT}");

    // The setup script performs a server-local health check before returning.
    // This additional probe detects whether this desktop is already connected
    // to the same tailnet, but its failure does not retroactively mark a valid
    // deployment as failed.
    let proxy = crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(&gateway_url, &token)?;
    let desktop_can_reach = matches!(
        tokio::time::timeout(Duration::from_secs(10), proxy.health_check()).await,
        Ok(Ok(true))
    );
    if desktop_can_reach {
        emit_log(
            &app,
            "[4/4] Deployment complete and reachable over Tailscale.",
        );
    } else {
        emit_log(
            &app,
            "[4/4] Deployment complete. This desktop is not yet able to reach the server's tailnet address; connect it to the same tailnet before using the agent.",
        );
    }

    Ok(RemoteDeployResult {
        status: "success".to_string(),
        url: gateway_url,
        token: token.to_string(),
        message: (!desktop_can_reach).then(|| {
            "The server is healthy, but this desktop could not reach its Tailscale address yet."
                .to_string()
        }),
        reachable: desktop_can_reach,
    })
}

fn validate_target(
    raw_ip: &str,
    raw_user: &str,
    tailscale_key: Option<String>,
) -> Result<DeployTarget, String> {
    let ip = raw_ip
        .trim()
        .parse::<IpAddr>()
        .map_err(|_| "server address must be a numeric IPv4 or IPv6 address".to_string())?;
    if ip.is_unspecified() || ip.is_multicast() {
        return Err("server address cannot be unspecified or multicast".to_string());
    }

    let user = if raw_user.trim().is_empty() {
        "root"
    } else {
        raw_user.trim()
    };
    let mut chars = user.chars();
    let valid_first = chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    let valid_rest = chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'));
    if !valid_first || !valid_rest || user.len() > 32 {
        return Err(
            "SSH user must be 1-32 ASCII letters, digits, '.', '_' or '-', and may not start with punctuation"
                .to_string(),
        );
    }

    let key = tailscale_key.unwrap_or_default();
    let key = key.trim();
    if key.is_empty() {
        return Err(
            "a Tailscale auth key is required: automated deployment refuses to expose a bearer-token gateway over plaintext public HTTP"
                .to_string(),
        );
    }
    if key.len() > MAX_TAILSCALE_KEY_BYTES
        || !key.starts_with("tskey-")
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err("malformed Tailscale auth key".to_string());
    }

    Ok(DeployTarget {
        ip,
        user: user.to_string(),
        tailscale_key: Zeroizing::new(key.to_string()),
    })
}

fn prepare_deploy_bundle(app: &AppHandle) -> Result<tempfile::TempDir, String> {
    let source = find_deploy_dir(app)
        .ok_or_else(|| "bundled deployment resources are unavailable".to_string())?;
    let source = source
        .canonicalize()
        .map_err(|error| format!("failed to resolve deployment resources: {error}"))?;
    let bundle = tempfile::Builder::new()
        .prefix("thinclaw-deploy-bundle-")
        .tempdir()
        .map_err(|error| format!("failed to stage deployment resources: {error}"))?;
    #[cfg(unix)]
    std::fs::set_permissions(
        bundle.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .map_err(|error| format!("failed to secure staged deployment resources: {error}"))?;

    for (name, executable) in [
        ("setup.sh", true),
        ("docker-compose.yml", false),
        ("env.example", false),
    ] {
        let candidate = source.join(name);
        let metadata = std::fs::symlink_metadata(&candidate)
            .map_err(|error| format!("deployment resource '{name}' is unavailable: {error}"))?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_BUNDLE_FILE_BYTES {
            return Err(format!(
                "deployment resource '{name}' must be a regular file no larger than {MAX_BUNDLE_FILE_BYTES} bytes"
            ));
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("failed to resolve deployment resource '{name}': {error}"))?;
        if !canonical.starts_with(&source) {
            return Err(format!("deployment resource '{name}' escapes its bundle"));
        }
        let destination = bundle.path().join(name);
        std::fs::copy(&canonical, &destination)
            .map_err(|error| format!("failed to stage deployment resource '{name}': {error}"))?;
        #[cfg(unix)]
        std::fs::set_permissions(
            &destination,
            std::os::unix::fs::PermissionsExt::from_mode(if executable { 0o700 } else { 0o600 }),
        )
        .map_err(|error| format!("failed to secure deployment resource '{name}': {error}"))?;
    }
    Ok(bundle)
}

fn find_deploy_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    if let Ok(resource_dir) = app.path().resource_dir() {
        let production = resource_dir.join("deploy");
        if production.join("docker-compose.yml").is_file() {
            return Some(production);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            let candidate = ancestor.join("deploy");
            if candidate.join("docker-compose.yml").is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn remote_install_script(remote_stage: &str, deployment_id: &str, systemd: bool) -> String {
    let systemd_flag = if systemd { " --systemd" } else { "" };
    format!(
        r#"set -eu
umask 077
stage={remote_stage}
live=/opt/thinclaw-deploy
previous=/opt/thinclaw-deploy.previous-{deployment_id}
test ! -L "$stage"
test -f "$stage/setup.sh"
mkdir -p /opt
had_previous=false
if [ -e "$live" ]; then
  test ! -L "$live"
  mv "$live" "$previous"
  had_previous=true
fi
mv "$stage" "$live"
if [ "$had_previous" = true ] && [ -f "$previous/.env" ] && [ ! -L "$previous/.env" ]; then
  cp -p "$previous/.env" "$live/.env"
fi
rollback_install() {{
  status=$?
  if [ "$status" -ne 0 ]; then
    rm -rf -- "$live"
    if [ "$had_previous" = true ] && [ -e "$previous" ]; then
      mv "$previous" "$live"
      systemctl daemon-reload >/dev/null 2>&1 || true
      systemctl restart thinclaw.service >/dev/null 2>&1 || (cd "$live" && docker compose up -d >/dev/null 2>&1) || true
    fi
  fi
  exit "$status"
}}
trap rollback_install EXIT
chmod 0700 "$live/setup.sh"
"$live/setup.sh" --secrets-stdin{systemd_flag}
if [ "$had_previous" = true ]; then
  rm -rf -- "$previous"
fi
trap - EXIT
"#,
    )
}

fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn privilege_wrapped_shell_command(script: &str) -> String {
    let quoted = shell_single_quote(script);
    format!(
        "if [ \"$(id -u)\" -eq 0 ]; then exec sh -c {quoted}; else exec sudo -n sh -c {quoted}; fi"
    )
}

fn ssh_transport_options() -> Vec<String> {
    [
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=4",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

async fn query_tailscale_ip(app: &AppHandle, target: &DeployTarget) -> Result<Ipv4Addr, String> {
    let query = privilege_wrapped_shell_command("exec tailscale ip -4");
    let mut args = ssh_transport_options();
    args.push("--".into());
    args.push(target.ssh_destination());
    args.push(query);
    let output = run_command_with_events(app, "ssh", &args, None, SSH_QUERY_TIMEOUT).await?;
    let ip = output
        .stdout
        .lines()
        .find_map(|line| line.trim().parse::<Ipv4Addr>().ok())
        .ok_or_else(|| {
            "remote setup succeeded but returned no Tailscale IPv4 address".to_string()
        })?;
    let numeric = u32::from(ip);
    let cgnat_start = u32::from(Ipv4Addr::new(100, 64, 0, 0));
    if numeric < cgnat_start || numeric >= cgnat_start + (1 << 22) {
        return Err("remote Tailscale address is outside the expected 100.64.0.0/10 range".into());
    }
    Ok(ip)
}

fn generate_secure_token() -> String {
    let bytes: [u8; 32] = rand::random();
    hex::encode(bytes)
}

fn emit_log(app: &AppHandle, message: &str) {
    let _ = app.emit("deploy-log", message);
}

async fn stream_process_output<R: AsyncRead + Unpin>(
    app: AppHandle,
    mut reader: R,
    prefix: &'static str,
    total: Arc<AtomicUsize>,
) -> Result<String, String> {
    let mut chunk = [0_u8; 8192];
    let mut line = Vec::with_capacity(1024);
    let mut line_truncated = false;
    let mut captured = Vec::new();

    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| format!("failed to read subprocess output: {error}"))?;
        if count == 0 {
            break;
        }
        let prior = total.fetch_add(count, Ordering::AcqRel);
        if prior.saturating_add(count) > MAX_PROCESS_OUTPUT_BYTES {
            return Err(format!(
                "subprocess output exceeded the {MAX_PROCESS_OUTPUT_BYTES}-byte limit"
            ));
        }
        let remaining_capture = MAX_CAPTURED_STDOUT_BYTES.saturating_sub(captured.len());
        captured.extend_from_slice(&chunk[..count.min(remaining_capture)]);

        for byte in &chunk[..count] {
            if *byte == b'\n' {
                emit_process_line(&app, prefix, &line, line_truncated);
                line.clear();
                line_truncated = false;
            } else if line.len() < MAX_LOG_LINE_BYTES {
                line.push(*byte);
            } else {
                line_truncated = true;
            }
        }
    }
    if !line.is_empty() || line_truncated {
        emit_process_line(&app, prefix, &line, line_truncated);
    }
    Ok(String::from_utf8_lossy(&captured).into_owned())
}

fn emit_process_line(app: &AppHandle, prefix: &str, bytes: &[u8], truncated: bool) {
    let decoded = String::from_utf8_lossy(bytes);
    let sanitized: String = decoded
        .chars()
        .filter(|ch| *ch == '\t' || (!ch.is_control() && *ch != '\u{7f}'))
        .collect();
    let suffix = if truncated { " [line truncated]" } else { "" };
    let _ = app.emit("deploy-log", format!("{prefix}{sanitized}{suffix}"));
}

async fn run_command_with_events(
    app: &AppHandle,
    program: &str,
    args: &[String],
    stdin_payload: Option<&[u8]>,
    timeout: Duration,
) -> Result<CommandOutput, String> {
    if stdin_payload.is_some_and(|payload| payload.len() > 4096) {
        return Err("subprocess stdin payload exceeds the deployment limit".into());
    }
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(if stdin_payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "PATH",
            format!(
                "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
                std::env::var("PATH").unwrap_or_default()
            ),
        );
    let mut child = OwnedChild::spawn(&mut command)
        .map_err(|error| format!("failed to start {program}: {error}"))?;

    if let Some(payload) = stdin_payload {
        let mut stdin = child
            .take_stdin()
            .ok_or_else(|| format!("failed to open {program} stdin"))?;
        tokio::time::timeout(Duration::from_secs(5), async {
            stdin.write_all(payload).await?;
            stdin.shutdown().await
        })
        .await
        .map_err(|_| format!("timed out writing {program} stdin"))?
        .map_err(|error| format!("failed to write {program} stdin: {error}"))?;
    }

    let stdout = child
        .take_stdout()
        .ok_or_else(|| format!("failed to capture {program} stdout"))?;
    let stderr = child
        .take_stderr()
        .ok_or_else(|| format!("failed to capture {program} stderr"))?;
    let total = Arc::new(AtomicUsize::new(0));
    let (failure_tx, mut failure_rx) = tokio::sync::mpsc::channel::<String>(2);
    let stdout_task = {
        let app = app.clone();
        let total = Arc::clone(&total);
        let failure_tx = failure_tx.clone();
        tokio::spawn(async move {
            let result = stream_process_output(app, stdout, "", total).await;
            if let Err(error) = &result {
                let _ = failure_tx.send(error.clone()).await;
            }
            result
        })
    };
    let stderr_task = {
        let app = app.clone();
        let total = Arc::clone(&total);
        let failure_tx = failure_tx.clone();
        tokio::spawn(async move {
            let result = stream_process_output(app, stderr, "[stderr] ", total).await;
            if let Err(error) = &result {
                let _ = failure_tx.send(error.clone()).await;
            }
            result
        })
    };
    // Keep the channel open so ordinary pipe EOF does not race process exit.
    let _failure_guard = failure_tx;

    enum WaitOutcome {
        Exited(std::io::Result<std::process::ExitStatus>),
        ReaderFailed(String),
        TimedOut,
    }
    let outcome = {
        let wait = child.wait();
        tokio::pin!(wait);
        tokio::select! {
            status = &mut wait => WaitOutcome::Exited(status),
            Some(error) = failure_rx.recv() => WaitOutcome::ReaderFailed(error),
            _ = tokio::time::sleep(timeout) => WaitOutcome::TimedOut,
        }
    };

    let status = match outcome {
        WaitOutcome::Exited(status) => {
            status.map_err(|error| format!("failed to wait for {program}: {error}"))?
        }
        WaitOutcome::ReaderFailed(error) => {
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(error);
        }
        WaitOutcome::TimedOut => {
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(format!(
                "{program} timed out after {} seconds",
                timeout.as_secs()
            ));
        }
    };

    let stdout = tokio::time::timeout(Duration::from_secs(5), stdout_task)
        .await
        .map_err(|_| format!("timed out draining {program} stdout"))?
        .map_err(|error| format!("{program} stdout task failed: {error}"))??;
    let stderr = tokio::time::timeout(Duration::from_secs(5), stderr_task)
        .await
        .map_err(|_| format!("timed out draining {program} stderr"))?
        .map_err(|error| format!("{program} stderr task failed: {error}"))??;
    if !status.success() {
        let detail = stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .unwrap_or("no diagnostic output");
        return Err(format!(
            "{program} exited with status {}: {detail}",
            status
                .code()
                .map_or_else(|| "signal".into(), |code| code.to_string())
        ));
    }
    Ok(CommandOutput { stdout })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_input_validation_rejects_shell_syntax_and_public_plaintext_mode() {
        assert!(validate_target(
            "1.2.3.4; touch /tmp/pwn",
            "root",
            Some("tskey-auth-safe".into())
        )
        .is_err());
        assert!(validate_target("1.2.3.4", "root;id", Some("tskey-auth-safe".into())).is_err());
        assert!(
            validate_target("1.2.3.4", "root", Some("tskey-auth-safe\nmalicious".into())).is_err()
        );
        assert!(validate_target("1.2.3.4", "root", None).is_err());
        assert!(validate_target("1.2.3.4", "ubuntu", Some("tskey-auth-safe_123".into())).is_ok());
    }

    #[test]
    fn remote_script_and_debuggable_arguments_never_contain_secrets() {
        let script = remote_install_script("/tmp/thinclaw-deploy-upload-safe", "abc123", true);
        let command = privilege_wrapped_shell_command(&script);
        assert!(command.contains("--secrets-stdin"));
        assert!(!command.contains("gateway-live-secret"));
        assert!(!command.contains("tailscale-live-secret"));
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn tailscale_range_validation_boundaries_are_correct() {
        let start = u32::from(Ipv4Addr::new(100, 64, 0, 0));
        let end = start + (1 << 22);
        assert!(u32::from(Ipv4Addr::new(100, 64, 0, 1)) >= start);
        assert!(u32::from(Ipv4Addr::new(100, 127, 255, 254)) < end);
        assert!(u32::from(Ipv4Addr::new(100, 128, 0, 1)) >= end);
    }
}
