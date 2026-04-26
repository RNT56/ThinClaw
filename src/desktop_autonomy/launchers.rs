use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn write_session_launcher(&self) -> Result<PathBuf, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => self.write_launch_agent_plist().await,
            DesktopBridgeBackend::WindowsPowerShell => self.write_windows_session_launcher().await,
            DesktopBridgeBackend::LinuxPython => self.write_linux_session_launcher().await,
            DesktopBridgeBackend::Unsupported => {
                Err("session launcher installation is unsupported on this platform".to_string())
            }
        }
    }

    pub(super) async fn activate_session_launcher(
        &self,
        launcher_path: &Path,
    ) -> Result<(), String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => self.load_launch_agent(launcher_path).await,
            DesktopBridgeBackend::WindowsPowerShell => {
                self.activate_windows_session_launcher(launcher_path).await
            }
            DesktopBridgeBackend::LinuxPython => {
                self.activate_linux_session_launcher(launcher_path).await
            }
            DesktopBridgeBackend::Unsupported => {
                Err("session launcher activation is unsupported on this platform".to_string())
            }
        }
    }

    pub(super) async fn write_launch_agent_plist(&self) -> Result<PathBuf, String> {
        #[cfg(not(target_os = "macos"))]
        {
            Err("launch agent installation is only supported on macOS".to_string())
        }

        #[cfg(target_os = "macos")]
        {
            let plist_path = self.session_launcher_path()?;
            if let Some(parent) = plist_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("failed to create launch agent dir: {e}"))?;
            }
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let home = self.session_launcher_home()?;
            let logs_dir = home.join(".thinclaw").join("logs");
            tokio::fs::create_dir_all(&logs_dir)
                .await
                .map_err(|e| format!("failed to create autonomy logs dir: {e}"))?;
            let stdout = logs_dir.join("desktop-autonomy.stdout.log");
            let stderr = logs_dir.join("desktop-autonomy.stderr.log");
            let mut env_entries = vec![
                (
                    "HOME".to_string(),
                    xml_escape(home.to_string_lossy().as_ref()),
                ),
                (
                    "PATH".to_string(),
                    "/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
                ),
                ("DESKTOP_AUTONOMY_ENABLED".to_string(), "true".to_string()),
                (
                    "DESKTOP_AUTONOMY_PROFILE".to_string(),
                    self.config.profile.as_str().to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_DEPLOYMENT_MODE".to_string(),
                    self.config.deployment_mode.as_str().to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_MAX_CONCURRENT_JOBS".to_string(),
                    self.config.desktop_max_concurrent_jobs.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_ACTION_TIMEOUT_SECS".to_string(),
                    self.config.desktop_action_timeout_secs.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_CAPTURE_EVIDENCE".to_string(),
                    self.config.capture_evidence.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH".to_string(),
                    xml_escape(self.config.emergency_stop_path.to_string_lossy().as_ref()),
                ),
                (
                    "DESKTOP_AUTONOMY_PAUSE_ON_BOOTSTRAP_FAILURE".to_string(),
                    self.config.pause_on_bootstrap_failure.to_string(),
                ),
                (
                    "DESKTOP_AUTONOMY_KILL_SWITCH_HOTKEY".to_string(),
                    xml_escape(&self.config.kill_switch_hotkey),
                ),
            ];
            if let Some(username) = self.config.target_username.as_deref() {
                env_entries.push((
                    "DESKTOP_AUTONOMY_TARGET_USERNAME".to_string(),
                    xml_escape(username),
                ));
            }
            let environment_variables = env_entries
                .into_iter()
                .map(|(key, value)| format!("    <key>{key}</key>\n    <string>{value}</string>\n"))
                .collect::<String>();

            let plist = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
  <key>Label</key>\n\
  <string>{label}</string>\n\
  <key>ProgramArguments</key>\n\
  <array>\n\
    <string>{exe}</string>\n\
    <string>run</string>\n\
    <string>--no-onboard</string>\n\
  </array>\n\
  <key>RunAtLoad</key>\n\
  <true/>\n\
  <key>KeepAlive</key>\n\
  <true/>\n\
  <key>EnvironmentVariables</key>\n\
  <dict>\n\
{environment_variables}\
  </dict>\n\
  <key>StandardOutPath</key>\n\
  <string>{stdout}</string>\n\
  <key>StandardErrorPath</key>\n\
  <string>{stderr}</string>\n\
