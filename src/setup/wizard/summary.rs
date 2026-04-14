//! Final wizard summary: save settings and print configuration overview.

use crate::settings::KeySource;
use crate::setup::prompts::{confirm, print_info, print_success, print_warning};

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

        println!();
        print_success("Configuration saved to database");
        println!();

        let readiness = self.readiness_summary();
        println!("Ready to Use");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("  Status: {}", readiness.headline);

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
            KeySource::Keychain => println!("  Security: OS keychain"),
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
        println!("    - CLI/TUI: enabled");

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

        for channel_name in &self.settings.channels.wasm_channels {
            let mode = if has_tunnel { "webhook" } else { "polling" };
            println!(
                "    - {}: enabled ({})",
                capitalize_first(channel_name),
                mode
            );
        }

        println!("  Agent: {}", self.settings.agent.name);
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
            let model = self
                .settings
                .claude_code_model
                .as_deref()
                .unwrap_or("sonnet");
            println!("  Claude Code: enabled (model: {})", model);
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
        print_info("ThinClaw will start normally with the settings you just reviewed.");
        print_info("There is no second setup loop here; the runtime uses these settings directly.");
        println!();

        // ── PATH check & symlink offer ──────────────────────────
        // If the current binary isn't on PATH, offer to create a symlink so
        // the user can just type `thinclaw` from any terminal.
        self.offer_path_setup();

        println!("Resume Later");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("To start ThinClaw, run:");
        println!("  thinclaw");
        println!();
        println!("To update settings later:");
        println!("  thinclaw config set <setting> <value>");
        println!("  thinclaw onboard");
        println!();

        Ok(())
    }

    /// Check if `thinclaw` is accessible on PATH and offer to create a
    /// symlink if it isn't.
    pub(super) fn offer_path_setup(&self) {
        use std::path::Path;

        // Check if `thinclaw` is already findable on PATH
        if which_thinclaw().is_some() {
            return; // Already on PATH, nothing to do
        }

        let current_exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(_) => return, // Can't determine our own path
        };

        // Choose symlink target based on platform
        let symlink_dir = if cfg!(target_os = "macos") {
            Path::new("/usr/local/bin")
        } else {
            // Linux: ~/.local/bin is in PATH for most distros
            let home = match dirs::home_dir() {
                Some(h) => h,
                None => return,
            };
            // We need a 'static-ish path, so use a leak-safe approach
            let local_bin = home.join(".local").join("bin");
            if !local_bin.exists() {
                let _ = std::fs::create_dir_all(&local_bin);
            }
            // Can't return a reference to a local, so handle inline below
            let target = local_bin.join("thinclaw");
            if try_symlink(&current_exe, &target) {
                print_success(&format!(
                    "Symlinked: {} → {}",
                    target.display(),
                    current_exe.display()
                ));
                println!("  You can now use 'thinclaw' from any terminal.");
                if !path_contains(&local_bin) {
                    println!(
                        "  Note: add {} to your PATH if it isn't already:",
                        local_bin.display()
                    );
                    println!(
                        "    echo 'export PATH=\"{}:$PATH\"' >> ~/.bashrc",
                        local_bin.display()
                    );
                }
            } else {
                println!();
                print_info(&format!(
                    "Tip: add thinclaw to your PATH:\n  \
                 sudo ln -sf {} /usr/local/bin/thinclaw\n  \
                 Or export PATH=\"{}:$PATH\"",
                    current_exe.display(),
                    current_exe
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ));
            }
            return;
        };

        let target = symlink_dir.join("thinclaw");

        if !symlink_dir.exists() {
            // /usr/local/bin doesn't exist (rare on macOS), just print a tip
            print_info(&format!(
                "Tip: add thinclaw to your PATH:\n  \
                 export PATH=\"{}:$PATH\"",
                current_exe
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            ));
            return;
        }

        // Try without sudo first (works if user owns /usr/local/bin, e.g. Homebrew)
        if try_symlink(&current_exe, &target) {
            print_success(&format!(
                "Symlinked: {} → {}",
                target.display(),
                current_exe.display()
            ));
            println!("  You can now use 'thinclaw' from any terminal.");
            return;
        }

        // Need elevated permissions — ask
        println!();
        print_info(
            "thinclaw is not on your PATH. Create a symlink so you can run it from anywhere?",
        );
        match confirm(
            "Create /usr/local/bin/thinclaw symlink (requires sudo)?",
            true,
        ) {
            Ok(true) => {
                let status = std::process::Command::new("sudo")
                    .args(["ln", "-sf"])
                    .arg(current_exe.display().to_string())
                    .arg(target.display().to_string())
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        print_success(&format!(
                            "Symlinked: {} → {}",
                            target.display(),
                            current_exe.display()
                        ));
                        println!("  You can now use 'thinclaw' from any terminal.");
                    }
                    _ => {
                        print_info(&format!(
                            "Symlink failed. Add it manually:\n  \
                             sudo ln -sf {} {}",
                            current_exe.display(),
                            target.display()
                        ));
                    }
                }
            }
            _ => {
                print_info(&format!(
                    "Skipped. To add it later:\n  \
                     sudo ln -sf {} {}",
                    current_exe.display(),
                    target.display()
                ));
            }
        }
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

/// Try to create a symlink, removing any existing file/link at the target.
/// Returns true on success.
#[cfg(unix)]
fn try_symlink(source: &std::path::Path, target: &std::path::Path) -> bool {
    // Remove existing symlink/file if present (ignore errors)
    let _ = std::fs::remove_file(target);
    std::os::unix::fs::symlink(source, target).is_ok()
}

#[cfg(not(unix))]
fn try_symlink(_source: &std::path::Path, _target: &std::path::Path) -> bool {
    false
}

/// Check if a directory is present in the current PATH.
fn path_contains(dir: &std::path::Path) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|p| p == dir)
}
