//! Remote deployment command — SSH + Docker Compose approach.
//!
//! Previous implementation relied on an Ansible/shell script that wasn't
//! bundled. This version is fully self-contained: it uses `ssh` and `scp`
//! (available on all major platforms) to deploy a Docker Compose stack
//! to any Linux server.
//!
//! ## Deploy flow:
//!   1. SCP the `ironclaw/deploy/` bundle to the target server
//!   2. Generate a secure auth token on the server if one isn't set
//!   3. Run `docker compose up -d --build` via SSH
//!   4. Emit `deploy-log` events to the frontend for live progress
//!   5. Emit `deploy-status` with `{ success: true, url: "...", token: "..." }`
//!
//! ## Connect flow (new):
//!   - Simply test connectivity and return the URL + token to the frontend.
//!   - The frontend calls `openclaw_switch_to_profile` to activate the connection.

use std::process::Stdio;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, BufReader};

/// Deploy the IronClaw remote agent to a Linux server via SSH + Docker Compose.
///
/// Accepts the SSH host, user, and optional configuration for Tailscale VPN
/// and systemd service. Emits live `deploy-log` events and a final
/// `deploy-status` event with `"success"` | `"failed:<reason>"`.
///
/// Steps:
///   1. Find the `ironclaw/deploy/` directory (bundled or source)
///   2. SCP the deploy bundle to the target server
///   3. Run `setup.sh` on the server:
///      - Always: Docker, UFW firewall, Fail2ban
///      - Optional: Tailscale VPN (--tailscale <key>)
///      - Optional: systemd service (--systemd)
///   4. Return the URL + generated token via `deploy-status`
#[tauri::command]
#[specta::specta]
pub async fn openclaw_deploy_remote(
    app: AppHandle,
    _state: State<'_, super::commands::OpenClawManager>,
    ip: String,
    user: String,
    tailscale_key: Option<String>,
    enable_systemd: Option<bool>,
) -> Result<(), String> {
    // ── Validate input ────────────────────────────────────────────────────────
    if ip.trim().is_empty() {
        return Err("IP address is required".to_string());
    }
    let user = if user.trim().is_empty() {
        "root".to_string()
    } else {
        user
    };

    // ── Locate deploy bundle ──────────────────────────────────────────────────
    //
    // In production builds: bundled with the app as `resources/deploy/`
    // In dev builds: `ironclaw/deploy/` relative to workspace root
    let deploy_dir = find_deploy_dir(&app);

    let emit = |app_handle: &AppHandle, msg: &str| {
        let _ = app_handle.emit("deploy-log", msg);
    };

    emit(&app, "=== IronClaw Remote Deploy ===");
    emit(&app, &format!("Target: {}@{}", user, ip));

    // ── Step 1: Copy deploy bundle via SCP ───────────────────────────────────
    emit(&app, "[1/4] Copying deploy bundle to remote server...");

    if let Some(ref dir) = deploy_dir {
        let scp_result = run_command_with_events(
            &app,
            "scp",
            &[
                "-o".to_string(),
                "StrictHostKeyChecking=no".to_string(),
                "-r".to_string(),
                dir.to_string_lossy().to_string(),
                format!("{}@{}:/tmp/ironclaw-deploy", user, ip),
            ],
        )
        .await;

        if let Err(e) = scp_result {
            let msg = format!("SCP failed: {}. Is SSH access configured?", e);
            let _ = app.emit("deploy-status", format!("failed: {}", msg));
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

    // Build setup.sh flags
    let mut setup_flags = format!("--token {}", token);
    if let Some(ref ts_key) = tailscale_key {
        if !ts_key.is_empty() {
            setup_flags.push_str(&format!(" --tailscale {}", ts_key));
        }
    }
    if enable_systemd.unwrap_or(false) {
        setup_flags.push_str(" --systemd");
    }

    let remote_cmds = if deploy_dir.is_some() {
        // Bundle was copied
        format!(
            "chmod +x /tmp/ironclaw-deploy/setup.sh && \
             /tmp/ironclaw-deploy/setup.sh {flags} 2>&1",
            flags = setup_flags
        )
    } else {
        // No local bundle — clone from GitHub + run setup
        format!(
            "apt-get update -q && \
             apt-get install -y docker.io docker-compose-plugin curl git -q && \
             systemctl start docker && \
             git clone --depth=1 https://github.com/RNT56/ThinClaw.git /opt/ironclaw 2>&1 || \
             git -C /opt/ironclaw pull 2>&1 && \
             chmod +x /opt/ironclaw/deploy/setup.sh && \
             /opt/ironclaw/deploy/setup.sh {flags} 2>&1",
            flags = setup_flags
        )
    };

    let ssh_result = run_command_with_events(
        &app,
        "ssh",
        &[
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            format!("{}@{}", user, ip),
            remote_cmds,
        ],
    )
    .await;

    // ── Step 3: Verify ───────────────────────────────────────────────────────
    emit(&app, "[3/5] Verifying connectivity to new deployment...");

    let gateway_url = format!("http://{}:{}", ip, 18789);
    let mut connected = false;

    // Give Docker a moment to start
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    for attempt in 1..=6 {
        emit(&app, &format!("[3/5] Connection attempt {}/6...", attempt));
        let proxy = crate::openclaw::remote_proxy::RemoteGatewayProxy::new(&gateway_url, &token);
        match proxy.health_check().await {
            Ok(true) => {
                connected = true;
                break;
            }
            _ => {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    // ── Step 4–5: Report result ──────────────────────────────────────────────
    if connected {
        emit(&app, "[4/5] Deployment successful!");
        emit(&app, &format!("  URL:   {}", gateway_url));
        emit(&app, &format!("  Token: {}", token));
        emit(&app, "");
        emit(
            &app,
            "Connect Scrappy to this agent via Gateway Settings > Add Remote Agent",
        );

        // Emit structured success event for the frontend wizard
        let _ = app.emit(
            "deploy-status",
            serde_json::json!({
                "status": "success",
                "url": gateway_url,
                "token": token,
            })
            .to_string(),
        );
    } else {
        let ssh_err = ssh_result.unwrap_err_or_default();
        let msg = format!(
            "Deployment may have started but health check timed out. \
             Try connecting manually to {} with token: {}. \
             SSH error (if any): {}",
            gateway_url, token, ssh_err
        );
        emit(&app, &format!("[4/5] Warning: {}", msg));
        let _ = app.emit(
            "deploy-status",
            serde_json::json!({
                "status": "timeout",
                "url": gateway_url,
                "token": token,
                "message": msg,
            })
            .to_string(),
        );
    }

    Ok(())
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Find the IronClaw deploy bundle directory.
///
/// Looks in:
///   1. Tauri resource_dir/deploy/ (production build)
///   2. workspace ../ironclaw/deploy/ (development)
fn find_deploy_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    // 1. Production bundle
    if let Ok(resource_dir) = app.path().resource_dir() {
        let prod_path = resource_dir.join("deploy");
        if prod_path.join("docker-compose.yml").exists() {
            return Some(prod_path);
        }
    }

    // 2. Development: workspace root / ironclaw / deploy
    if let Ok(cwd) = std::env::current_dir() {
        let dev_path = cwd
            .parent()
            .and_then(|p| Some(p.join("ironclaw").join("deploy")))
            .or_else(|| Some(cwd.join("..").join("ironclaw").join("deploy")));
        if let Some(path) = dev_path {
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
) -> Result<(), String> {
    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env(
            "PATH",
            format!(
                "{}:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .spawn()
        .map_err(|e| format!("Failed to run {}: {}", program, e))?;

    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;
    let stderr = child.stderr.take().ok_or("Failed to capture stderr")?;

    let app_out = app.clone();
    let app_err = app.clone();

    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = app_out.emit("deploy-log", line);
        }
    });

    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = app_err.emit("deploy-log", format!("[stderr] {}", line));
        }
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
