//! OS service management for running ThinClaw as a daemon.
//!
//! Generates and manages platform-native service definitions:
//! - **macOS**: launchd plist at `~/Library/LaunchAgents/com.thinclaw.daemon.plist`
//! - **Linux**: systemd user unit at `~/.config/systemd/user/thinclaw.service`
//! - **Windows**: Service Control Manager entry backed by a ThinClaw wrapper
//!
//! The installed service runs `thinclaw run --no-onboard` and is configured to
//! restart automatically on failure.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

const SERVICE_LABEL: &str = "com.thinclaw.daemon";
#[cfg(target_os = "linux")]
const SYSTEMD_UNIT: &str = "thinclaw.service";
#[cfg(target_os = "windows")]
const WINDOWS_SERVICE_NAME: &str = "thinclaw";
#[cfg(target_os = "windows")]
const WINDOWS_SERVICE_DISPLAY_NAME: &str = "ThinClaw";

#[cfg(target_os = "windows")]
pub const WINDOWS_SERVICE_RUNTIME_COMMAND: &str = "__windows-service";

/// Route a service subcommand to the appropriate handler.
pub fn handle_command(command: &ServiceAction) -> Result<()> {
    match command {
        ServiceAction::Install => install(),
        ServiceAction::Start => start(),
        ServiceAction::Stop => stop(),
        ServiceAction::Status => status(),
        ServiceAction::Uninstall => uninstall(),
    }
}

/// The five service lifecycle actions.
#[derive(Debug, Clone)]
pub enum ServiceAction {
    Install,
    Start,
    Stop,
    Status,
    Uninstall,
}

fn install() -> Result<()> {
    let onboarding_blocker = onboarding_blocker();
    let force_install = force_service_install();
    if let Some(ref reason) = onboarding_blocker
        && !force_install
    {
        bail!(
            "Service install blocked: onboarding is not ready ({reason}). Run `thinclaw onboard` first, or set THINCLAW_FORCE_SERVICE_INSTALL=true to bypass this guard intentionally."
        );
    }
    if let Some(reason) = onboarding_blocker
        && force_install
    {
        println!("WARNING: Forcing service install before onboarding is ready ({reason}).");
        println!("  The service will start in headless bypass mode until onboarding is completed.");
        println!();
    }

    guard_remote_gateway_install(force_install)?;

    #[cfg(target_os = "macos")]
    {
        install_macos(force_install)?;
    }

    #[cfg(target_os = "linux")]
    {
        install_linux(force_install)?;
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::install()?;
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Service management is only supported on macOS, Linux, and Windows");
    }

    print_service_install_summary();
    Ok(())
}

fn start() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        start_macos()
    }

    #[cfg(target_os = "linux")]
    {
        start_linux()
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::start()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Service management is only supported on macOS, Linux, and Windows");
    }
}

fn stop() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        stop_macos()
    }

    #[cfg(target_os = "linux")]
    {
        stop_linux()
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::stop()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Service management is only supported on macOS, Linux, and Windows");
    }
}

fn status() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        status_macos()
    }

    #[cfg(target_os = "linux")]
    {
        status_linux()
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::status()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Service management is only supported on macOS, Linux, and Windows");
    }
}

fn uninstall() -> Result<()> {
    let _ = stop();

    #[cfg(target_os = "macos")]
    {
        uninstall_macos()
    }

    #[cfg(target_os = "linux")]
    {
        uninstall_linux()
    }

    #[cfg(target_os = "windows")]
    {
        windows_impl::uninstall()
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        bail!("Service management is only supported on macOS, Linux, and Windows");
    }
}

