use super::*;
impl DesktopAutonomyManager {
    pub(super) async fn platform_bootstrap_prerequisites(&self) -> DesktopBootstrapPrerequisites {
        let mut checks = Vec::new();
        let mut notes = Vec::new();
        let mut blocking_reason = None;

        match self.bridge_backend() {
            DesktopBridgeBackend::MacOsSwift => {
                let app_checks = [
                    ("calendar_app", "/Applications/Calendar.app"),
                    ("numbers_app", "/Applications/Numbers.app"),
                    ("pages_app", "/Applications/Pages.app"),
                    ("textedit_app", "/Applications/TextEdit.app"),
                ];
                for (name, path) in app_checks {
                    let evidence = self.attach_runtime_evidence(
                        "bootstrap_prerequisite",
                        serde_json::json!({ "path": path }),
                    );
                    if Path::new(path).exists() {
                        checks.push(passed_check(name, None, evidence));
                    } else {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            name,
                            format!("required app missing at {path}"),
                            evidence,
                        ));
                    }
                }
            }
            DesktopBridgeBackend::WindowsPowerShell => {
                let command_checks = [
                    (
                        "outlook_com",
                        "requires_supported_apps",
                        "try { $app = New-Object -ComObject Outlook.Application; if ($null -ne $app) { $app.Quit() }; exit 0 } catch { Write-Error $_; exit 1 }",
                    ),
                    (
                        "excel_com",
                        "requires_supported_apps",
                        "try { $app = New-Object -ComObject Excel.Application; if ($null -ne $app) { $app.Quit() }; exit 0 } catch { Write-Error $_; exit 1 }",
                    ),
                    (
                        "word_com",
                        "requires_supported_apps",
                        "try { $app = New-Object -ComObject Word.Application; if ($null -ne $app) { $app.Quit() }; exit 0 } catch { Write-Error $_; exit 1 }",
                    ),
                    (
                        "notepad_app",
                        "requires_supported_apps",
                        "if (Test-Path \"$env:WINDIR\\System32\\notepad.exe\") { exit 0 } else { exit 1 }",
                    ),
                ];
                let interactive_session = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "user_interactive": std::env::var("SESSIONNAME").ok(),
                        "username": std::env::var("USERNAME").ok(),
                    }),
                );
                if std::env::var("SESSIONNAME")
                    .ok()
                    .is_some_and(|name| !name.trim().is_empty())
                {
                    checks.push(passed_check(
                        "interactive_session",
                        None,
                        interactive_session,
                    ));
                } else {
                    blocking_reason.get_or_insert_with(|| "unsupported_display_stack".to_string());
                    checks.push(failed_check(
                        "interactive_session",
                        "Windows reckless desktop requires an interactive desktop session"
                            .to_string(),
                        interactive_session,
                    ));
                }
                for (name, reason, script) in command_checks {
                    let result = run_cmd(
                        Command::new("powershell")
                            .arg("-NoLogo")
                            .arg("-NoProfile")
                            .arg("-Command")
                            .arg(script),
                    )
                    .await;
                    let evidence = self.attach_runtime_evidence(
                        "bootstrap_prerequisite",
                        serde_json::json!({ "script": script }),
                    );
                    match result {
                        Ok(_) => checks.push(passed_check(name, None, evidence)),
                        Err(err) => {
                            blocking_reason.get_or_insert_with(|| reason.to_string());
                            checks.push(failed_check(name, err, evidence));
                        }
                    }
                }
            }
            DesktopBridgeBackend::LinuxPython => {
                let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
                let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
                let known_desktop = current_desktop.split([':', ';']).any(|value| {
                    matches!(
                        value.trim().to_ascii_lowercase().as_str(),
                        "gnome"
                            | "kde"
                            | "plasma"
                            | "xfce"
                            | "lxqt"
                            | "mate"
                            | "cinnamon"
                            | "unity"
                            | "budgie"
                            | "sway"
                    )
                });
                let has_display = std::env::var_os("DISPLAY").is_some()
                    || std::env::var_os("WAYLAND_DISPLAY").is_some();
                let session_supported = matches!(
                    session_type.to_ascii_lowercase().as_str(),
                    "x11" | "wayland" | ""
                );
                let display_ok = has_display && session_supported;
                let display_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "display": std::env::var("DISPLAY").ok(),
                        "wayland_display": std::env::var("WAYLAND_DISPLAY").ok(),
                        "xdg_session_type": std::env::var("XDG_SESSION_TYPE").ok(),
                        "xdg_current_desktop": std::env::var("XDG_CURRENT_DESKTOP").ok(),
                        "known_desktop": known_desktop,
                    }),
                );
                if display_ok {
                    checks.push(passed_check("display_stack", None, display_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "unsupported_display_stack".to_string());
                    checks.push(failed_check(
                        "display_stack",
                        "Linux reckless desktop requires a logged-in desktop session with DISPLAY or WAYLAND_DISPLAY on X11 or Wayland."
                            .to_string(),
                        display_evidence,
                    ));
                }
                let dbus_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "dbus_session_bus_address": std::env::var("DBUS_SESSION_BUS_ADDRESS").ok(),
                    }),
                );
                if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some() {
                    checks.push(passed_check("dbus_session", None, dbus_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "requires_supported_apps".to_string());
                    checks.push(failed_check(
                        "dbus_session",
                        "Linux reckless desktop requires a live user D-Bus session for Evolution/EDS access"
                            .to_string(),
                        dbus_evidence,
                    ));
                }
                let app_checks = [
                    ("python3", "python3"),
                    ("libreoffice", "libreoffice"),
                    ("evolution", "evolution"),
                    ("gdbus", "gdbus"),
                ];
                for (name, command_name) in app_checks {
                    let evidence = self.attach_runtime_evidence(
                        "bootstrap_prerequisite",
                        serde_json::json!({ "command": command_name }),
                    );
                    match run_cmd(
                        Command::new("sh")
                            .arg("-lc")
                            .arg(format!("command -v {command_name}")),
                    )
                    .await
                    {
                        Ok(_) => checks.push(passed_check(name, None, evidence)),
                        Err(err) => {
                            blocking_reason
                                .get_or_insert_with(|| "requires_supported_apps".to_string());
                            checks.push(failed_check(name, err, evidence));
                        }
                    }
                }
                let input_backend = ["xdotool", "ydotool", "dotool"]
                    .iter()
                    .copied()
                    .find(|command| command_on_path(command));
                let input_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "supported_commands": ["xdotool", "ydotool", "dotool"],
                        "selected": input_backend,
                        "wayland_note": "Wayland/KDE/general desktops need ydotool or dotool for pointer actions unless running X11 with xdotool.",
                    }),
                );
                if input_backend.is_some() {
                    checks.push(passed_check("input_backend", None, input_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "requires_supported_apps".to_string());
                    checks.push(failed_check(
                        "input_backend",
                        "Linux pointer automation requires xdotool on X11, or ydotool/dotool for Wayland/KDE/general desktops."
                            .to_string(),
                        input_evidence,
                    ));
                }
                let window_backend = if command_on_path("wmctrl") {
                    Some("wmctrl")
                } else if python_module_on_path("pyatspi") {
                    Some("pyatspi")
                } else {
                    None
                };
                let window_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "supported_commands": ["wmctrl"],
                        "supported_modules": ["pyatspi"],
                        "selected": window_backend,
                    }),
                );
                if window_backend.is_some() {
                    checks.push(passed_check("window_backend", None, window_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "requires_supported_apps".to_string());
                    checks.push(failed_check(
                        "window_backend",
                        "Linux window discovery requires wmctrl on X11 or pyatspi accessibility on Wayland/general desktops."
                            .to_string(),
                        window_evidence,
                    ));
                }
                notes.push(
                    "Ubuntu/Debian desktop prerequisites: sudo apt install python3 python3-gi python3-pyatspi libreoffice libreoffice-script-provider-python evolution evolution-data-server-bin wmctrl tesseract-ocr gnome-screenshot scrot imagemagick at-spi2-core libglib2.0-bin geoclue-2.0 ffmpeg fswebcam. Add xdotool for X11, or ydotool/dotool for Wayland/KDE/general desktop pointer automation."
                        .to_string(),
                );
                for (name, module) in [("pyatspi_module", "pyatspi"), ("pygobject_module", "gi")] {
                    match run_cmd(
                        Command::new("python3")
                            .arg("-c")
                            .arg(format!("import {module}")),
                    )
                    .await
                    {
                        Ok(_) => checks.push(passed_check(
                            name,
                            None,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({ "python_module": module }),
                            ),
                        )),
                        Err(err) => {
                            blocking_reason
                                .get_or_insert_with(|| "requires_supported_apps".to_string());
                            checks.push(failed_check(
                                name,
                                err,
                                self.attach_runtime_evidence(
                                    "bootstrap_prerequisite",
                                    serde_json::json!({ "python_module": module }),
                                ),
                            ));
                        }
                    }
                }
                match run_cmd(Command::new("python3").arg("-c").arg("import uno")).await {
                    Ok(_) => checks.push(passed_check(
                        "libreoffice_uno",
                        None,
                        self.attach_runtime_evidence(
                            "bootstrap_prerequisite",
                            serde_json::json!({ "python_module": "uno" }),
                        ),
                    )),
                    Err(err) => {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            "libreoffice_uno",
                            err,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({ "python_module": "uno" }),
                            ),
                        ));
                    }
                }
                match run_cmd(Command::new("sh").arg("-lc").arg("command -v tesseract")).await {
                    Ok(_) => checks.push(passed_check(
                        "ocr_tooling",
                        None,
                        self.attach_runtime_evidence(
                            "bootstrap_prerequisite",
                            serde_json::json!({ "command": "tesseract" }),
                        ),
                    )),
                    Err(err) => {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            "ocr_tooling",
                            err,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({ "command": "tesseract" }),
                            ),
                        ));
                    }
                }
                match run_cmd(
                    Command::new("sh")
                        .arg("-lc")
                        .arg("command -v gedit || command -v xdg-text-editor"),
                )
                .await
                {
                    Ok(_) => checks.push(passed_check(
                        "generic_editor",
                        None,
                        self.attach_runtime_evidence(
                            "bootstrap_prerequisite",
                            serde_json::json!({
                                "commands": ["gedit", "xdg-text-editor"],
                                "provider": self.generic_ui_provider(),
                            }),
                        ),
                    )),
                    Err(err) => {
                        blocking_reason
                            .get_or_insert_with(|| "requires_supported_apps".to_string());
                        checks.push(failed_check(
                            "generic_editor",
                            err,
                            self.attach_runtime_evidence(
                                "bootstrap_prerequisite",
                                serde_json::json!({
                                    "commands": ["gedit", "xdg-text-editor"],
                                    "provider": self.generic_ui_provider(),
                                }),
                            ),
                        ));
                    }
                }
                let at_spi_evidence = self.attach_runtime_evidence(
                    "bootstrap_prerequisite",
                    serde_json::json!({
                        "at_spi_bus_address": std::env::var("AT_SPI_BUS_ADDRESS").ok(),
                        "gtk_modules": std::env::var("GTK_MODULES").ok(),
                    }),
                );
                if std::env::var_os("AT_SPI_BUS_ADDRESS").is_some()
                    || std::env::var_os("GTK_MODULES")
                        .is_some_and(|value| value.to_string_lossy().contains("gail"))
                {
                    checks.push(passed_check("accessibility_bus", None, at_spi_evidence));
                } else {
                    blocking_reason.get_or_insert_with(|| "requires_supported_apps".to_string());
                    checks.push(failed_check(
                        "accessibility_bus",
                        "Linux reckless desktop requires an active AT-SPI accessibility session"
                            .to_string(),
                        at_spi_evidence,
                    ));
                }
            }
            DesktopBridgeBackend::Unsupported => {
                blocking_reason = Some("unsupported_display_stack".to_string());
                checks.push(failed_check(
                    "bridge_backend",
                    "desktop autonomy bridge is unsupported on this platform".to_string(),
                    self.attach_runtime_evidence("bootstrap_prerequisite", serde_json::json!({})),
                ));
            }
        }

        DesktopBootstrapPrerequisites {
            passed: checks.iter().all(|check| check.passed),
            blocking_reason,
            evidence: self.attach_runtime_evidence(
                "bootstrap_prerequisites",
                serde_json::json!({ "check_count": checks.len() }),
            ),
            checks,
            notes,
        }
    }

    pub(super) async fn prepare_dedicated_user_bootstrap(
        &self,
    ) -> Result<DedicatedUserBootstrap, String> {
        if self.config.deployment_mode != crate::settings::DesktopDeploymentMode::DedicatedUser {
            return Ok(DedicatedUserBootstrap {
                session_ready: true,
                ..Default::default()
            });
        }

        let username = self
            .config
            .target_username
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                "dedicated_user deployment requires desktop_autonomy.target_username".to_string()
            })?;
        let keychain_label = format!("ThinClaw Desktop Autonomy/{username}");

        let mut bootstrap = DedicatedUserBootstrap {
            keychain_label: Some(keychain_label.clone()),
            ..Default::default()
        };

        let exists = self.user_exists(&username).await?;
        if !exists {
            if !self.has_privileged_bootstrap().await {
                bootstrap.blocking_reason =
                    Some(dedicated_bootstrap_blocking_reason(false, false, false).to_string());
                return Ok(bootstrap);
            }

            let password = generate_dedicated_user_secret();
            self.create_dedicated_user(&username, &password).await?;
            if let Err(error) =
                crate::platform::secure_store::store_api_key(&keychain_label, &password).await
            {
                tracing::warn!(
                    error = %error,
                    "failed to store dedicated-user password in secure store; returning one-time login secret in bootstrap report"
                );
                bootstrap.keychain_label = None;
            }
            bootstrap.created_user = true;
            bootstrap.one_time_login_secret = Some(password);
        }

        let session_subject = self.target_session_subject().await?;
        bootstrap.session_ready = self
            .gui_session_ready(&session_subject, Some(&username))
            .await;
        if !bootstrap.session_ready {
            bootstrap.blocking_reason =
                Some(dedicated_bootstrap_blocking_reason(true, true, false).to_string());
        }

        Ok(bootstrap)
    }
}
