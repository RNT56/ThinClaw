//! Remote deployment command — SSH + Docker Compose approach.
//!
//! Previous implementation relied on an Ansible/shell script that wasn't
//! bundled. This version is fully self-contained: it uses `ssh` and `scp`
//! (available on all major platforms) to deploy a Docker Compose stack
//! to any Linux server.
//!
//! ## Deploy flow:
//!   1. SCP the ThinClaw `deploy/` bundle to the target server
//!   2. Generate a secure auth token locally and send it through SSH stdin
//!   3. Run `docker compose up -d --build` via SSH
//!   4. Emit `deploy-log` events to the frontend for live progress
//!   5. Return the credential only to the initiating command caller
//!
//! ## Connect flow (new):
//!   - Simply test connectivity and return the URL + token to the frontend.
//!   - The frontend calls `thinclaw_switch_to_profile` to activate the connection.

use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use zeroize::{Zeroize, Zeroizing};

const REMOTE_GATEWAY_PORT: u16 = 3000;
const MAX_DEPLOY_LOG_LINE_BYTES: usize = 64 * 1024;

#[derive(serde::Serialize, specta::Type, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDeployResult {
    pub status: String,
    pub url: String,
    pub token: String,
    pub message: Option<String>,
}

/// Deploy the ThinClaw remote agent to a Linux server via SSH + Docker Compose.
///
/// Accepts the SSH host, user, and optional configuration for Tailscale VPN
/// and systemd service. Emits credential-free `deploy-log` events and returns
/// the generated credential only to the initiating IPC request.
///
/// Steps:
///   1. Find the ThinClaw `deploy/` directory (bundled or source)
///   2. SCP the deploy bundle to the target server
///   3. Run `setup.sh` on the server:
///      - Always: Docker, UFW firewall, Fail2ban
///      - Optional: Tailscale VPN (--tailscale <key>)
///      - Optional: systemd service (--systemd)
///   4. Return the URL + generated token as the command result
#[tauri::command]
#[specta::specta]
pub async fn thinclaw_deploy_remote(
    app: AppHandle,
    _state: State<'_, super::commands::ThinClawManager>,
    ip: String,
    user: String,
    tailscale_key: Option<String>,
    enable_systemd: Option<bool>,
) -> Result<RemoteDeployResult, String> {
    // ── Validate input ────────────────────────────────────────────────────────
    let ip = validate_ssh_host(&ip)?;
    let user = if user.trim().is_empty() {
        "root".to_string()
    } else {
        validate_ssh_user(&user)?
    };
    let tailscale_key = tailscale_key
        .filter(|key| !key.trim().is_empty())
        .map(|key| validate_tailscale_key(&key).map(Zeroizing::new))
        .transpose()?;
    let ssh_target = format!("{}@{}", user, ip);
    if tailscale_key.is_none() {
        let candidate_url = format!("http://{}:{}", url_host(&ip), REMOTE_GATEWAY_PORT);
        crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(
            &candidate_url,
            "transport-validation-only",
        )
        .map_err(|error| {
            format!(
                "This deployment would expose a bearer token over an unsafe transport: {error}. Use a private IP or configure Tailscale."
            )
        })?;
    }

    // ── Locate deploy bundle ──────────────────────────────────────────────────
    //
    // In production builds: bundled with the app as `resources/deploy/`
    // In dev builds: root `deploy/` from the ThinClaw repository.
    let deploy_dir = find_deploy_dir(&app);

    let emit = |app_handle: &AppHandle, msg: &str| {
        let _ = app_handle.emit("deploy-log", msg);
    };

    emit(&app, "=== ThinClaw Remote Deploy ===");
    emit(&app, &format!("Target: {}@{}", user, ip));

    // ── Step 1: Copy deploy bundle via SCP ───────────────────────────────────
    emit(&app, "[1/4] Copying deploy bundle to remote server...");

    if let Some(ref dir) = deploy_dir {
        let scp_result = run_command_with_events(
            &app,
            "scp",
            &[
                "-o".to_string(),
                "StrictHostKeyChecking=accept-new".to_string(),
                "-o".to_string(),
                "BatchMode=yes".to_string(),
                "-r".to_string(),
                "--".to_string(),
                dir.to_string_lossy().to_string(),
                format!("{}@{}:/tmp/thinclaw-deploy", user, url_host(&ip)),
            ],
            None,
            Vec::new(),
        )
        .await;

        if let Err(e) = scp_result {
            let msg = format!("SCP failed: {}. Is SSH access configured?", e);
            return Err(msg);
        }
        emit(&app, "[1/4] Deploy bundle copied.");
    } else {
        emit(
            &app,
            "[1/4] Deploy bundle not found locally — fetching from git on remote...",
        );
    }

    // ── Step 2: Run setup script on remote ──────────────────────────────────
    emit(
        &app,
        "[2/5] Setting up server (Docker, Firewall, Fail2ban)...",
    );

    // Generate a token locally so we know it before deploying
    let token = generate_secure_token();

    // Credentials travel over the SSH process stdin, never command arguments.
    let mut setup_flags = "--credentials-stdin".to_string();
    if enable_systemd.unwrap_or(false) {
        setup_flags.push_str(" --systemd");
    }

    let remote_cmds = if deploy_dir.is_some() {
        // Bundle was copied
        format!(
            "chmod +x /tmp/thinclaw-deploy/setup.sh && \
             /tmp/thinclaw-deploy/setup.sh {flags} 2>&1",
            flags = setup_flags
        )
    } else {
        // No local bundle — clone from GitHub + run setup
        format!(
            "apt-get update -q && \
             apt-get install -y docker.io docker-compose-plugin curl git -q && \
             systemctl start docker && \
             {{ git clone --depth=1 https://github.com/RNT56/ThinClaw.git /opt/thinclaw 2>&1 || \
             git -C /opt/thinclaw pull 2>&1; }} && \
             chmod +x /opt/thinclaw/deploy/setup.sh && \
             /opt/thinclaw/deploy/setup.sh {flags} 2>&1",
            flags = setup_flags
        )
    };

    let ssh_result = run_command_with_events(
        &app,
        "ssh",
        &[
            "-o".to_string(),
            "StrictHostKeyChecking=accept-new".to_string(),
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "--".to_string(),
            ssh_target.clone(),
            remote_cmds,
        ],
        Some(format!(
            "{}\n{}\n",
            token,
            tailscale_key
                .as_ref()
                .map(|key| key.as_str())
                .unwrap_or("")
        )),
        vec![
            token.clone(),
            tailscale_key
                .as_ref()
                .map(|key| key.to_string())
                .unwrap_or_default(),
        ],
    )
    .await;

    // ── Step 3: Verify ───────────────────────────────────────────────────────
    emit(&app, "[3/5] Verifying connectivity to new deployment...");

    let mut connection_note = None;
    let gateway_host = if tailscale_key.is_some() {
        match discover_tailscale_ip(&ssh_target).await {
            Ok(tailscale_ip) => tailscale_ip,
            Err(error) => {
                connection_note = Some(error);
                ip.clone()
            }
        }
    } else {
        ip.clone()
    };
    let gateway_url = format!(
        "http://{}:{}",
        url_host(&gateway_host),
        REMOTE_GATEWAY_PORT
    );
    let mut connected = false;

    // Give Docker a moment to start
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    for attempt in 1..=6 {
        emit(&app, &format!("[3/5] Connection attempt {}/6...", attempt));
        let proxy = match crate::thinclaw::remote_proxy::RemoteGatewayProxy::new(
            &gateway_url,
            &token,
        ) {
            Ok(proxy) => proxy,
            Err(error) => {
                connection_note = Some(error);
                break;
            }
        };
        match proxy.health_check().await {
            Ok(true) => {
                connected = true;
                break;
            }
            Ok(false) => {
                connection_note = Some("gateway rejected the generated credential".to_string());
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            Err(error) => {
                connection_note = Some(error);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    // ── Step 4–5: Report result ──────────────────────────────────────────────
    if connected {
        emit(&app, "[4/5] Deployment successful!");
        emit(&app, &format!("  URL:   {}", gateway_url));
        emit(
            &app,
            "The generated credential is ready in this deployment wizard; it was not written to deployment logs.",
        );

        Ok(RemoteDeployResult {
            status: "success".to_string(),
            url: gateway_url,
            token,
            message: None,
        })
    } else {
        let ssh_err = ssh_result.unwrap_err_or_default();
        let detail = connection_note.unwrap_or_else(|| "gateway did not become ready".to_string());
        let msg = format!(
            "Deployment may have started but health check timed out. \
             Try connecting manually to {} using the credential returned by this wizard. \
             Connection detail: {}. SSH error (if any): {}",
            gateway_url, detail, ssh_err
        );
        emit(&app, &format!("[4/5] Warning: {}", msg));
        Ok(RemoteDeployResult {
            status: "timeout".to_string(),
            url: gateway_url,
            token,
            message: Some(msg),
        })
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Find the ThinClaw deploy bundle directory.
///
/// Looks in:
///   1. Tauri resource_dir/deploy/ (production build)
///   2. nearest ancestor deploy/ directory (development)
fn find_deploy_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    // 1. Production bundle
    if let Ok(resource_dir) = app.path().resource_dir() {
        let prod_path = resource_dir.join("deploy");
        if prod_path.join("docker-compose.yml").exists() {
            return Some(prod_path);
        }
    }

    // 2. Development: walk ancestors until the ThinClaw root deploy/ is found.
    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            let path = ancestor.join("deploy");
            if path.join("docker-compose.yml").exists() {
                return Some(path);
            }
        }
    }

    None
}

/// Generate a cryptographically random hex token.
fn generate_secure_token() -> String {
    let bytes: [u8; 32] = rand::random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn validate_ssh_host(raw: &str) -> Result<String, String> {
    let host = raw.trim();
    if host.is_empty() || host.len() > 253 || host.starts_with('-') {
        return Err("SSH host must be a valid IP address or DNS hostname".to_string());
    }
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(host.to_string());
    }
    let valid = host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    });
    if !valid {
        return Err("SSH host must be a valid IP address or DNS hostname".to_string());
    }
    Ok(host.to_string())
}

fn validate_ssh_user(raw: &str) -> Result<String, String> {
    let user = raw.trim();
    let mut bytes = user.bytes();
    let valid_start = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    let valid_rest = bytes.all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
    });
    if user.len() > 32 || !valid_start || !valid_rest {
        return Err("SSH user contains unsupported characters".to_string());
    }
    Ok(user.to_string())
}

