//! Presentation wizard steps: Web UI theming and observability.

use crate::branding::skin::CliSkin;
use crate::setup::prompts::{confirm, print_info, print_success, print_warning, select_one};
use crate::terminal_branding::set_runtime_cli_skin_override;

use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) fn step_cli_skin(&mut self) -> Result<(), SetupError> {
        print_info("Pick the skin for onboarding, the CLI, and the default web look.");
        print_info("Your choice applies immediately and can be changed later.");

        let current_skin = self.settings.agent.cli_skin.clone();
        let mut skin_names = CliSkin::available_names();
        skin_names.sort();
        if let Some(index) = skin_names.iter().position(|name| *name == current_skin) {
            let current = skin_names.remove(index);
            skin_names.insert(0, current);
        }

        let skin_options: Vec<String> = skin_names
            .iter()
            .map(|name| {
                let skin = CliSkin::load(name);
                let tagline = skin
                    .tagline()
                    .unwrap_or("No tagline available for this skin.");
                if *name == current_skin {
                    format!("{name:<12} — {tagline} [current]")
                } else {
                    format!("{name:<12} — {tagline}")
                }
            })
            .collect();
        let skin_refs: Vec<&str> = skin_options.iter().map(String::as_str).collect();
        let skin_idx =
            select_one("Choose your cockpit skin", &skin_refs).map_err(SetupError::Io)?;
        let chosen = skin_names
            .get(skin_idx)
            .cloned()
            .unwrap_or_else(|| current_skin.clone());

        self.settings.agent.cli_skin = chosen.clone();
        set_runtime_cli_skin_override(chosen.clone());
        print_success(&format!("Skin set to '{}'.", chosen));
        Ok(())
    }

    pub(super) fn step_web_ui(&mut self) -> Result<(), SetupError> {
        print_info("ThinClaw includes a web dashboard for chat, monitoring, and operator control.");
        print_info(
            "This step tunes the cockpit feel without changing the underlying runtime behavior.",
        );
        crate::setup::prompts::print_blank_line();

        if matches!(
            self.selected_profile,
            super::OnboardingProfile::RemoteServer
        ) {
            self.step_remote_web_ui_access()?;
            crate::setup::prompts::print_blank_line();
        }

        if !confirm("Customize web UI appearance?", false).map_err(SetupError::Io)? {
            print_info(
                "Keeping the default cockpit presentation: system theme, default accent color, and branding shown.",
            );
            return Ok(());
        }

        crate::setup::prompts::print_blank_line();

        let mut skin_options = vec!["Follow CLI skin".to_string()];
        skin_options.extend(CliSkin::available_names());
        let skin_refs: Vec<&str> = skin_options.iter().map(String::as_str).collect();
        let skin_idx = select_one("Web UI skin", &skin_refs).map_err(SetupError::Io)?;
        self.settings.webchat_skin = if skin_idx == 0 {
            None
        } else {
            Some(skin_options[skin_idx].clone())
        };

        // Theme selection
        let theme_options: &[&str] = &["System (follow OS preference)", "Light", "Dark"];
        let theme_idx = select_one("Theme", theme_options).map_err(SetupError::Io)?;
        let theme = match theme_idx {
            1 => "light",
            2 => "dark",
            _ => "system",
        };
        self.settings.webchat_theme = theme.to_string();

        // Branding badge
        let show_branding =
            confirm("Show \"Powered by ThinClaw\" badge?", true).map_err(SetupError::Io)?;
        self.settings.webchat_show_branding = show_branding;

        let skin_display = self
            .settings
            .webchat_skin
            .as_deref()
            .unwrap_or("follow CLI skin");
        print_success(&format!(
            "Web UI cockpit configured (skin: {}, theme: {}, branding: {})",
            skin_display,
            theme,
            if show_branding { "shown" } else { "hidden" }
        ));

        Ok(())
    }

    fn step_remote_web_ui_access(&mut self) -> Result<(), SetupError> {
        print_info("Remote WebUI access stays private by default through an SSH tunnel.");
        let options = [
            "SSH tunnel (recommended)  - bind 127.0.0.1 and forward the port from your laptop",
            "Private LAN / Tailscale   - bind 0.0.0.0 and use the token URL on a trusted network",
            "Reverse proxy / public    - keep token auth; TLS, proxy, and firewall are operator-owned",
        ];
        let choice = select_one("Remote WebUI access", &options).map_err(SetupError::Io)?;

        self.settings.channels.gateway_enabled = Some(true);
        self.settings.channels.gateway_port =
            Some(self.settings.channels.gateway_port.unwrap_or(3000));
        self.settings.channels.cli_enabled = Some(false);
        self.ensure_gateway_auth_token();

        match choice {
            1 => {
                self.settings.channels.gateway_host = Some("0.0.0.0".to_string());
                print_warning(
                    "Gateway will listen on all interfaces. Keep this to a private LAN or tailnet.",
                );
            }
            2 => {
                self.settings.channels.gateway_host = Some("127.0.0.1".to_string());
                print_warning(
                    "Reverse proxy/public exposure needs TLS, firewall rules, and proxy auth outside ThinClaw.",
                );
            }
            _ => {
                self.settings.channels.gateway_host = Some("127.0.0.1".to_string());
            }
        }

        let access = crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(
            Some(&self.settings),
        );
        print_success("Remote gateway bootstrap configured.");
        print_info(&format!("Bind: {}", access.bind_display()));
        print_info(&format!("Local URL: {}", access.local_url()));
        if access.is_loopback() {
            print_info(&format!("SSH tunnel: {}", access.ssh_tunnel_command()));
        }
        if let Some(url) = access.token_url(true) {
            print_info(&format!("Token URL: {}", url));
        }
        print_info(
            "Service handoff: run `thinclaw run --no-onboard`, or install/start the OS service after onboarding.",
        );

        Ok(())
    }

    /// Step 18: Observability configuration.
    ///
    /// Selects the event and metric recording backend.
    pub(super) fn step_observability(&mut self) -> Result<(), SetupError> {
        print_info(
            "Observability controls how much operational signal ThinClaw emits for debugging and monitoring.",
        );
        crate::setup::prompts::print_blank_line();

        let options: &[&str] = &[
            "None (no overhead, default)",
            "Log (structured events via tracing)",
        ];
        let idx = select_one("Observability backend", options).map_err(SetupError::Io)?;
        let backend = match idx {
            1 => "log",
            _ => "none",
        };
        self.settings.observability_backend = backend.to_string();

        if backend == "log" {
            print_success("Observability enabled. Events will be emitted through tracing.");
        } else {
            print_info("Observability left lean. Additional event logging will stay off for now.");
        }

        Ok(())
    }
}
