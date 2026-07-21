//! Gateway management CLI commands.
//!
//! Subcommands:
//! - `gateway start` — start the web gateway (foreground or background)
//! - `gateway stop` — stop a running gateway
//! - `gateway status` — show gateway status
//! - `gateway access` — print WebUI access URLs and SSH tunnel guidance

use clap::Subcommand;
use fs4::{FileExt, TryLockError};
use sysinfo::{Pid, ProcessesToUpdate, Signal, System};

use crate::platform::gateway_access::GatewayAccessInfo;
use crate::settings::Settings;
use crate::terminal_branding::TerminalBranding;

const GATEWAY_PID_RECORD_VERSION: u8 = 1;
const MAX_GATEWAY_PID_RECORD_BYTES: u64 = 4 * 1024;
const GATEWAY_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

struct GatewayOperationLock(std::fs::File);

impl GatewayOperationLock {
    async fn acquire() -> anyhow::Result<Self> {
        let lock_path = pid_file_path().with_extension("pid.lock");
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            options.custom_flags(
                windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT,
            );
        }
        let file = options.open(&lock_path)?;
        if !file.metadata()?.is_file() {
            anyhow::bail!("gateway operation lock is not a regular file");
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match FileExt::try_lock(&file) {
                Ok(()) => return Ok(Self(file)),
                Err(TryLockError::WouldBlock) => {
                    if tokio::time::Instant::now() >= deadline {
                        anyhow::bail!("timed out waiting for another gateway operation");
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }
}

impl Drop for GatewayOperationLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct GatewayPidRecord {
    version: u8,
    pid: u32,
    start_time: u64,
    instance_token: String,
}

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
            let operation_lock = GatewayOperationLock::acquire().await?;
            TerminalBranding::current().print_banner("Gateway", Some("Start the web cockpit"));
            start_gateway(port, host, foreground, operation_lock).await
        }
        GatewayCommand::Stop => {
            let _operation_lock = GatewayOperationLock::acquire().await?;
            TerminalBranding::current().print_banner("Gateway", Some("Stop the web cockpit"));
            stop_gateway().await
        }
        GatewayCommand::Reload {
            port,
            host,
            foreground,
        } => {
            let operation_lock = GatewayOperationLock::acquire().await?;
            TerminalBranding::current().print_banner("Gateway", Some("Reload the web cockpit"));
            reload_gateway(port, host, foreground, operation_lock).await
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

fn process_for_pid(pid: u32) -> Option<(System, Pid)> {
    let mut system = System::new_all();
    system.refresh_processes(ProcessesToUpdate::All, false);
    let system_pid = Pid::from_u32(pid);
    system.process(system_pid)?;
    Some((system, system_pid))
}

fn record_matches_process(record: &GatewayPidRecord) -> bool {
    if record.version != GATEWAY_PID_RECORD_VERSION
        || record.instance_token.is_empty()
        || record.instance_token.len() > 128
    {
        return false;
    }
    let Some((system, pid)) = process_for_pid(record.pid) else {
        return false;
    };
    let Some(process) = system.process(pid) else {
        return false;
    };
    if process.start_time() != record.start_time {
        return false;
    }
    let Ok(current_exe) = std::env::current_exe().and_then(std::fs::canonicalize) else {
        return false;
    };
    let executable_matches = process
        .exe()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .is_some_and(|path| path == current_exe);
    let command_matches = process
        .cmd()
        .iter()
        .any(|argument| argument == std::ffi::OsStr::new("run"));
    executable_matches && command_matches
}

async fn terminate_gateway_record(record: &GatewayPidRecord) -> bool {
    if !record_matches_process(record) {
        return false;
    }
    let Some((system, pid)) = process_for_pid(record.pid) else {
        return false;
    };
    let signalled = if let Some(process) = system.process(pid) {
        process
            .kill_with(Signal::Term)
            .unwrap_or_else(|| process.kill())
    } else {
        false
    };
    if !signalled {
        return false;
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        if !record_matches_process(record) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    if let Some((system, pid)) = process_for_pid(record.pid)
        && let Some(process) = system.process(pid)
        && process.start_time() == record.start_time
    {
        let _ = process.kill();
    }
    let hard_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < hard_deadline {
        if !record_matches_process(record) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    false
}

fn read_gateway_pid_record(path: &std::path::Path) -> anyhow::Result<Option<GatewayPidRecord>> {
    let bytes =
        match thinclaw_platform::read_regular_file_bounded(path, MAX_GATEWAY_PID_RECORD_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "cannot safely read gateway PID record {}: {error}",
                    path.display()
                ));
            }
        };
    let record = serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::anyhow!(
            "gateway PID record {} is malformed: {error}",
            path.display()
        )
    })?;
    Ok(Some(record))
}