fn validate_tailscale_key(raw: &str) -> Result<String, String> {
    let key = raw.trim();
    if !key.starts_with("tskey-")
        || key.len() > 512
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Tailscale auth key has an invalid format".to_string());
    }
    Ok(key.to_string())
}

fn url_host(host: &str) -> String {
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

async fn discover_tailscale_ip(ssh_target: &str) -> Result<String, String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("ssh")
            .args([
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "BatchMode=yes",
                "--",
                ssh_target,
                "tailscale ip -4",
            ])
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| "timed out while discovering the deployed Tailscale address".to_string())?
    .map_err(|error| format!("failed to discover the deployed Tailscale address: {error}"))?;

    if !output.status.success() {
        return Err("the remote host did not report a Tailscale IPv4 address".to_string());
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let candidate = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| "the remote host reported an empty Tailscale address".to_string())?;
    let address = candidate
        .parse::<std::net::Ipv4Addr>()
        .map_err(|_| "the remote host reported an invalid Tailscale address".to_string())?;
    let octets = address.octets();
    if octets[0] != 100 || !(64..=127).contains(&octets[1]) {
        return Err("the remote host reported an address outside Tailscale CGNAT space".to_string());
    }
    Ok(address.to_string())
}

/// Trait extension to safely get the error from a Result.
trait ResultExt<E: std::fmt::Display> {
    fn unwrap_err_or_default(&self) -> String;
}

