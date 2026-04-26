//! Gateway management CLI commands.
//!
//! Subcommands:
//! - `gateway start` — start the web gateway (foreground or background)
//! - `gateway stop` — stop a running gateway
//! - `gateway status` — show gateway status
//! - `gateway access` — print WebUI access URLs and SSH tunnel guidance

use clap::Subcommand;
use sysinfo::{Pid, ProcessesToUpdate, Signal, System};

use crate::platform::gateway_access::GatewayAccessInfo;
use crate::settings::Settings;
use crate::terminal_branding::TerminalBranding;

#[derive(Subcommand, Debug, Clone)]
pub enum GatewayCommand {
    /// Start the web gateway
    Start {
        /// Port to listen on (default: from GATEWAY_PORT env or 3000)
        #[arg(short, long)]
        port: Option<u16>,

        /// Host to bind to (default: from GATEWAY_HOST env or "127.0.0.1")
        #[arg(long)]
        host: Option<String>,

        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },

    /// Stop a running gateway
    Stop,

    /// Restart or refresh the managed gateway process
    Reload {
        /// Port to listen on (default: from GATEWAY_PORT env or 3000)
        #[arg(short, long)]
        port: Option<u16>,

        /// Host to bind to (default: from GATEWAY_HOST env or "127.0.0.1")
        #[arg(long)]
        host: Option<String>,

        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },

    /// Show gateway status
    Status,

    /// Print WebUI access URLs, token status, and SSH tunnel guidance
    Access {
        /// Show the full token URL instead of redacting the token.
        #[arg(long)]
        show_token: bool,
    },
}

/// Run a gateway command.
pub async fn run_gateway_command(cmd: GatewayCommand) -> anyhow::Result<()> {
    match cmd {
        GatewayCommand::Start {
            port,
            host,
            foreground,
        } => {
            TerminalBranding::current().print_banner("Gateway", Some("Start the web cockpit"));
            start_gateway(port, host, foreground).await
        }
        GatewayCommand::Stop => {
            TerminalBranding::current().print_banner("Gateway", Some("Stop the web cockpit"));
            stop_gateway().await
        }
        GatewayCommand::Reload {
            port,
            host,
            foreground,
        } => {
            TerminalBranding::current().print_banner("Gateway", Some("Reload the web cockpit"));
            reload_gateway(port, host, foreground).await
        }
        GatewayCommand::Status => {
            TerminalBranding::current().print_banner("Gateway", Some("Inspect the web cockpit"));
            gateway_status().await
        }
        GatewayCommand::Access { show_token } => {
            TerminalBranding::current().print_banner("Gateway", Some("WebUI access"));
            gateway_access(show_token).await
        }
    }
}

/// PID file location.
fn pid_file_path() -> std::path::PathBuf {
    crate::platform::state_paths().gateway_pid_file
}

fn pid_is_running(pid: u32) -> bool {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, false);
    system.process(Pid::from_u32(pid)).is_some()
}

fn terminate_pid(pid: u32) -> bool {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, false);
    if let Some(process) = system.process(Pid::from_u32(pid)) {
        return process
            .kill_with(Signal::Term)
            .unwrap_or_else(|| process.kill());
    }
    false
}

/// Start the gateway.
async fn start_gateway(
    port: Option<u16>,
    host: Option<String>,
    foreground: bool,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let pid_path = pid_file_path();

    // Check if already running.
    if pid_path.exists()
        && let Ok(contents) = std::fs::read_to_string(&pid_path)
        && let Ok(pid) = contents.trim().parse::<u32>()
    {
        // Check if process is alive.
        let alive = pid_is_running(pid);

        if alive {
            anyhow::bail!(
                "Gateway is already running (PID {}). Stop it first with: thinclaw gateway stop",
                pid
            );
        } else {
            // Stale PID file.
            let _ = std::fs::remove_file(&pid_path);
        }
    }

    // Resolve host and port from args, env, or defaults.
    let gw_host = host.unwrap_or_else(|| {
        std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
    });
    let gw_port = port.unwrap_or_else(|| {
        std::env::var("GATEWAY_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000)
    });

    if foreground {
        println!(
            "{}",
            branding.accent(format!(
                "Bringing gateway online at {}:{}...",
                gw_host, gw_port
            ))
        );
        println!("  {}", branding.muted("Press Ctrl+C to stop."));
        println!();

        let exe = std::env::current_exe()?;
        let mut child = std::process::Command::new(&exe)
            .arg("run")
            .env("GATEWAY_ENABLED", "true")
            .env("GATEWAY_HOST", &gw_host)
            .env("GATEWAY_PORT", gw_port.to_string())
            .env("CLI_ENABLED", "false")
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .spawn()?;

        // Write PID file.
        let pid = child.id();
        if let Some(parent) = pid_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&pid_path, pid.to_string())?;
        println!(
            "  {}",
            branding.muted(format!("Gateway process running (PID {}).", pid))
        );
        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("Gateway process exited with status {}", status);
        }

        // Clean up PID file.
        let _ = std::fs::remove_file(&pid_path);
    } else {
        // Background: spawn `thinclaw run` as a detached child process.
        let exe = std::env::current_exe()?;

        let child = std::process::Command::new(&exe)
            .arg("run")
            .env("GATEWAY_ENABLED", "true")
            .env("GATEWAY_HOST", &gw_host)
            .env("GATEWAY_PORT", gw_port.to_string())
            .env("CLI_ENABLED", "false")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;

        let pid = child.id();

        // Write PID file.
        if let Some(parent) = pid_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&pid_path, pid.to_string())?;

        println!(
            "  {}",
            branding.good(format!(
                "Gateway online at {}:{} (PID {})",
                gw_host, gw_port, pid
            ))
        );
        println!("  {}", branding.muted("Stop with: thinclaw gateway stop"));
    }

    Ok(())
}