fn write_gateway_pid_record(
    path: &std::path::Path,
    record: &GatewayPidRecord,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(record)?;
    thinclaw_platform::write_private_file_atomic(path, &bytes, true)?;
    Ok(())
}

async fn record_for_spawned_process(
    pid: u32,
    instance_token: String,
) -> anyhow::Result<GatewayPidRecord> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        if let Some((system, system_pid)) = process_for_pid(pid)
            && let Some(process) = system.process(system_pid)
        {
            return Ok(GatewayPidRecord {
                version: GATEWAY_PID_RECORD_VERSION,
                pid,
                start_time: process.start_time(),
                instance_token,
            });
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("gateway process exited before its PID record could be created");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

fn remove_gateway_pid_record_if_current(
    path: &std::path::Path,
    instance_token: &str,
) -> anyhow::Result<()> {
    if read_gateway_pid_record(path)?
        .as_ref()
        .is_some_and(|record| record.instance_token == instance_token)
    {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn validate_gateway_host(host: &str) -> anyhow::Result<()> {
    if host.is_empty()
        || host.len() > 253
        || host.trim() != host
        || !host.is_ascii()
        || host.chars().any(char::is_control)
    {
        anyhow::bail!("gateway host is empty, oversized, or malformed");
    }
    if host.parse::<std::net::IpAddr>().is_ok() || host.eq_ignore_ascii_case("localhost") {
        return Ok(());
    }
    if host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    }) {
        return Ok(());
    }
    anyhow::bail!("gateway host is not an IP address or valid DNS name")
}

fn gateway_url_host(host: &str) -> String {
    if matches!(host, "0.0.0.0" | "::") {
        return "127.0.0.1".to_string();
    }
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

async fn wait_for_gateway_ready(
    host: &str,
    port: u16,
    record: &GatewayPidRecord,
) -> anyhow::Result<()> {
    let request_host = gateway_url_host(host);
    let url = format!("http://{request_host}:{port}/api/health");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1))
        .connect_timeout(std::time::Duration::from_millis(500))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?;
    let deadline = tokio::time::Instant::now() + GATEWAY_READY_TIMEOUT;
    loop {
        if !record_matches_process(record) {
            anyhow::bail!("gateway process exited before becoming ready");
        }
        if client
            .get(&url)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!(
                "gateway did not become ready at {request_host}:{port} within {} seconds",
                GATEWAY_READY_TIMEOUT.as_secs()
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Start the gateway.
async fn start_gateway(
    port: Option<u16>,
    host: Option<String>,
    foreground: bool,
    operation_lock: GatewayOperationLock,
) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let pid_path = pid_file_path();

    // Check if the exact process recorded by ThinClaw is still running. PID
    // reuse alone is never enough authority to signal a process.
    if let Some(record) = read_gateway_pid_record(&pid_path)? {
        if record_matches_process(&record) {
            anyhow::bail!(
                "Gateway is already running (PID {}). Stop it first with: thinclaw gateway stop",
                record.pid
            );
        } else {
            std::fs::remove_file(&pid_path)?;
        }
    }

    // Resolve host and port from args, env, or defaults.
    let gw_host = host.unwrap_or_else(|| {
        std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
    });
    validate_gateway_host(&gw_host)?;
    let gw_port = match (port, std::env::var("GATEWAY_PORT").ok()) {
        (Some(port), _) => port,
        (None, Some(port)) => port
            .parse::<u16>()
            .map_err(|_| anyhow::anyhow!("GATEWAY_PORT must be a valid non-zero TCP port"))?,
        (None, None) => 3000,
    };
    if gw_port == 0 {
        anyhow::bail!("gateway port must be non-zero");
    }

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
        let instance_token = uuid::Uuid::new_v4().to_string();
        let mut command = std::process::Command::new(&exe);
        command
            .arg("run")
            .env("GATEWAY_ENABLED", "true")
            .env("GATEWAY_HOST", &gw_host)
            .env("GATEWAY_PORT", gw_port.to_string())
            .env("CLI_ENABLED", "false")
            .env("THINCLAW_GATEWAY_INSTANCE_TOKEN", &instance_token)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let mut child = thinclaw_platform::OwnedStdChild::spawn(&mut command)?;

        // Write PID file.
        let pid = child.id();
        if let Some(parent) = pid_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let record = match record_for_spawned_process(pid, instance_token.clone()).await {
            Ok(record) => record,
            Err(error) => {
                let _ = child.kill();
                return Err(error);
            }
        };
        if let Err(error) = write_gateway_pid_record(&pid_path, &record) {
            let _ = child.kill();
            return Err(error);
        }
        drop(operation_lock);
        println!(
            "  {}",
            branding.muted(format!("Gateway process running (PID {}).", pid))
        );
        let status = child.wait()?;
        let _cleanup_lock = GatewayOperationLock::acquire().await?;
        remove_gateway_pid_record_if_current(&pid_path, &instance_token)?;
        if !status.success() {
            anyhow::bail!("Gateway process exited with status {}", status);
        }
    } else {
        // Background: spawn `thinclaw run` as a detached child process.
        let exe = std::env::current_exe()?;

        let instance_token = uuid::Uuid::new_v4().to_string();
        let mut command = std::process::Command::new(&exe);
        command
            .arg("run")
            .env("GATEWAY_ENABLED", "true")
            .env("GATEWAY_HOST", &gw_host)
            .env("GATEWAY_PORT", gw_port.to_string())
            .env("CLI_ENABLED", "false")
            .env("THINCLAW_GATEWAY_INSTANCE_TOKEN", &instance_token)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            use windows_sys::Win32::System::Threading::{
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, DETACHED_PROCESS,
            };
            command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | DETACHED_PROCESS);
        }
        let mut child = command.spawn()?;

        let pid = child.id();

        // Write PID file.
        if let Some(parent) = pid_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let record = match record_for_spawned_process(pid, instance_token).await {
            Ok(record) => record,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        if let Err(error) = write_gateway_pid_record(&pid_path, &record) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }

        if let Err(error) = wait_for_gateway_ready(&gw_host, gw_port, &record).await {
            let _ = terminate_gateway_record(&record).await;
            let _ = child.wait();
            let _ = remove_gateway_pid_record_if_current(&pid_path, &record.instance_token);
            return Err(error);
        }
        drop(operation_lock);

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
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
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
    operation_lock: GatewayOperationLock,
) -> anyhow::Result<()> {
    let pid_path = pid_file_path();
    if read_gateway_pid_record(&pid_path)?.is_some() {
        stop_gateway().await?;
    }
    start_gateway(port, host, foreground, operation_lock).await
}

