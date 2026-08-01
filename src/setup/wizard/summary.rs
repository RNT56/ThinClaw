//! Final wizard summary: save settings and print configuration overview.

use crate::config::resolve_personality_pack_from_settings;
use crate::settings::KeySource;
use crate::setup::prompts::{
    PromptUiMode as PromptRenderMode, current_prompt_ui_mode, print_info, print_success,
    print_warning,
};

use super::helpers::capitalize_first;
use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) async fn save_and_summarize(&mut self) -> Result<(), SetupError> {
        self.persist_followups();
        self.settings.onboard_completed = true;

        // Final persist (idempotent — earlier incremental saves already wrote
        // most settings, but this ensures onboard_completed is saved).
        let saved = self.persist_settings().await?;

        if !saved {
            return Err(SetupError::Database(
                "No database connection, cannot save settings".to_string(),
            ));
        }

        // Write bootstrap env (also idempotent)
        self.write_bootstrap_env()?;

        if current_prompt_ui_mode() == PromptRenderMode::Tui {
            print_success("Configuration saved to database");
            print_info(&self.runtime_handoff_summary());
            self.report_path_status();
            return Ok(());
        }

        println!();
        print_success("Configuration saved to database");
        println!();

        let readiness = self.readiness_summary();
        println!("Ready to Use");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Status: {}", readiness.headline);
        println!(
            "  Readiness: {} ready · {} attention · {} follow-ups",
            readiness.ready_now, readiness.needs_attention, readiness.followups
        );

        let backend = self
            .settings
            .database_backend
            .as_deref()
            .unwrap_or("postgres");
        match backend {
            "libsql" => {
                if let Some(ref path) = self.settings.libsql_path {
                    println!("  Database: libSQL ({})", path);
                } else {
                    println!("  Database: libSQL (default path)");
                }
                if self.settings.libsql_url.is_some() {
                    println!("  Turso sync: enabled");
                }
            }
            _ => {
                if self.settings.database_url.is_some() {
                    println!("  Database: PostgreSQL (configured)");
                }
            }
        }

        match self.settings.secrets_master_key_source {
            KeySource::Keychain => println!(
                "  Security: {}",
                crate::platform::secure_store::display_name()
            ),
            KeySource::Env => println!("  Security: environment variable"),
            KeySource::None => println!("  Security: disabled"),
        }

        if let Some(ref provider) = self.settings.llm_backend {
            let display = match provider.as_str() {
                "anthropic" => "Anthropic",
                "openai" => "OpenAI",
                "ollama" => "Ollama",
                "openai_compatible" => "OpenAI-compatible",
                other => other,
            };
            println!("  AI Provider: {}", display);
        }

        if let Some(ref model) = self.settings.selected_model {
            // Truncate long model names (char-based to avoid UTF-8 panic)
            let display = if model.chars().count() > 40 {
                let truncated: String = model.chars().take(37).collect();
                format!("{}...", truncated)
            } else {
                model.clone()
            };
            println!("  Primary Model: {}", display);
        }

        if self.settings.embeddings.enabled {
            println!(
                "  Semantic Search: {} ({})",
                self.settings.embeddings.provider, self.settings.embeddings.model
            );
        } else {
            println!("  Semantic Search: disabled");
        }

        if let Some(ref tunnel_url) = self.settings.tunnel.public_url {
            println!("  Tunnel: {} (static)", tunnel_url);
        } else if let Some(ref provider) = self.settings.tunnel.provider {
            println!("  Tunnel: {} (managed, starts at boot)", provider);
        }

        let has_tunnel =
            self.settings.tunnel.public_url.is_some() || self.settings.tunnel.provider.is_some();

        println!("  Channels:");
        let cli_enabled = self.settings.channels.cli_enabled.unwrap_or(true);
        println!(
            "    - CLI/TUI: {}",
            if cli_enabled { "enabled" } else { "disabled" }
        );
        if self.settings.channels.gateway_enabled.unwrap_or(true) {
            let access = crate::platform::gateway_access::GatewayAccessInfo::from_env_and_settings(
                Some(&self.settings),
            );
            println!("    - Web Gateway: enabled ({})", access.bind_display());
            println!("      Web UI: {}", access.local_url());
            if access.is_loopback() {
                println!("      SSH tunnel: {}", access.ssh_tunnel_command());
            }
        }

        if self.settings.channels.http_enabled {
            let port = self.settings.channels.http_port.unwrap_or(8080);
            println!("    - HTTP: enabled (port {})", port);
        }

        if self.settings.channels.signal_enabled {
            println!("    - Signal: enabled");
        }

        if self.settings.channels.discord_enabled {
            println!("    - Discord: enabled");
        }

        if self.settings.channels.slack_enabled {
            println!("    - Slack: enabled");
        }

        if self.settings.channels.nostr_enabled {
            println!("    - Nostr: enabled");
        }

        if self.settings.channels.gmail_enabled {
            println!("    - Gmail: enabled");
        }

        #[cfg(target_os = "macos")]
        if self.settings.channels.imessage_enabled {
            println!("    - iMessage: enabled");
        }

        #[cfg(target_os = "macos")]
        if self.settings.channels.apple_mail_enabled {
            println!("    - Apple Mail: enabled");
        }

        if self.settings.channels.bluebubbles_enabled {
            println!("    - BlueBubbles (iMessage): enabled");
        }

        for channel_name in &self.settings.channels.wasm_channels {
            let mode = if has_tunnel { "webhook" } else { "polling" };
            println!(
                "    - {}: enabled ({})",
                capitalize_first(channel_name),
                mode
            );
        }

        println!("  Agent: {}", self.settings.agent.name);
        let effective_pack = resolve_personality_pack_from_settings(&self.settings);
        println!("  Personality Pack: {}", effective_pack);
        println!("  CLI Skin: {}", self.settings.agent.cli_skin);

        if let Some(ref tz) = self.settings.user_timezone {
            println!("  Timezone: {}", tz);
        }

        if let Some(ref cheap_model) = self.settings.providers.cheap_model {
            println!(
                "  Routing: {} ({})",
                self.settings.providers.routing_mode.as_str(),
                cheap_model
            );
        } else {
            println!(
                "  Routing: {}",
                self.settings.providers.routing_mode.as_str()
            );
        }

        if self.settings.heartbeat.enabled {
            println!(
                "  Heartbeat: every {} minutes",
                self.settings.heartbeat.interval_secs / 60
            );
        }

        if self.settings.routines_enabled {
            println!("  Routines: enabled");
        }

        if self.settings.skills_enabled {
            println!("  Skills: enabled");
        }

        if self.settings.claude_code_enabled {
            let default_claude_model = crate::config::ClaudeCodeConfig::default().model;
            let model = self
                .settings
                .claude_code_model
                .as_deref()
                .unwrap_or(default_claude_model.as_str());
            println!("  Claude Code: enabled (model: {})", model);
        }

        if self.settings.codex_code_enabled {
            let model = self
                .settings
                .codex_code_model
                .as_deref()
                .unwrap_or("gpt-5.3-codex");
            println!("  Codex: enabled (model: {})", model);
        }

        if self.settings.webchat_skin.is_some()
            || self.settings.webchat_theme != "system"
            || self.settings.webchat_accent_color.is_some()
            || !self.settings.webchat_show_branding
        {
            let skin_mode = self
                .settings
                .webchat_skin
                .as_deref()
                .unwrap_or("follow CLI skin");
            let mut summary = format!(
                "  Web UI: skin={}, theme={}, branding={}",
                skin_mode,
                self.settings.webchat_theme,
                if self.settings.webchat_show_branding {
                    "shown"
                } else {
                    "hidden"
                }
            );
            if let Some(accent) = self.settings.webchat_accent_color.as_deref() {
                summary.push_str(&format!(", accent override={}", accent));
            }
            println!("{}", summary);
        }

        if self.settings.observability_backend != "none" {
            println!("  Observability: {}", self.settings.observability_backend);
        } else {
            println!("  Observability: disabled");
        }

        println!();

        println!("Needs Attention");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        if self.followups.is_empty() {
            print_success("No follow-up items were deferred.");
        } else {
            for followup in &self.followups {
                print_warning(&format!("{} — {}", followup.title, followup.instructions));
                if let Some(ref hint) = followup.action_hint {
                    print_info(hint);
                }
            }
        }
        println!();

        println!("What Happens Next");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        print_info(&self.runtime_handoff_summary());
        if self.should_continue_to_runtime() {
            print_info(
                "There is no second setup loop here; the runtime uses these settings directly.",
            );
        } else {
            print_info(
                "This was a settings pass only, so runtime stays paused until you launch it yourself.",
            );
        }
        println!();

        // ── PATH check & symlink offer ──────────────────────────
        // If the current binary isn't on PATH, offer to create a symlink so
        // the user can just type `thinclaw` from any terminal.
        self.report_path_status();

        println!("Resume Later");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        for command in self.what_next_commands() {
            println!("  {}", command);
        }
        println!("  thinclaw config set <setting> <value>");
        println!();

        Ok(())
    }

    /// Report PATH state without mutating user files or invoking elevation.
    pub(super) fn report_path_status(&self) {
        if which_thinclaw().is_some() {
            return;
        }

        let Ok(current_exe) = std::env::current_exe() else {
            return;
        };
        let parent = current_exe
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "the ThinClaw installation directory".to_string());
        print_info(&format!(
            "ThinClaw is not on PATH. Continue with `{}` or reinstall it through your package manager. To manage PATH yourself, add `{parent}` explicitly.",
            current_exe.display()
        ));
    }
}

impl Default for SetupWizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if `thinclaw` is findable on PATH by scanning PATH directories.
fn which_thinclaw() -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("thinclaw");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}