</dict>\n\
</plist>\n",
                label = self.launch_agent_label(),
                exe = xml_escape(exe.to_string_lossy().as_ref()),
                environment_variables = environment_variables,
                stdout = xml_escape(stdout.to_string_lossy().as_ref()),
                stderr = xml_escape(stderr.to_string_lossy().as_ref()),
            );
            tokio::fs::write(&plist_path, plist)
                .await
                .map_err(|e| format!("failed to write launch agent plist: {e}"))?;
            Ok(plist_path)
        }
    }

    pub(super) async fn load_launch_agent(&self, plist_path: &Path) -> Result<(), String> {
        #[cfg(not(target_os = "macos"))]
        {
            let _ = plist_path;
            Err("launch agent bootstrap is only supported on macOS".to_string())
        }

        #[cfg(target_os = "macos")]
        {
            let uid = self.target_session_subject().await?;
            let _ = run_cmd(
                Command::new("launchctl")
                    .arg("bootout")
                    .arg(format!("gui/{uid}"))
                    .arg(plist_path),
            )
            .await;
            run_cmd(
                Command::new("launchctl")
                    .arg("bootstrap")
                    .arg(format!("gui/{uid}"))
                    .arg(plist_path),
            )
            .await?;
            run_cmd(
                Command::new("launchctl")
                    .arg("kickstart")
                    .arg("-k")
                    .arg(format!("gui/{uid}/{}", self.launch_agent_label())),
            )
            .await?;
            Ok(())
        }
    }

    pub(super) async fn write_windows_session_launcher(&self) -> Result<PathBuf, String> {
        #[cfg(not(target_os = "windows"))]
        {
            Err("windows session launcher is only supported on Windows".to_string())
        }

        #[cfg(target_os = "windows")]
        {
            let launcher_path = self.session_launcher_path()?;
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let mut lines = vec![
                "@echo off".to_string(),
                "set DESKTOP_AUTONOMY_ENABLED=true".to_string(),
                format!(
                    "set DESKTOP_AUTONOMY_PROFILE={}",
                    self.config.profile.as_str()
                ),
                format!(
                    "set DESKTOP_AUTONOMY_DEPLOYMENT_MODE={}",
                    self.config.deployment_mode.as_str()
                ),
                format!(
                    "set DESKTOP_AUTONOMY_CAPTURE_EVIDENCE={}",
                    self.config.capture_evidence
                ),
                format!(
                    "set DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH={}",
                    self.config.emergency_stop_path.display()
                ),
            ];
            if let Some(username) = self.config.target_username.as_deref() {
                lines.push(format!("set DESKTOP_AUTONOMY_TARGET_USERNAME={username}"));
            }
            lines.push(format!("\"{}\" run --no-onboard", exe.display()));
            let script = format!("{}\r\n", lines.join("\r\n"));
            tokio::fs::write(&launcher_path, script)
                .await
                .map_err(|e| format!("failed to write windows session launcher: {e}"))?;
            Ok(launcher_path)
        }
    }

    pub(super) async fn activate_windows_session_launcher(
        &self,
        launcher_path: &Path,
    ) -> Result<(), String> {
        #[cfg(not(target_os = "windows"))]
        {
            let _ = launcher_path;
            Err("windows session launcher activation is only supported on Windows".to_string())
        }

        #[cfg(target_os = "windows")]
        {
            let task_name = self.launch_agent_label();
            let launcher_command = format!("\"{}\"", launcher_path.display());
            let mut command = Command::new("schtasks");
            command
                .arg("/Create")
                .arg("/F")
                .arg("/TN")
                .arg(&task_name)
                .arg("/SC")
                .arg("ONLOGON")
                .arg("/TR")
                .arg(&launcher_command);

            if self.config.deployment_mode == crate::settings::DesktopDeploymentMode::DedicatedUser
            {
                let username = self
                    .config
                    .target_username
                    .as_deref()
                    .ok_or_else(|| "missing target username".to_string())?;
                command.arg("/RU").arg(username);
                if let Some(secret) = crate::platform::secure_store::get_api_key(&format!(
                    "ThinClaw Desktop Autonomy/{username}"
                ))
                .await
                {
                    command.arg("/RP").arg(secret);
                }
            }

            run_cmd(&mut command).await?;
            let _ = run_cmd(
                Command::new("schtasks")
                    .arg("/Run")
                    .arg("/TN")
                    .arg(&task_name),
            )
            .await;
            Ok(())
        }
    }

    pub(super) async fn write_linux_session_launcher(&self) -> Result<PathBuf, String> {
        #[cfg(not(target_os = "linux"))]
        {
            Err("linux session launcher is only supported on Linux".to_string())
        }

        #[cfg(target_os = "linux")]
        {
            let launcher_path = self.session_launcher_path()?;
            if let Some(parent) = launcher_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("failed to create linux autostart dir: {e}"))?;
            }
            let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
            let home = self.session_launcher_home()?;
            let wrapper_path = home
                .join(".local")
                .join("bin")
                .join("thinclaw-desktop-autonomy-session");
            if let Some(parent) = wrapper_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("failed to create linux launcher bin dir: {e}"))?;
            }
            let mut env_lines = vec![
                "export DESKTOP_AUTONOMY_ENABLED=true".to_string(),
                format!(
                    "export DESKTOP_AUTONOMY_PROFILE={}",
                    shell_single_quote(self.config.profile.as_str())
                ),
                format!(
                    "export DESKTOP_AUTONOMY_DEPLOYMENT_MODE={}",
                    shell_single_quote(self.config.deployment_mode.as_str())
                ),
                format!(
                    "export DESKTOP_AUTONOMY_MAX_CONCURRENT_JOBS={}",
                    shell_single_quote(&self.config.desktop_max_concurrent_jobs.to_string())
                ),
                format!(
                    "export DESKTOP_AUTONOMY_ACTION_TIMEOUT_SECS={}",
                    shell_single_quote(&self.config.desktop_action_timeout_secs.to_string())
                ),
                format!(
                    "export DESKTOP_AUTONOMY_CAPTURE_EVIDENCE={}",
                    shell_single_quote(&self.config.capture_evidence.to_string())
                ),
                format!(
                    "export DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH={}",
                    shell_single_quote(self.config.emergency_stop_path.to_string_lossy().as_ref())
                ),
                format!(
                    "export DESKTOP_AUTONOMY_PAUSE_ON_BOOTSTRAP_FAILURE={}",
                    shell_single_quote(&self.config.pause_on_bootstrap_failure.to_string())
                ),
                format!(
                    "export DESKTOP_AUTONOMY_KILL_SWITCH_HOTKEY={}",
                    shell_single_quote(&self.config.kill_switch_hotkey)
                ),
            ];
            if let Some(username) = self.config.target_username.as_deref() {
                env_lines.push(format!(
                    "export DESKTOP_AUTONOMY_TARGET_USERNAME={}",
                    shell_single_quote(username)
                ));
            }
            let wrapper = format!(
                "#!/bin/sh\nset -eu\nexport HOME={home}\n{env}\nexec {exe} run --no-onboard\n",
                home = shell_single_quote(home.to_string_lossy().as_ref()),
                env = env_lines.join("\n"),
                exe = shell_single_quote(exe.to_string_lossy().as_ref()),
            );
            tokio::fs::write(&wrapper_path, wrapper)
                .await
                .map_err(|e| format!("failed to write linux session wrapper: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = tokio::fs::metadata(&wrapper_path)
                    .await
                    .map_err(|e| format!("failed to inspect linux session wrapper: {e}"))?
                    .permissions();
                perms.set_mode(0o755);
                tokio::fs::set_permissions(&wrapper_path, perms)
                    .await
                    .map_err(|e| format!("failed to chmod linux session wrapper: {e}"))?;
            }
            let desktop_entry = format!(
                "[Desktop Entry]\nType=Application\nName=ThinClaw Desktop Autonomy\nComment=ThinClaw reckless desktop session launcher\nExec={}\nPath={}\nX-GNOME-Autostart-enabled=true\nX-KDE-autostart-after=panel\nTerminal=false\n",
                wrapper_path.display(),
                home.display(),
            );
            tokio::fs::write(&launcher_path, desktop_entry)
                .await
                .map_err(|e| format!("failed to write linux session launcher: {e}"))?;
            if self.config.deployment_mode == crate::settings::DesktopDeploymentMode::DedicatedUser
                && let Some(username) = self.config.target_username.as_deref()
                && self.has_privileged_bootstrap().await
            {
                let user = shell_single_quote(username);
                let launcher = shell_single_quote(launcher_path.to_string_lossy().as_ref());
                let wrapper = shell_single_quote(wrapper_path.to_string_lossy().as_ref());
                let _ = run_cmd(
                    Command::new("sh")
                        .arg("-lc")
                        .arg(format!("chown {user}:{user} {launcher} {wrapper}")),
                )
                .await;
            }
            Ok(launcher_path)
        }
    }

    pub(super) async fn activate_linux_session_launcher(
        &self,
        launcher_path: &Path,
    ) -> Result<(), String> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = launcher_path;
            Err("linux session launcher activation is only supported on Linux".to_string())
        }

        #[cfg(target_os = "linux")]
        {
            let raw = tokio::fs::read_to_string(launcher_path)
                .await
                .map_err(|e| format!("failed to read linux session launcher: {e}"))?;
            if !raw.contains("[Desktop Entry]")
                || !raw.contains("Exec=")
                || !raw.contains("thinclaw-desktop-autonomy-session")
            {
                return Err(
                    "linux session launcher does not contain the required cross-desktop autostart entry"
                        .to_string(),
                );
            }
            Ok(())
        }
    }

    pub(super) fn launch_agent_label(&self) -> String {
        format!(
            "com.thinclaw.desktop-autonomy.{}",
            self.config.deployment_mode.as_str()
        )
    }

    pub(super) fn session_launcher_home(&self) -> Result<PathBuf, String> {
        match self.config.deployment_mode {
            crate::settings::DesktopDeploymentMode::WholeMachineAdmin => dirs::home_dir()
                .ok_or_else(|| "failed to resolve current user home directory".to_string()),
            crate::settings::DesktopDeploymentMode::DedicatedUser => {
                let username = self
                    .config
                    .target_username
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| {
                        "dedicated_user deployment requires desktop_autonomy.target_username"
                            .to_string()
                    })?;
                match self.bridge_backend() {
                    DesktopBridgeBackend::MacOsSwift => Ok(PathBuf::from("/Users").join(username)),
                    DesktopBridgeBackend::WindowsPowerShell => {
                        Ok(PathBuf::from(r"C:\Users").join(username))
                    }
                    DesktopBridgeBackend::LinuxPython => Ok(linux_user_home(username)
                        .unwrap_or_else(|| PathBuf::from("/home").join(username))),
                    DesktopBridgeBackend::Unsupported => {
                        Err("failed to resolve target home for unsupported platform".to_string())
                    }
                }
            }
        }
    }

    pub(super) fn session_launcher_path(&self) -> Result<PathBuf, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => Ok(self
                .session_launcher_home()?
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{}.plist", self.launch_agent_label()))),
            DesktopBridgeBackend::WindowsPowerShell => Ok(self
                .state_root
                .join(format!("{}.cmd", self.launch_agent_label()))),
            DesktopBridgeBackend::LinuxPython => Ok(self
                .session_launcher_home()?
                .join(".config")
                .join("autostart")
                .join(format!("{}.desktop", self.launch_agent_label()))),
            DesktopBridgeBackend::Unsupported => {
                Err("session launcher path is unsupported on this platform".to_string())
            }
        }
    }

    pub(super) async fn target_session_subject(&self) -> Result<String, String> {
        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => match self.config.deployment_mode {
                crate::settings::DesktopDeploymentMode::WholeMachineAdmin => {
                    run_cmd(Command::new("id").arg("-u"))
                        .await
                        .map(|value| value.trim().to_string())
                }
                crate::settings::DesktopDeploymentMode::DedicatedUser => {
                    let username = self
                        .config
                        .target_username
                        .as_deref()
                        .ok_or_else(|| "missing target username".to_string())?;
                    let output = run_cmd(
                        Command::new("dscl")
                            .arg(".")
                            .arg("-read")
                            .arg(format!("/Users/{username}"))
                            .arg("UniqueID"),
                    )
                    .await?;
                    output
                        .split_whitespace()
                        .last()
                        .map(str::to_string)
                        .ok_or_else(|| "failed to parse dedicated user uid".to_string())
                }
            },
            DesktopBridgeBackend::WindowsPowerShell | DesktopBridgeBackend::LinuxPython => {
                match self.config.deployment_mode {
                    crate::settings::DesktopDeploymentMode::WholeMachineAdmin => {
                        std::env::var("USER")
                            .or_else(|_| std::env::var("USERNAME"))
                            .or_else(|_| std::env::var("LOGNAME"))
                            .map_err(|e| format!("failed to resolve interactive username: {e}"))
                    }
                    crate::settings::DesktopDeploymentMode::DedicatedUser => self
                        .config
                        .target_username
                        .clone()
                        .ok_or_else(|| "missing target username".to_string()),
                }
            }
            DesktopBridgeBackend::Unsupported => {
                Err("target session lookup is unsupported on this platform".to_string())
            }
        }
    }
}