/// Print gateway access guidance.
async fn gateway_access(show_token: bool) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let settings = Settings::load();
    let access = GatewayAccessInfo::from_env_and_settings(Some(&settings));
    let health_ok = gateway_health_ok(&access).await.unwrap_or(false);

    println!("{}", branding.key_value("Enabled", access.enabled));
    println!("{}", branding.key_value("Bind", access.bind_display()));
    println!("{}", branding.key_value("Port", access.port));
    println!("{}", branding.key_value("Auth", access.auth_status()));
    println!("{}", branding.key_value("Local URL", access.local_url()));
    if let Some(url) = access.token_url(show_token) {
        println!("{}", branding.key_value("Token URL", url));
    }
    println!(
        "{}",
        branding.key_value("SSH tunnel", access.ssh_tunnel_command())
    );
    println!(
        "{}",
        branding.key_value(
            "Health",
            if health_ok {
                branding.good("reachable")
            } else {
                branding.warn("not reachable")
            },
        )
    );

    for warning in access.service_warnings() {
        println!("{}", branding.warn(format!("Warning: {warning}")));
    }
    if !show_token && access.auth_token.is_some() {
        println!(
            "{}",
            branding
                .muted("Use `thinclaw gateway access --show-token` to print the full token URL.")
        );
    }

    Ok(())
}

async fn gateway_health_ok(access: &GatewayAccessInfo) -> anyhow::Result<bool> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    Ok(client
        .get(access.health_url())
        .send()
        .await
        .map(|response| response.status().is_success())
        .unwrap_or(false))
}

async fn reload_gateway(
    port: Option<u16>,
    host: Option<String>,
    foreground: bool,
) -> anyhow::Result<()> {
    let pid_path = pid_file_path();
    if pid_path.exists() {
        let _ = stop_gateway().await;
    }
    start_gateway(port, host, foreground).await
}

/// Stop a running gateway.
async fn stop_gateway() -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let pid_path = pid_file_path();

    if !pid_path.exists() {
        println!("{}", branding.warn("No gateway PID file found."));
        println!("{}", branding.key_value("PID file", pid_path.display()));
        return Ok(());
    }

    let contents = std::fs::read_to_string(&pid_path)?;
    let pid: u32 = contents
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid PID in {}", pid_path.display()))?;

    if terminate_pid(pid) {
        println!(
            "  {}",
            branding.good(format!("Stopped gateway process (PID {})", pid))
        );
    } else {
        println!(
            "  {}",
            branding.warn(format!(
                "Failed to stop PID {} (process may have already exited)",
                pid
            ))
        );
    }

    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

/// Show gateway status.
async fn gateway_status() -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let pid_path = pid_file_path();

    // Check PID file.
    let pid_info = if pid_path.exists()
        && let Ok(contents) = std::fs::read_to_string(&pid_path)
        && let Ok(pid) = contents.trim().parse::<u32>()
    {
        let alive = pid_is_running(pid);

        if alive {
            Some((pid, true))
        } else {
            Some((pid, false))
        }
    } else {
        None
    };

    // Try to reach the health endpoint.
    let gw_host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let gw_port = std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "3000".to_string());
    let health_url = format!("http://{}:{}/api/health", gw_host, gw_port);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    let health_ok = client
        .get(&health_url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    println!(
        "{}",
        branding.key_value("Endpoint", format!("{}:{}", gw_host, gw_port))
    );

    match pid_info {
        Some((pid, true)) => {
            println!("{}", branding.key_value("PID", format!("{pid} (running)")));
        }
        Some((pid, false)) => {
            println!(
                "{}",
                branding.key_value("PID", format!("{pid} (stale, process not found)"))
            );
        }
        None => {
            println!("{}", branding.key_value("PID", "not tracked"));
        }
    }

    if health_ok {
        println!(
            "{}",
            branding.key_value("Health", branding.good("reachable"))
        );

        // Try to get detailed status.
        if let Ok(resp) = client
            .get(format!("http://{}:{}/api/gateway/status", gw_host, gw_port))
            .send()
            .await
            && let Ok(json) = resp.json::<serde_json::Value>().await
        {
            if let Some(uptime) = json.get("uptime_secs").and_then(|v| v.as_u64()) {
                let hours = uptime / 3600;
                let mins = (uptime % 3600) / 60;
                println!(
                    "{}",
                    branding.key_value("Uptime", format!("{hours}h {mins}m"))
                );
            }
            if let Some(conns) = json.get("total_connections").and_then(|v| v.as_u64()) {
                println!("{}", branding.key_value("Clients", conns));
            }
        }
    } else {
        println!(
            "{}",
            branding.key_value("Health", branding.bad("not reachable"))
        );
    }

    Ok(())
}
