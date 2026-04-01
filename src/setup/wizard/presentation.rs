//! Presentation wizard steps: Web UI theming and observability.

use crate::setup::prompts::{confirm, optional_input, print_info, print_success, select_one};

use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) fn step_web_ui(&mut self) -> Result<(), SetupError> {
        print_info("ThinClaw includes a web dashboard (gateway UI) for chat and monitoring.");
        print_info("You can customize its appearance here.");
        println!();

        if !confirm("Customize web UI appearance?", false).map_err(SetupError::Io)? {
            print_info("Using defaults (system theme, default accent color, branding shown).");
            return Ok(());
        }

        println!();

        // Theme selection
        let theme_options: &[&str] = &["System (follow OS preference)", "Light", "Dark"];
        let theme_idx = select_one("Theme", theme_options).map_err(SetupError::Io)?;
        let theme = match theme_idx {
            1 => "light",
            2 => "dark",
            _ => "system",
        };
        self.settings.webchat_theme = theme.to_string();

        // Accent color
        let accent = optional_input(
            "Accent color (hex, e.g. #22c55e)",
            Some("leave blank for default"),
        )
        .map_err(SetupError::Io)?;
        if let Some(ref color) = accent {
            if !color.is_empty() {
                self.settings.webchat_accent_color = Some(color.clone());
            }
        }

        // Branding badge
        let show_branding =
            confirm("Show \"Powered by ThinClaw\" badge?", true).map_err(SetupError::Io)?;
        self.settings.webchat_show_branding = show_branding;

        let accent_display = self
            .settings
            .webchat_accent_color
            .as_deref()
            .unwrap_or("default");
        print_success(&format!(
            "Web UI configured (theme: {}, accent: {}, branding: {})",
            theme,
            accent_display,
            if show_branding { "shown" } else { "hidden" }
        ));

        Ok(())
    }

    /// Step 18: Observability configuration.
    ///
    /// Selects the event and metric recording backend.
    pub(super) fn step_observability(&mut self) -> Result<(), SetupError> {
        print_info("Observability records events and metrics for debugging and monitoring.");
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
            print_success("Observability enabled — events will be emitted via tracing.");
        } else {
            print_info("Observability disabled — zero overhead mode.");
        }

        Ok(())
    }
}
