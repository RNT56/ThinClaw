//! Automation wizard steps: routines, skills, heartbeat.

use crate::setup::prompts::{confirm, optional_input, print_info, print_success};

use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) fn step_routines(&mut self) -> Result<(), SetupError> {
        print_info("Routines let ThinClaw run scheduled work for you.");
        print_info("Examples: backups, daily summaries, and cron-style jobs.");
        print_info("Recommended: keep routines on unless you want a very minimal setup.");
        println!();

        if !confirm("Enable routines?", true).map_err(SetupError::Io)? {
            self.settings.routines_enabled = false;
            print_info("Routines disabled. You can turn them back on with ROUTINES_ENABLED=true.");
            return Ok(());
        }

        self.settings.routines_enabled = true;
        print_success("Routines enabled");
        Ok(())
    }

    /// Step 14: Skills.
    pub(super) fn step_skills(&mut self) -> Result<(), SetupError> {
        print_info("Skills are reusable capability packs that add domain knowledge.");
        print_info(
            "Examples include coding standards, deployment steps, and project-specific rules.",
        );
        print_info("They load from ~/.thinclaw/skills/.");
        print_info("Recommended: keep skills on unless you want a very minimal local runtime.");
        println!();

        if !confirm("Enable skills system?", true).map_err(SetupError::Io)? {
            self.settings.skills_enabled = false;
            print_info("Skills disabled. You can turn them back on with SKILLS_ENABLED=true.");
            return Ok(());
        }

        self.settings.skills_enabled = true;
        print_success("Skills system enabled");
        Ok(())
    }

    /// Step 11: Claude Code sandbox.
    pub(super) fn step_heartbeat(&mut self) -> Result<(), SetupError> {
        print_info(
            "Heartbeat runs scheduled background tasks like calendar checks and notifications.",
        );
        if matches!(
            self.selected_profile,
            super::OnboardingProfile::ChannelFirst
        ) {
            print_info(
                "Recommended for this profile: wait to enable heartbeat until your notification channel is ready.",
            );
        } else if matches!(
            self.selected_profile,
            super::OnboardingProfile::CustomAdvanced
        ) {
            print_info(
                "Custom / Advanced keeps heartbeat opt-in. Leave it off until your channels, routines, and notification routing are exactly how you want them.",
            );
        } else {
            print_info(
                "Recommended: keep heartbeat off for day one unless you already know where alerts should go.",
            );
        }
        println!();

        let default_enabled = matches!(
            self.selected_profile,
            super::OnboardingProfile::ChannelFirst
        ) && self.settings.notifications.preferred_channel.is_some();
        if !confirm("Enable heartbeat?", default_enabled).map_err(SetupError::Io)? {
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