fn force_service_install() -> bool {
    std::env::var("THINCLAW_FORCE_SERVICE_INSTALL")
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn onboarding_blocker() -> Option<String> {
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_path(crate::platform::state_paths().env_file);

    #[cfg(any(feature = "postgres", feature = "libsql"))]
    {
        crate::setup::check_onboard_needed(None, false)
    }

    #[cfg(not(any(feature = "postgres", feature = "libsql")))]
    {
        None
    }
}

fn service_run_args(_force_no_onboard: bool) -> Vec<&'static str> {
    vec!["run", "--no-onboard"]
}

fn guard_remote_gateway_install(force_install: bool) -> Result<()> {
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_path(crate::platform::state_paths().env_file);

    let settings = crate::settings::Settings::load();
    let access =
        crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(Some(&settings));

    if remote_gateway_explicitly_configured(&settings)
        && access.enabled
        && access.auth_token.is_none()
        && !force_install
    {
        bail!(
            "Service install blocked: remote gateway is enabled without GATEWAY_AUTH_TOKEN. Run `thinclaw onboard --profile remote` or set a long random token in {}.",
            crate::platform::state_paths().env_file.display()
        );
    }

    if access.cli_enabled {
        println!("WARNING: CLI_ENABLED is true in the current environment.");
        println!("  The installed service will set CLI_ENABLED=false to avoid stdin EOF shutdown.");
        println!();
    }

    Ok(())
}

fn remote_gateway_explicitly_configured(settings: &crate::settings::Settings) -> bool {
    std::env::var_os("GATEWAY_ENABLED").is_some()
        || std::env::var_os("GATEWAY_HOST").is_some()
        || std::env::var_os("GATEWAY_PORT").is_some()
        || std::env::var_os("GATEWAY_AUTH_TOKEN").is_some()
        || settings.channels.gateway_enabled.is_some()
        || settings.channels.gateway_host.is_some()
        || settings.channels.gateway_port.is_some()
        || settings.channels.gateway_auth_token.is_some()
}

fn print_service_install_summary() {
    let settings = crate::settings::Settings::load();
    let access =
        crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(Some(&settings));
    let env_path = crate::platform::state_paths().env_file;

    println!("  Env file: {}", env_path.display());
    println!("  Runtime command: thinclaw run --no-onboard");
    println!("  WebUI URL: {}", access.local_url());
    if let Some(url) = access.token_url(false) {
        println!("  Token URL: {}", url);
    }
    println!("  SSH tunnel: {}", access.ssh_tunnel_command());
}

#[cfg(target_os = "macos")]
fn install_macos(force_no_onboard: bool) -> Result<()> {
    let file = macos_plist_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let logs_dir = thinclaw_logs_dir()?;
    std::fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");
    let service_args = service_run_args(force_no_onboard)
        .into_iter()
        .map(|arg| format!("    <string>{}</string>\n", xml_escape(arg)))
        .collect::<String>();

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
{service_args}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>ExitTimeOut</key>
  <integer>15</integer>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key>
    <string>{home}</string>
    <key>PATH</key>
    <string>{path}</string>
    <key>CLI_ENABLED</key>
    <string>false</string>
    <key>THINCLAW_SERVICE_MANAGER</key>
    <string>launchd</string>
  </dict>
  <key>StandardOutPath</key>
  <string>{stdout}</string>
  <key>StandardErrorPath</key>
  <string>{stderr}</string>
</dict>
</plist>
"#,
        label = SERVICE_LABEL,
        exe = xml_escape(&exe.display().to_string()),
        home = xml_escape(
            &dirs::home_dir()
                .map(|h| h.display().to_string())
                .unwrap_or_default()
        ),
        path = xml_escape(
            &std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/usr/local/bin".to_string())
        ),
        service_args = service_args,
        stdout = xml_escape(&stdout.display().to_string()),
        stderr = xml_escape(&stderr.display().to_string()),
    );

    std::fs::write(&file, plist)?;
    println!("Installed launchd service: {}", file.display());
    println!("  Start with: thinclaw service start");
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_linux(force_no_onboard: bool) -> Result<()> {
    let file = linux_unit_path()?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let exe = std::env::current_exe().context("failed to resolve current executable")?;
    let exec_args = service_run_args(force_no_onboard).join(" ");
    let unit = format!(
        "[Unit]\n\
         Description=ThinClaw daemon\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         Environment=CLI_ENABLED=false\n\
         Environment=THINCLAW_SERVICE_MANAGER=systemd\n\
         ExecStart=\"{exe}\" {exec_args}\n\
         Restart=always\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = exe.display(),
        exec_args = exec_args,
    );

    std::fs::write(&file, unit)?;
    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"])).ok();
    run_checked(Command::new("systemctl").args(["--user", "enable", SYSTEMD_UNIT])).ok();
    println!("Installed systemd user service: {}", file.display());
    println!("  Start with: thinclaw service start");
    Ok(())
}