impl ResultExt<String> for Result<(), String> {
    fn unwrap_err_or_default(&self) -> String {
        match self {
            Err(e) => e.clone(),
            Ok(()) => String::new(),
        }
    }
}

/// Run a command, emitting stdout/stderr lines as `deploy-log` events.
/// Returns Ok(()) on exit code 0, Err(message) otherwise.
async fn run_command_with_events(
    app: &AppHandle,
    program: &str,
    args: &[String],
    stdin_payload: Option<String>,
    mut redactions: Vec<String>,
) -> Result<(), String> {
    let child_result = tokio::process::Command::new(program)
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
        )
        .spawn();
    let mut child = match child_result {
        Ok(child) => child,
        Err(error) => {
            redactions.zeroize();
            return Err(format!("Failed to run {program}: {error}"));
        }
    };

    if let Some(mut payload) = stdin_payload {
        let Some(mut stdin) = child.stdin.take() else {
            payload.zeroize();
            redactions.zeroize();
            return Err("Failed to open command stdin".to_string());
        };
        let write_result = stdin.write_all(payload.as_bytes()).await;
        payload.zeroize();
        if let Err(error) = write_result {
            redactions.zeroize();
            let _ = child.kill().await;
            return Err(format!("Failed to send deployment credentials: {error}"));
        }
        drop(stdin);
    }

    let Some(stdout) = child.stdout.take() else {
        redactions.zeroize();
        let _ = child.kill().await;
        return Err("Failed to capture stdout".to_string());
    };
    let Some(stderr) = child.stderr.take() else {
        redactions.zeroize();
        let _ = child.kill().await;
        return Err("Failed to capture stderr".to_string());
    };

    let app_out = app.clone();
    let app_err = app.clone();
    let mut stdout_redactions = redactions.clone();
    let mut stderr_redactions = redactions;

    let stdout_task = tokio::spawn(async move {
        stream_command_output(&app_out, stdout, "", &stdout_redactions).await;
        stdout_redactions.zeroize();
    });

    let stderr_task = tokio::spawn(async move {
        stream_command_output(&app_err, stderr, "[stderr] ", &stderr_redactions).await;
        stderr_redactions.zeroize();
    });

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Command '{}' failed to wait: {}", program, e))?;

    let _ = tokio::join!(stdout_task, stderr_task);

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Command '{}' exited with code {:?}",
            program,
            status.code()
        ))
    }
}

