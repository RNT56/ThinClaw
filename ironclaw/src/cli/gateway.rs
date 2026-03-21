//! Gateway management CLI commands.
//!
//! Subcommands:
//! - `gateway start` — start the web gateway (foreground or background)
//! - `gateway stop` — stop a running gateway
//! - `gateway status` — show gateway status

use clap::Subcommand;

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

    /// Show gateway status
    Status,
}

/// Run a gateway command.
pub async fn run_gateway_command(cmd: GatewayCommand) -> anyhow::Result<()> {
    match cmd {
        GatewayCommand::Start {
            port,
            host,
            foreground,
        } => start_gateway(port, host, foreground).await,
        GatewayCommand::Stop => stop_gateway().await,
        GatewayCommand::Status => gateway_status().await,
    }
}

/// PID file location.
fn pid_file_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(home)
        .join(".thinclaw")
        .join("gateway.pid")
}

/// Start the gateway.
async fn start_gateway(
    port: Option<u16>,
    host: Option<String>,
    foreground: bool,
) -> anyhow::Result<()> {
    let pid_path = pid_file_path();

    // Check if already running.
    if pid_path.exists()
        && let Ok(contents) = std::fs::read_to_string(&pid_path)
        && let Ok(pid) = contents.trim().parse::<u32>()
    {
        // Check if process is alive.
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if alive {
            anyhow::bail!(
                "Gateway is already running (PID {}). Stop it first with: ironclaw gateway stop",
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
        println!("🌐 Starting gateway on {}:{}...", gw_host, gw_port);
        println!("   Press Ctrl+C to stop.");
        println!();

        // Set env vars so the agent picks them up.
        // SAFETY: We are single-threaded at this point (before any agent tasks start).
        unsafe {
            std::env::set_var("GATEWAY_HOST", &gw_host);
            std::env::set_var("GATEWAY_PORT", gw_port.to_string());
            std::env::set_var("GATEWAY_ENABLED", "true");
        }

        // Write PID file.
        let pid = std::process::id();
        if let Some(parent) = pid_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&pid_path, pid.to_string())?;

        // The actual gateway runs as part of `thinclaw run`. In foreground mode,
        // we tell the user to use `thinclaw run` with gateway enabled.
        println!(
            "Note: The gateway starts as part of the agent. Run:\n\n  \
             GATEWAY_ENABLED=true GATEWAY_HOST={} GATEWAY_PORT={} thinclaw run\n",
            gw_host, gw_port
        );

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
            "🌐 Gateway started on {}:{} (PID {})",
            gw_host, gw_port, pid
        );
        println!("   Stop with: ironclaw gateway stop");
    }

    Ok(())
}

/// Stop a running gateway.
async fn stop_gateway() -> anyhow::Result<()> {
    let pid_path = pid_file_path();

    if !pid_path.exists() {
        println!("No gateway PID file found. The gateway may not be running.");
        println!("   PID file: {}", pid_path.display());
        return Ok(());
    }

    let contents = std::fs::read_to_string(&pid_path)?;
    let pid: u32 = contents
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid PID in {}", pid_path.display()))?;

    // Send SIGTERM.
    let status = std::process::Command::new("kill")
        .args([&pid.to_string()])
        .status()?;

    if status.success() {
        println!("✅ Sent SIGTERM to gateway (PID {})", pid);
    } else {
        println!(
            "⚠️  Failed to send signal to PID {} (process may have already exited)",
            pid
        );
    }

    let _ = std::fs::remove_file(&pid_path);
    Ok(())
}

/// Show gateway status.
async fn gateway_status() -> anyhow::Result<()> {
    let pid_path = pid_file_path();

    // Check PID file.
    let pid_info = if pid_path.exists()
        && let Ok(contents) = std::fs::read_to_string(&pid_path)
        && let Ok(pid) = contents.trim().parse::<u32>()
    {
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

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

    println!("Gateway Status:");
    println!("  Endpoint:  {}:{}", gw_host, gw_port);

    match pid_info {
        Some((pid, true)) => {
            println!("  PID:       {} (running)", pid);
        }
        Some((pid, false)) => {
            println!("  PID:       {} (stale — process not found)", pid);
        }
        None => {
            println!("  PID:       not tracked");
        }
    }

    if health_ok {
        println!("  Health:    ✅ reachable");

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
                println!("  Uptime:    {}h {}m", hours, mins);
            }
            if let Some(conns) = json.get("total_connections").and_then(|v| v.as_u64()) {
                println!("  Clients:   {}", conns);
            }
        }
    } else {
        println!("  Health:    ❌ not reachable");
    }

    Ok(())
}