#[cfg(target_os = "macos")]
fn start_macos() -> Result<()> {
    let plist = macos_plist_path()?;
    if !plist.exists() {
        bail!("Service not installed. Run `thinclaw service install` first.");
    }
    run_checked(Command::new("launchctl").arg("load").arg("-w").arg(&plist))?;
    run_checked(Command::new("launchctl").arg("start").arg(SERVICE_LABEL))?;
    println!("Service started");
    Ok(())
}

#[cfg(target_os = "linux")]
fn start_linux() -> Result<()> {
    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run_checked(Command::new("systemctl").args(["--user", "start", SYSTEMD_UNIT]))?;
    println!("Service started");
    Ok(())
}

#[cfg(target_os = "macos")]
fn stop_macos() -> Result<()> {
    let plist = macos_plist_path()?;
    run_checked(Command::new("launchctl").arg("stop").arg(SERVICE_LABEL)).ok();
    run_checked(
        Command::new("launchctl")
            .arg("unload")
            .arg("-w")
            .arg(&plist),
    )
    .ok();
    println!("Service stopped");
    Ok(())
}

#[cfg(target_os = "linux")]
fn stop_linux() -> Result<()> {
    run_checked(Command::new("systemctl").args(["--user", "stop", SYSTEMD_UNIT])).ok();
    println!("Service stopped");
    Ok(())
}