async fn stream_command_output<R: AsyncRead + Unpin>(
    app: &AppHandle,
    mut reader: R,
    prefix: &str,
    redactions: &[String],
) {
    let mut chunk = [0_u8; 8192];
    let mut line = Vec::new();
    let mut truncated = false;

    loop {
        let count = match reader.read(&mut chunk).await {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => {
                let _ = app.emit(
                    "deploy-log",
                    format!("{prefix}[output read failed: {error}]"),
                );
                return;
            }
        };

        for byte in &chunk[..count] {
            if *byte == b'\n' {
                emit_bounded_deploy_line(app, prefix, &line, truncated, redactions);
                line.clear();
                truncated = false;
            } else if line.len() < MAX_DEPLOY_LOG_LINE_BYTES {
                line.push(*byte);
            } else {
                truncated = true;
            }
        }
    }

    if !line.is_empty() || truncated {
        emit_bounded_deploy_line(app, prefix, &line, truncated, redactions);
    }
}

fn emit_bounded_deploy_line(
    app: &AppHandle,
    prefix: &str,
    bytes: &[u8],
    truncated: bool,
    redactions: &[String],
) {
    let mut line = String::from_utf8_lossy(bytes).into_owned();
    if truncated {
        line.push_str(" … [line truncated]");
    }
    let _ = app.emit(
        "deploy-log",
        format!("{prefix}{}", redact_line(line, redactions)),
    );
}

fn redact_line(mut line: String, redactions: &[String]) -> String {
    for secret in redactions.iter().filter(|secret| !secret.is_empty()) {
        line = line.replace(secret, "[REDACTED]");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::{redact_line, url_host, validate_ssh_host, validate_ssh_user, validate_tailscale_key};

    #[test]
    fn deployment_targets_reject_shell_and_option_injection() {
        assert_eq!(validate_ssh_host("agent.tailnet.ts.net").unwrap(), "agent.tailnet.ts.net");
        assert!(validate_ssh_host("-oProxyCommand=evil").is_err());
        assert!(validate_ssh_host("host; touch /tmp/pwned").is_err());
        assert_eq!(validate_ssh_user("deploy-user").unwrap(), "deploy-user");
        assert!(validate_ssh_user("root;id").is_err());
    }

    #[test]
    fn tailscale_keys_are_validated_and_log_lines_are_redacted() {
        let secret = "tskey-auth-private_123";
        assert_eq!(validate_tailscale_key(secret).unwrap(), secret);
        assert!(validate_tailscale_key("$(touch /tmp/pwned)").is_err());
        assert_eq!(
            redact_line(format!("credential={secret}"), &[secret.to_string()]),
            "credential=[REDACTED]"
        );
    }

    #[test]
    fn ipv6_hosts_are_bracketed_for_gateway_urls() {
        assert_eq!(url_host("fd00::1"), "[fd00::1]");
        assert_eq!(url_host("192.168.1.20"), "192.168.1.20");
    }
}
