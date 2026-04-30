use std::future::Future;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeExecRegistrationMode {
    Disabled,
    LocalHost,
    DockerSandbox,
}

pub fn process_registration_mode(workspace_mode: &str) -> RuntimeExecRegistrationMode {
    match workspace_mode {
        "sandboxed" | "project" => RuntimeExecRegistrationMode::Disabled,
        _ => RuntimeExecRegistrationMode::LocalHost,
    }
}

pub fn execute_code_registration_mode(
    workspace_mode: &str,
    sandbox_enabled: bool,
) -> RuntimeExecRegistrationMode {
    match workspace_mode {
        "sandboxed" if sandbox_enabled => RuntimeExecRegistrationMode::DockerSandbox,
        "sandboxed" | "project" => RuntimeExecRegistrationMode::Disabled,
        _ => RuntimeExecRegistrationMode::LocalHost,
    }
}

pub fn desktop_autonomy_headless_blocker() -> Option<&'static str> {
    let runtime_profile = std::env::var("THINCLAW_RUNTIME_PROFILE").unwrap_or_default();
    desktop_autonomy_headless_blocker_for(
        runtime_profile.trim(),
        thinclaw_platform::env_flag_enabled("THINCLAW_HEADLESS"),
    )
}

pub fn desktop_autonomy_headless_blocker_for(
    runtime_profile: &str,
    headless_enabled: bool,
) -> Option<&'static str> {
    let normalized_profile = runtime_profile
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    match normalized_profile.as_str() {
        "pi" | "pi-os-lite" | "pi-os-lite-64" | "raspberry-pi-os-lite" => Some("pi-os-lite-64"),
        _ if headless_enabled => Some("headless"),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeEntryMode {
    Default,
    Cli,
    Tui,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AppBuilderFlags {
    pub no_db: bool,
}

/// Initialize tracing for simple CLI commands.
pub fn init_cli_tracing(debug: bool) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            if debug {
                EnvFilter::new("debug")
            } else {
                EnvFilter::new("warn")
            }
        }))
        .init();
}

pub fn restart_is_managed_by_service() -> bool {
    std::env::var_os("INVOCATION_ID").is_some()
        || std::env::var_os("JOURNAL_STREAM").is_some()
        || std::env::var_os("SYSTEMD_EXEC_PID").is_some()
        || std::env::var_os("LAUNCH_JOB_NAME").is_some()
        || std::env::var_os("THINCLAW_SERVICE_MANAGER").is_some()
}

pub fn relaunch_current_process() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.args(std::env::args_os().skip(1));
    let child = cmd.spawn()?;
    eprintln!(
        "Restarting ThinClaw (spawned PID {} from {})...",
        child.id(),
        exe.display()
    );
    Ok(())
}

pub fn block_on_async_main<F>(future: F) -> anyhow::Result<()>
where
    F: Future<Output = anyhow::Result<()>>,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(Box::pin(future))
}

const STARTUP_SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Minimal terminal spinner shown during quiet interactive startup.
pub struct QuietStartupSpinner {
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl QuietStartupSpinner {
    pub fn start() -> Self {
        let running = Arc::new(AtomicBool::new(true));
        let running_for_thread = Arc::clone(&running);

        let handle = std::thread::spawn(move || {
            let mut frame_idx = 0usize;
            let mut stdout = std::io::stdout();

            while running_for_thread.load(Ordering::Relaxed) {
                let frame = STARTUP_SPINNER_FRAMES[frame_idx % STARTUP_SPINNER_FRAMES.len()];
                let _ = write!(stdout, "\r\x1b[2K  {frame} Starting ThinClaw...");
                let _ = stdout.flush();
                frame_idx += 1;
                std::thread::sleep(Duration::from_millis(80));
            }

            let _ = write!(stdout, "\r\x1b[2K");
            let _ = stdout.flush();
        });

        Self {
            running,
            handle: Some(handle),
        }
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for QuietStartupSpinner {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn should_show_quiet_startup_spinner(
    should_run_agent: bool,
    debug: bool,
    has_single_message: bool,
    cli_enabled: bool,
    has_rust_log_override: bool,
    stdin_is_tty: bool,
    stdout_is_tty: bool,
) -> bool {
    should_run_agent
        && !debug
        && !has_single_message
        && cli_enabled
        && !has_rust_log_override
        && stdin_is_tty
        && stdout_is_tty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricted_modes_disable_background_processes() {
        assert_eq!(
            process_registration_mode("sandboxed"),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            process_registration_mode("project"),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            process_registration_mode("unrestricted"),
            RuntimeExecRegistrationMode::LocalHost
        );
    }

    #[test]
    fn execute_code_requires_real_isolation_in_restricted_modes() {
        assert_eq!(
            execute_code_registration_mode("sandboxed", true),
            RuntimeExecRegistrationMode::DockerSandbox
        );
        assert_eq!(
            execute_code_registration_mode("sandboxed", false),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            execute_code_registration_mode("project", true),
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            execute_code_registration_mode("unrestricted", false),
            RuntimeExecRegistrationMode::LocalHost
        );
    }

    #[test]
    fn pi_os_lite_runtime_blocks_desktop_autonomy_registration() {
        assert_eq!(
            desktop_autonomy_headless_blocker_for("pi-os-lite-64", false),
            Some("pi-os-lite-64")
        );
        assert_eq!(
            desktop_autonomy_headless_blocker_for("raspberry-pi-os-lite", false),
            Some("pi-os-lite-64")
        );
        assert_eq!(
            desktop_autonomy_headless_blocker_for("remote", true),
            Some("headless")
        );
        assert_eq!(desktop_autonomy_headless_blocker_for("remote", false), None);
    }

    #[test]
    fn quiet_spinner_shows_for_interactive_quiet_agent_runs() {
        assert!(should_show_quiet_startup_spinner(
            true, false, false, true, false, true, true
        ));
    }

    #[test]
    fn quiet_spinner_stays_off_for_debug_runs() {
        assert!(!should_show_quiet_startup_spinner(
            true, true, false, true, false, true, true
        ));
    }

    #[test]
    fn quiet_spinner_stays_off_for_non_tty_or_message_runs() {
        assert!(!should_show_quiet_startup_spinner(
            true, false, true, true, false, true, true
        ));
        assert!(!should_show_quiet_startup_spinner(
            true, false, false, true, false, false, true
        ));
        assert!(!should_show_quiet_startup_spinner(
            true, false, false, true, false, true, false
        ));
    }
}