/// Stop a running gateway.
async fn stop_gateway() -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let pid_path = pid_file_path();

    let Some(record) = read_gateway_pid_record(&pid_path)? else {
        println!("{}", branding.warn("No gateway PID file found."));
        println!("{}", branding.key_value("PID file", pid_path.display()));
        return Ok(());
    };

    if terminate_gateway_record(&record).await {
        println!(
            "  {}",
            branding.good(format!("Stopped gateway process (PID {})", record.pid))
        );
    } else {
        anyhow::bail!(
            "refusing to signal PID {} because it no longer matches the recorded ThinClaw gateway process",
            record.pid
        );
    }

    remove_gateway_pid_record_if_current(&pid_path, &record.instance_token)?;
    Ok(())
}

/// Show gateway status.
async fn gateway_status() -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let pid_path = pid_file_path();

    // Check PID file.
    let pid_info = read_gateway_pid_record(&pid_path)?
        .map(|record| (record.pid, record_matches_process(&record)));

    // Try to reach the health endpoint.
    let gw_host = std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    validate_gateway_host(&gw_host)?;
    let gw_port = std::env::var("GATEWAY_PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("GATEWAY_PORT must be a valid TCP port"))?;
    if gw_port == 0 {
        anyhow::bail!("gateway port must be non-zero");
    }
    let url_host = gateway_url_host(&gw_host);
    let health_url = format!("http://{url_host}:{gw_port}/api/health");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?;

    let health_ok = client
        .get(&health_url)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    println!(
        "{}",
        branding.key_value("Endpoint", format!("{url_host}:{gw_port}"))
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
            .get(format!("http://{url_host}:{gw_port}/api/gateway/status"))
            .send()
            .await
            && resp.status().is_success()
            && let Ok(json) =
                thinclaw_types::http_response::bounded_json::<serde_json::Value>(resp, 1024 * 1024)
                    .await
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
