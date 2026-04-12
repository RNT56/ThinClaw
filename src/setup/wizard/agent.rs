//! Agent personality wizard steps: identity, tool approval, notification preferences.

use crate::setup::prompts::{
    confirm, input, optional_input, print_info, print_success, select_one,
};

use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) fn step_agent_identity(&mut self) -> Result<(), SetupError> {
        print_info("Give your ThinClaw agent a name. This is used in greetings,");
        print_info("the boot screen, and session metadata.");
        println!();

        let current = &self.settings.agent.name;
        let default_label = format!("current: {}", current);
        let name = optional_input("Agent name", Some(&default_label)).map_err(SetupError::Io)?;

        if let Some(n) = name {
            if !n.is_empty() {
                self.settings.agent.name = n.clone();
                print_success(&format!("Agent name set to '{}'", n));
            } else {
                print_success(&format!("Keeping '{}'", current));
            }
        } else {
            print_success(&format!("Keeping '{}'", current));
        }

        Ok(())
    }

    /// Timezone detection and confirmation.
    ///
    /// Auto-detects the system timezone via `iana_time_zone` and asks the
    /// user to confirm. If the detection is wrong (e.g. VPS in UTC but user
    /// is in Europe/Berlin), the user can enter a different IANA timezone.
    ///
    /// The confirmed timezone is stored in `Settings.user_timezone` and used
    /// by heartbeat active hours, routine scheduling, cost dashboard day
    /// boundaries, and the boot greeting's time-of-day awareness.
    pub(super) fn step_timezone(&mut self) -> Result<(), SetupError> {
        let detected = crate::timezone::detect_system_timezone();
        let detected_str = detected.to_string();

        // If already set from a previous run, show the current value
        if let Some(ref existing) = self.settings.user_timezone
            && !existing.is_empty()
            && existing != "UTC"
        {
            print_info(&format!("Current timezone: {}", existing));
            if confirm(&format!("Keep '{}'?", existing), true).map_err(SetupError::Io)? {
                print_success(&format!("Timezone: {}", existing));
                return Ok(());
            }
        }

        print_info(&format!("Detected system timezone: {}", detected_str));

        if confirm("Is this correct?", true).map_err(SetupError::Io)? {
            self.settings.user_timezone = Some(detected_str.clone());
            print_success(&format!("Timezone set to '{}'", detected_str));
        } else {
            print_info(
                "Enter your timezone as an IANA name (e.g. America/New_York, Europe/Berlin, Asia/Tokyo).",
            );
            let tz_input = input("Timezone").map_err(SetupError::Io)?;
            if tz_input.is_empty() {
                self.settings.user_timezone = Some(detected_str.clone());
                print_success(&format!("Keeping detected timezone '{}'", detected_str));
            } else if crate::timezone::parse_timezone(&tz_input).is_some() {
                self.settings.user_timezone = Some(tz_input.clone());
                print_success(&format!("Timezone set to '{}'", tz_input));
            } else {
                print_info(&format!(
                    "'{}' is not a valid IANA timezone. Using detected '{}'.",
                    tz_input, detected_str
                ));
                self.settings.user_timezone = Some(detected_str);
            }
        }

        Ok(())
    }

    /// Step 13: Routines (scheduled tasks).
    pub(super) async fn step_notification_preferences(&mut self) -> Result<(), SetupError> {
        print_info("ThinClaw sends proactive notifications (heartbeat alerts, routine results,");
        print_info("self-repair messages) to a channel of your choice.");
        println!();

        let secrets = self.init_secrets_context().await.ok();
        let discord_has_token = self
            .settings
            .channels
            .discord_bot_token
            .as_ref()
            .is_some_and(|token| !token.trim().is_empty())
            || std::env::var("DISCORD_BOT_TOKEN")
                .ok()
                .is_some_and(|token| !token.trim().is_empty())
            || if let Some(ref ctx) = secrets {
                ctx.secret_exists("discord_bot_token").await
            } else {
                false
            };
        let slack_has_bot_token = self
            .settings
            .channels
            .slack_bot_token
            .as_ref()
            .is_some_and(|token| !token.trim().is_empty())
            || std::env::var("SLACK_BOT_TOKEN")
                .ok()
                .is_some_and(|token| !token.trim().is_empty())
            || if let Some(ref ctx) = secrets {
                ctx.secret_exists("slack_bot_token").await
            } else {
                false
            };
        let slack_has_app_token = self
            .settings
            .channels
            .slack_app_token
            .as_ref()
            .is_some_and(|token| !token.trim().is_empty())
            || std::env::var("SLACK_APP_TOKEN")
                .ok()
                .is_some_and(|token| !token.trim().is_empty())
            || if let Some(ref ctx) = secrets {
                ctx.secret_exists("slack_app_token").await
            } else {
                false
            };
        let is_ready = |channel: &str, fallback: bool| {
            self.verified_channels
                .get(channel)
                .copied()
                .unwrap_or(fallback)
        };

        // Collect configured channels
        let mut channels: Vec<String> = Vec::new();
        channels.push("web".to_string()); // Always available
        // Telegram is a WASM channel — detected by owner binding or wasm_channels list
        if self.settings.channels.telegram_owner_id.is_some()
            || self
                .settings
                .channels
                .wasm_channels
                .iter()
                .any(|c| c == "telegram")
        {
            channels.push("telegram".to_string());
        }
        if self.settings.channels.imessage_enabled {
            channels.push("imessage".to_string());
        }
        if self.settings.channels.apple_mail_enabled {
            channels.push("apple_mail".to_string());
        }
        if self.settings.channels.signal_enabled
            && is_ready("signal", self.settings.channels.signal_account.is_some())
        {
            channels.push("signal".to_string());
        }
        if self.settings.channels.discord_enabled && is_ready("discord", discord_has_token) {
            channels.push("discord".to_string());
        }
        if self.settings.channels.slack_enabled
            && is_ready("slack", slack_has_bot_token && slack_has_app_token)
        {
            channels.push("slack".to_string());
        }
        if self.settings.channels.nostr_enabled
            && is_ready("nostr", self.settings.channels.nostr_relays.is_some())
        {
            channels.push("nostr".to_string());
        }

        if channels.len() == 1 {
            // Only web — no external channels configured
            print_info("Only the web channel is configured.");
            print_info("Notifications will appear in the Web UI.");
            self.settings.notifications.preferred_channel = Some("web".to_string());
            self.settings.notifications.recipient = Some("default".to_string());
            return Ok(());
        }

        if channels.len() == 2 {
            // Exactly one external channel — auto-select it
            let ch = channels[1].clone(); // Skip "web"
            print_info(&format!(
                "Auto-selecting '{}' as your notification channel (only external channel configured).",
                ch
            ));
            self.settings.notifications.preferred_channel = Some(ch.clone());
            self.collect_notification_recipient(&ch)?;
            return Ok(());
        }

        // Multiple channels — ask user to pick
        let options: Vec<String> = channels
            .iter()
            .map(|ch| match ch.as_str() {
                "web" => "web       — Web UI only (always available)".to_string(),
                "telegram" => "telegram  — Telegram bot messages".to_string(),
                "imessage" => "imessage    — iMessage (macOS)".to_string(),
                "apple_mail" => "apple_mail  — Apple Mail (macOS)".to_string(),
                "signal" => "signal    — Signal messenger".to_string(),
                "discord" => "discord   — Discord bot".to_string(),
                "slack" => "slack     — Slack workspace".to_string(),
                "nostr" => "nostr     — Nostr relay".to_string(),
                other => other.to_string(),
            })
            .collect();

        let option_strs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
        let choice = select_one("Which channel for proactive notifications?", &option_strs)
            .map_err(SetupError::Io)?;

        let selected = channels[choice].clone();
        self.settings.notifications.preferred_channel = Some(selected.clone());

        if selected != "web" {
            self.collect_notification_recipient(&selected)?;
        } else {
            self.settings.notifications.recipient = Some("default".to_string());
        }

        print_success(&format!("Notifications will be sent via '{}'", selected));
        print_info("You can change this later in Settings > Notifications.");

        Ok(())
    }

    /// Collect the recipient identifier for a given notification channel.
    pub(super) fn collect_notification_recipient(
        &mut self,
        channel: &str,
    ) -> Result<(), SetupError> {
        match channel {
            "telegram" => {
                // Auto-populate from Telegram owner binding
                if let Some(owner_id) = self.settings.channels.telegram_owner_id {
                    print_info(&format!("Telegram owner detected (ID: {}).", owner_id));
                    if confirm("Use this account for notifications?", true)
                        .map_err(SetupError::Io)?
                    {
                        self.settings.notifications.recipient = Some(owner_id.to_string());
                        return Ok(());
                    }
                }
                let id = input("Telegram chat ID (numeric)").map_err(SetupError::Io)?;
                if !id.is_empty() {
                    self.settings.notifications.recipient = Some(id);
                }
            }
            "imessage" => {
                print_info("Enter your phone number or Apple ID for iMessage notifications.");
                let contact = input("Phone number or Apple ID (e.g., +4917612345678)")
                    .map_err(SetupError::Io)?;
                if !contact.is_empty() {
                    self.settings.notifications.recipient = Some(contact);
                } else {
                    print_info("No recipient set — iMessage notifications disabled.");
                    self.settings.notifications.preferred_channel = Some("web".to_string());
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            "apple_mail" => {
                print_info("Enter your email address for Apple Mail notifications.");
                let email = input("Email address").map_err(SetupError::Io)?;
                if !email.is_empty() {
                    self.settings.notifications.recipient = Some(email);
                } else {
                    print_info("No recipient set — Apple Mail notifications disabled.");
                    self.settings.notifications.preferred_channel = Some("web".to_string());
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            "signal" => {
                print_info("Enter your phone number for Signal notifications.");
                let number = input("Phone number (E.164 format, e.g., +4917612345678)")
                    .map_err(SetupError::Io)?;
                if !number.is_empty() {
                    self.settings.notifications.recipient = Some(number);
                } else {
                    print_info("No recipient set — Signal notifications disabled.");
                    self.settings.notifications.preferred_channel = Some("web".to_string());
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            "discord" => {
                print_info("Enter your Discord user ID for notifications.");
                let id = input("Discord user ID").map_err(SetupError::Io)?;
                if !id.is_empty() {
                    self.settings.notifications.recipient = Some(id);
                } else {
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            _ => {
                self.settings.notifications.recipient = Some("default".to_string());
            }
        }
        Ok(())
    }

    /// Step 12: Tool approval mode.
    pub(super) fn step_tool_approval(&mut self) -> Result<(), SetupError> {
        print_info(
            "ThinClaw can execute tools (shell commands, file operations, etc.) on your behalf.",
        );
        print_info("Choose how much autonomy to grant the agent:");
        println!();

        let options = [
            "Standard  — Ask before running destructive operations (recommended)",
            "Autonomous — Auto-approve safe operations, still block destructive commands\n               (rm -rf, DROP TABLE, git push --force, etc.)",
            "Full Auto  — Skip ALL approval checks (for benchmarks/CI only)\n               ⚠️  WARNING: The agent can execute ANY command without asking!",
        ];
        let option_refs: Vec<&str> = options.to_vec();
        let choice = select_one("Tool approval mode", &option_refs).map_err(SetupError::Io)?;

        match choice {
            0 => {
                self.settings.agent.auto_approve_tools = false;
                // Standard mode: keep sandboxed workspace (default)
                print_success(
                    "Standard approval mode — agent will ask before destructive operations.",
                );
            }
            1 => {
                self.settings.agent.auto_approve_tools = true;
                // Autonomous mode: grant unrestricted filesystem/shell access.
                // Without this, the system prompt says "sandboxed" and the LLM
                // halluccinates restrictions (e.g. refusing to run osascript, ps).
                self.settings.agent.workspace_mode = Some("unrestricted".to_string());
                print_success(
                    "Autonomous mode — safe operations auto-approved, destructive commands still blocked.",
                );
                print_info(
                    "Note: Commands matching NEVER_AUTO_APPROVE_PATTERNS (rm -rf, DROP TABLE, etc.)",
                );
                print_info("will still require your approval even in this mode.");
            }
            2 => {
                self.settings.agent.auto_approve_tools = true;
                // Full auto: grant unrestricted filesystem/shell access.
                self.settings.agent.workspace_mode = Some("unrestricted".to_string());
                print_success(
                    "Full auto-approve mode — ALL tool executions will run without asking.",
                );
                print_info(
                    "⚠️  Use with extreme caution. This is intended for benchmarks/CI environments.",
                );
            }
            _ => {
                self.settings.agent.auto_approve_tools = false;
            }
        }
        Ok(())
    }
}
