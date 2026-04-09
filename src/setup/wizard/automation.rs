//! Automation wizard steps: routines, skills, heartbeat.

use crate::setup::prompts::{confirm, optional_input, print_info, print_success};

use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) fn step_routines(&mut self) -> Result<(), SetupError> {
        print_info("Routines let ThinClaw execute scheduled tasks automatically.");
        print_info("Examples: periodic file backups, daily summaries, cron-style jobs.");
        println!();

        if !confirm("Enable routines?", true).map_err(SetupError::Io)? {
            self.settings.routines_enabled = false;
            print_info("Routines disabled. Enable later with ROUTINES_ENABLED=true.");
            return Ok(());
        }

        self.settings.routines_enabled = true;
        print_success("Routines enabled");
        Ok(())
    }

    /// Step 14: Skills.
    pub(super) fn step_skills(&mut self) -> Result<(), SetupError> {
        print_info("Skills are composable capability plugins that give ThinClaw");
        print_info("domain-specific knowledge (e.g., coding standards, deployment");
        print_info("procedures). They are loaded from ~/.thinclaw/skills/.");
        println!();

        if !confirm("Enable skills system?", true).map_err(SetupError::Io)? {
            self.settings.skills_enabled = false;
            print_info("Skills disabled. Enable later with SKILLS_ENABLED=true.");
            return Ok(());
        }

        self.settings.skills_enabled = true;
        print_success("Skills system enabled");
        Ok(())
    }

    /// Step 11: Claude Code sandbox.
    pub(super) fn step_heartbeat(&mut self) -> Result<(), SetupError> {
        print_info("Heartbeat runs periodic background tasks (e.g., checking your calendar,");
        print_info("monitoring for notifications, running scheduled workflows).");
        println!();

        if !confirm("Enable heartbeat?", false).map_err(SetupError::Io)? {
            self.settings.heartbeat.enabled = false;
            print_info("Heartbeat disabled.");
            return Ok(());
        }

        self.settings.heartbeat.enabled = true;

        // Interval
        let interval_str = optional_input("Check interval in minutes", Some("default: 30"))
            .map_err(SetupError::Io)?;

        if let Some(s) = interval_str {
            if let Ok(mins) = s.parse::<u64>() {
                self.settings.heartbeat.interval_secs = mins * 60;
            }
        } else {
            self.settings.heartbeat.interval_secs = 1800; // 30 minutes
        }

        // Notification channel is configured in step 16 (Notification Preferences)
        // which handles recipient selection for all proactive messages.

        print_success(&format!(
            "Heartbeat enabled (every {} minutes)",
            self.settings.heartbeat.interval_secs / 60
        ));

        Ok(())
    }
}
