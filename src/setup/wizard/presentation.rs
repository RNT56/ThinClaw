//! Presentation wizard steps: Web UI theming and observability.

use crate::branding::skin::CliSkin;
use crate::setup::prompts::{confirm, print_info, print_success, select_one};

use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) fn step_web_ui(&mut self) -> Result<(), SetupError> {
        print_info("ThinClaw includes a web dashboard for chat, monitoring, and operator control.");
        print_info(
            "This step tunes the cockpit feel without changing the underlying runtime behavior.",
        );
        println!();

        if !confirm("Customize web UI appearance?", false).map_err(SetupError::Io)? {
            print_info(
                "Keeping the default cockpit presentation: system theme, default accent color, and branding shown.",
            );
            return Ok(());
        }

        println!();

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

    /// Step 18: Observability configuration.
    ///
    /// Selects the event and metric recording backend.
    pub(super) fn step_observability(&mut self) -> Result<(), SetupError> {
        print_info(
            "Observability controls how much operational signal ThinClaw emits for debugging and monitoring.",
        );
        println!();

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