#[cfg(target_os = "macos")]
fn status_macos() -> Result<()> {
    let out = run_capture(Command::new("launchctl").arg("list"))?;
    let running = out.lines().any(|line| line.contains(SERVICE_LABEL));
    println!(
        "Service: {}",
        if running {
            "running/loaded"
        } else {
            "not loaded"
        }
    );
    println!("Unit: {}", macos_plist_path()?.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn status_linux() -> Result<()> {
    let state = run_capture(Command::new("systemctl").args(["--user", "is-active", SYSTEMD_UNIT]))
        .unwrap_or_else(|_| "unknown".into());
    println!("Service state: {}", state.trim());
    println!("Unit: {}", linux_unit_path()?.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn uninstall_macos() -> Result<()> {
    let file = macos_plist_path()?;
    if file.exists() {
        std::fs::remove_file(&file)
            .with_context(|| format!("failed to remove {}", file.display()))?;
    }
    println!("Service uninstalled ({})", file.display());
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall_linux() -> Result<()> {
    let file = linux_unit_path()?;
    if file.exists() {
        std::fs::remove_file(&file)
            .with_context(|| format!("failed to remove {}", file.display()))?;
    }
    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"])).ok();
    println!("Service uninstalled ({})", file.display());
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn run_windows_service_dispatcher(home_override: Option<PathBuf>) -> Result<()> {
    if let Some(home) = home_override {
        // SAFETY: The hidden Windows service runtime command runs during early
        // process setup before any worker threads are spawned.
        unsafe {
            std::env::set_var("THINCLAW_HOME", home);
        }
    }
    windows_impl::run_dispatcher()
}

fn macos_plist_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

#[cfg(target_os = "linux")]
fn linux_unit_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("could not find home directory")?;
    Ok(home
        .join(".config")
        .join("systemd")
        .join("user")
        .join(SYSTEMD_UNIT))
}

fn thinclaw_logs_dir() -> Result<PathBuf> {
    Ok(crate::platform::state_paths().logs_dir)
}

fn run_checked(command: &mut Command) -> Result<()> {
    let output = command.output().context("failed to spawn command")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("command failed: {}", stderr.trim());
    }
    Ok(())
}

fn run_capture(command: &mut Command) -> Result<String> {
    let output = command.output().context("failed to spawn command")?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    Ok(text)
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use std::ffi::OsString;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, anyhow};
    use windows_service::service::{
        ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
        ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
        ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::service_dispatcher;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::{
        Error as WinServiceError, Result as WinServiceResult, define_windows_service,
    };

    use super::{
        WINDOWS_SERVICE_DISPLAY_NAME, WINDOWS_SERVICE_NAME, WINDOWS_SERVICE_RUNTIME_COMMAND,
    };

    const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

    define_windows_service!(ffi_service_main, service_main);

    pub(super) fn install() -> Result<()> {
        std::fs::create_dir_all(crate::platform::state_paths().logs_dir.clone())
            .context("failed to create ThinClaw logs directory")?;

        let manager_access = ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE;
        let service_manager = ServiceManager::local_computer(None::<&str>, manager_access)
            .context("failed to connect to the Windows Service Control Manager")?;

        let service_info = service_info()?;
        let service_access = ServiceAccess::QUERY_STATUS
            | ServiceAccess::QUERY_CONFIG
            | ServiceAccess::CHANGE_CONFIG
            | ServiceAccess::START
            | ServiceAccess::STOP
            | ServiceAccess::DELETE;

        let service = match service_manager.create_service(&service_info, service_access) {
            Ok(service) => service,
            Err(_) => {
                let service = service_manager
                    .open_service(WINDOWS_SERVICE_NAME, service_access)
                    .with_context(|| {
                        format!("failed to create or open Windows service '{WINDOWS_SERVICE_NAME}'")
                    })?;
                service
                    .change_config(&service_info)
                    .context("failed to update Windows service configuration")?;
                service
            }
        };

        service
            .set_description("ThinClaw background runtime service")
            .context("failed to set Windows service description")?;

        let failure_actions = ServiceFailureActions {
            reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 60 * 60)),
            reboot_msg: None,
            command: None,
            actions: Some(vec![
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(5),
                },
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(15),
                },
                ServiceAction {
                    action_type: ServiceActionType::Restart,
                    delay: Duration::from_secs(30),
                },
            ]),
        };
        service
            .update_failure_actions(failure_actions)
            .context("failed to configure Windows service recovery actions")?;
        service
            .set_failure_actions_on_non_crash_failures(true)
            .context("failed to enable Windows service recovery on non-crash failures")?;

        println!(
            "Installed Windows service: {}",
            WINDOWS_SERVICE_DISPLAY_NAME
        );
        println!("  Start with: thinclaw service start");
        Ok(())
    }

    pub(super) fn start() -> Result<()> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("failed to connect to the Windows Service Control Manager")?;
        let service = manager
            .open_service(
                WINDOWS_SERVICE_NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::START,
            )
            .with_context(|| {
                format!(
                    "Windows service '{}' is not installed. Run `thinclaw service install` first.",
                    WINDOWS_SERVICE_NAME
                )
            })?;

        let status = service
            .query_status()
            .context("failed to query Windows service status")?;
        if status.current_state == ServiceState::Running {
            println!("Service already running");
            return Ok(());
        }

        service
            .start(&[] as &[&std::ffi::OsStr])
            .context("failed to start Windows service")?;
        let status = wait_for_state(&service, ServiceState::Running, Duration::from_secs(20))
            .context("Windows service did not reach the running state")?;

        println!(
            "Service started (state: {}, PID: {})",
            state_label(status.current_state),
            status.process_id.unwrap_or_default()
        );
        Ok(())
    }

    pub(super) fn stop() -> Result<()> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("failed to connect to the Windows Service Control Manager")?;
        let service = manager
            .open_service(
                WINDOWS_SERVICE_NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
            )
            .with_context(|| {
                format!(
                    "Windows service '{}' is not installed. Run `thinclaw service install` first.",
                    WINDOWS_SERVICE_NAME
                )
            })?;

        let status = service
            .query_status()
            .context("failed to query Windows service status")?;
        if status.current_state == ServiceState::Stopped {
            println!("Service already stopped");
            return Ok(());
        }

        service.stop().context("failed to stop Windows service")?;
        let status = wait_for_state(&service, ServiceState::Stopped, Duration::from_secs(20))
            .context("Windows service did not stop cleanly")?;

        println!(
            "Service stopped (state: {})",
            state_label(status.current_state)
        );
        Ok(())
    }

    pub(super) fn status() -> Result<()> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("failed to connect to the Windows Service Control Manager")?;
        let service = manager
            .open_service(
                WINDOWS_SERVICE_NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::QUERY_CONFIG,
            )
            .with_context(|| {
                format!(
                    "Windows service '{}' is not installed. Run `thinclaw service install` first.",
                    WINDOWS_SERVICE_NAME
                )
            })?;

        let status = service
            .query_status()
            .context("failed to query Windows service status")?;
        let config = service
            .query_config()
            .context("failed to query Windows service configuration")?;

        println!("Service state: {}", state_label(status.current_state));
        println!("Service name: {}", WINDOWS_SERVICE_NAME);
        println!("Display name: {}", WINDOWS_SERVICE_DISPLAY_NAME);
        println!("Startup: {:?}", config.start_type);
        println!("Binary: {}", config.executable_path.display());
        if let Some(pid) = status.process_id {
            println!("PID: {}", pid);
        }
        Ok(())
    }

    pub(super) fn uninstall() -> Result<()> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .context("failed to connect to the Windows Service Control Manager")?;
        let service = manager
            .open_service(
                WINDOWS_SERVICE_NAME,
                ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
            )
            .with_context(|| {
                format!(
                    "Windows service '{}' is not installed. Nothing to uninstall.",
                    WINDOWS_SERVICE_NAME
                )
            })?;

        if let Ok(status) = service.query_status()
            && status.current_state != ServiceState::Stopped
        {
            let _ = service.stop();
            let _ = wait_for_state(&service, ServiceState::Stopped, Duration::from_secs(20));
        }

        service
            .delete()
            .context("failed to mark the Windows service for deletion")?;
        println!("Service uninstalled ({})", WINDOWS_SERVICE_DISPLAY_NAME);
        Ok(())
    }

    pub(super) fn run_dispatcher() -> Result<()> {
        service_dispatcher::start(WINDOWS_SERVICE_NAME, ffi_service_main)
            .context("failed to start Windows service dispatcher")
    }

    fn service_main(_arguments: Vec<OsString>) {
        if let Err(error) = run_service() {
            append_manager_log(&format!("Windows service runtime failed: {error}"));
        }
    }

    fn run_service() -> WinServiceResult<()> {
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

        let event_handler = move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                ServiceControl::Stop => {
                    let _ = shutdown_tx.send(());
                    ServiceControlHandlerResult::NoError
                }
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };

        let status_handle = service_control_handler::register(WINDOWS_SERVICE_NAME, event_handler)?;

        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::StartPending,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::NO_ERROR,
            checkpoint: 1,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        })?;

        let mut child = spawn_runtime_child().map_err(WinServiceError::Winapi)?;

        status_handle.set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP,
            exit_code: ServiceExitCode::NO_ERROR,
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })?;

        loop {
            match shutdown_rx.recv_timeout(Duration::from_secs(1)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    status_handle.set_service_status(ServiceStatus {
                        service_type: SERVICE_TYPE,
                        current_state: ServiceState::StopPending,
                        controls_accepted: ServiceControlAccept::empty(),
                        exit_code: ServiceExitCode::NO_ERROR,
                        checkpoint: 1,
                        wait_hint: Duration::from_secs(15),
                        process_id: None,
                    })?;

                    terminate_child(&mut child);

                    status_handle.set_service_status(ServiceStatus {
                        service_type: SERVICE_TYPE,
                        current_state: ServiceState::Stopped,
                        controls_accepted: ServiceControlAccept::empty(),
                        exit_code: ServiceExitCode::NO_ERROR,
                        checkpoint: 0,
                        wait_hint: Duration::default(),
                        process_id: None,
                    })?;
                    return Ok(());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }

            if let Some(exit) = child.try_wait().map_err(WinServiceError::Winapi)? {
                let code = exit.code().unwrap_or(1);
                if code == 0 {
                    status_handle.set_service_status(ServiceStatus {
                        service_type: SERVICE_TYPE,
                        current_state: ServiceState::Stopped,
                        controls_accepted: ServiceControlAccept::empty(),
                        exit_code: ServiceExitCode::NO_ERROR,
                        checkpoint: 0,
                        wait_hint: Duration::default(),
                        process_id: None,
                    })?;
                    return Ok(());
                }

                append_manager_log(&format!(
                    "ThinClaw runtime exited unexpectedly with code {code}; letting SCM restart it"
                ));

                status_handle.set_service_status(ServiceStatus {
                    service_type: SERVICE_TYPE,
                    current_state: ServiceState::Stopped,
                    controls_accepted: ServiceControlAccept::empty(),
                    exit_code: ServiceExitCode::ServiceSpecific(code.max(1) as u32),
                    checkpoint: 0,
                    wait_hint: Duration::default(),
                    process_id: None,
                })?;
                return Err(WinServiceError::Winapi(std::io::Error::other(format!(
                    "ThinClaw runtime exited with code {code}"
                ))));
            }
        }
    }

    fn spawn_runtime_child() -> std::io::Result<Child> {
        let exe = std::env::current_exe()?;
        let logs_dir = crate::platform::state_paths().logs_dir;
        std::fs::create_dir_all(&logs_dir)?;

        let stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs_dir.join("service.stdout.log"))?;
        let stderr = OpenOptions::new()
            .create(true)
            .append(true)
            .open(logs_dir.join("service.stderr.log"))?;

        let mut cmd = Command::new(exe);
        for arg in super::service_run_args(false) {
            cmd.arg(arg);
        }
        cmd.env("THINCLAW_SERVICE_MANAGER", "windows")
            .env("CLI_ENABLED", "false")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .stdin(Stdio::null());

        if let Some(home) = std::env::var_os("THINCLAW_HOME") {
            cmd.env("THINCLAW_HOME", home);
        }

        cmd.spawn()
    }

    fn terminate_child(child: &mut Child) {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }

        let _ = child.kill();
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(15) {
            match child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => thread::sleep(Duration::from_millis(250)),
                Err(_) => return,
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    fn wait_for_state(
        service: &windows_service::service::Service,
        target: ServiceState,
        timeout: Duration,
    ) -> Result<ServiceStatus> {
        let started = Instant::now();
        loop {
            let status = service
                .query_status()
                .context("failed to query Windows service status")?;
            if status.current_state == target {
                return Ok(status);
            }
            if started.elapsed() >= timeout {
                return Err(anyhow!(
                    "timed out waiting for service state {} (last state: {})",
                    state_label(target),
                    state_label(status.current_state)
                ));
            }
            thread::sleep(Duration::from_millis(500));
        }
    }

    fn service_info() -> Result<ServiceInfo> {
        let exe = std::env::current_exe().context("failed to resolve current executable")?;
        let home = crate::platform::state_paths().home;

        Ok(ServiceInfo {
            name: OsString::from(WINDOWS_SERVICE_NAME),
            display_name: OsString::from(WINDOWS_SERVICE_DISPLAY_NAME),
            service_type: SERVICE_TYPE,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: exe,
            launch_arguments: vec![
                OsString::from(WINDOWS_SERVICE_RUNTIME_COMMAND),
                OsString::from("--home"),
                home.into_os_string(),
            ],
            dependencies: vec![],
            account_name: None,
            account_password: None,
        })
    }

    fn state_label(state: ServiceState) -> &'static str {
        match state {
            ServiceState::Stopped => "stopped",
            ServiceState::StartPending => "start-pending",
            ServiceState::StopPending => "stop-pending",
            ServiceState::Running => "running",
            ServiceState::ContinuePending => "continue-pending",
            ServiceState::PausePending => "pause-pending",
            ServiceState::Paused => "paused",
        }
    }

    fn append_manager_log(message: &str) {
        let logs_dir = crate::platform::state_paths().logs_dir;
        if std::fs::create_dir_all(&logs_dir).is_err() {
            return;
        }
        let path = logs_dir.join("service.manager.log");
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(
                file,
                "[{}] {}",
                chrono::Utc::now().to_rfc3339(),
                message.trim()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::platform::shell_launcher;
    use crate::service::*;

    #[test]
    fn xml_escape_handles_reserved_chars() {
        let escaped = xml_escape("<&>\"' and text");
        assert_eq!(escaped, "&lt;&amp;&gt;&quot;&apos; and text");
    }

    #[test]
    fn xml_escape_passes_through_plain_text() {
        assert_eq!(xml_escape("hello world"), "hello world");
    }

    #[test]
    fn run_capture_reads_stdout() {
        let mut command = shell_launcher().std_command("echo hello");
        let out = run_capture(&mut command).expect("stdout capture should succeed");
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn run_capture_falls_back_to_stderr() {
        let script = if cfg!(target_os = "windows") {
            "echo warn 1>&2"
        } else {
            "echo warn 1>&2"
        };
        let mut command = shell_launcher().std_command(script);
        let out = run_capture(&mut command).expect("stderr capture should succeed");
        assert_eq!(out.trim(), "warn");
    }

    #[test]
    fn run_checked_errors_on_non_zero_exit() {
        let script = if cfg!(target_os = "windows") {
            "exit /B 17"
        } else {
            "exit 17"
        };
        let mut command = shell_launcher().std_command(script);
        let err = run_checked(&mut command).expect_err("non-zero exit should error");
        assert!(err.to_string().contains("command failed"));
    }

    #[test]
    fn run_checked_succeeds_on_zero_exit() {
        let script = if cfg!(target_os = "windows") {
            "exit /B 0"
        } else {
            "exit 0"
        };
        let mut command = shell_launcher().std_command(script);
        assert!(run_checked(&mut command).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_plist_path_has_expected_suffix() {
        let path = macos_plist_path().unwrap();
        let s = path.to_string_lossy();
        assert!(
            s.ends_with("Library/LaunchAgents/com.thinclaw.daemon.plist"),
            "unexpected path: {s}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_unit_path_has_expected_suffix() {
        let path = linux_unit_path().unwrap();
        let s = path.to_string_lossy();
        assert!(
            s.ends_with(".config/systemd/user/thinclaw.service"),
            "unexpected path: {s}"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_runtime_command_is_hidden_internal_name() {
        assert_eq!(WINDOWS_SERVICE_RUNTIME_COMMAND, "__windows-service");
    }

    #[test]
    fn logs_dir_under_thinclaw() {
        let path = thinclaw_logs_dir().unwrap();
        let s = path.to_string_lossy();
        assert!(s.ends_with(".thinclaw/logs"), "unexpected path: {s}");
    }
}
