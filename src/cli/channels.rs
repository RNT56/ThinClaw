//! Channel management CLI commands.
//!
//! Subcommands:
//! - `channels list` — list all configured channels and their status
//! - `channels info` — show channel details

use clap::Subcommand;

use crate::terminal_branding::TerminalBranding;

#[derive(Subcommand, Debug, Clone)]
pub enum ChannelCommand {
    /// List all configured channels and their status
    List {
        /// Output format: table (default) or json
        #[arg(long, default_value = "table")]
        format: String,
    },

    /// Show details for a specific channel
    Info {
        /// Channel name (e.g. "telegram", "signal", "gateway")
        channel: String,
    },
}

/// Run a channels CLI command.
pub async fn run_channels_command(cmd: ChannelCommand) -> anyhow::Result<()> {
    match cmd {
        ChannelCommand::List { format } => list_channels(&format).await,
        ChannelCommand::Info { channel } => channel_info(&channel).await,
    }
}

/// Known channel configuration keys and how to detect them.
struct ChannelCheck {
    name: &'static str,
    env_key: &'static str,
    description: &'static str,
}

const KNOWN_CHANNELS: &[ChannelCheck] = &[
    ChannelCheck {
        name: "gateway",
        env_key: "GATEWAY_ENABLED",
        description: "Web gateway (chat, memory, jobs, logs)",
    },
    ChannelCheck {
        name: "cli",
        env_key: "CLI_ENABLED",
        description: "Interactive CLI / REPL",
    },
    ChannelCheck {
        name: "signal",
        env_key: "SIGNAL_HTTP_URL",
        description: "Signal messenger (signal-cli daemon)",
    },
    ChannelCheck {
        name: "nostr",
        env_key: "NOSTR_SECRET_KEY",
        description: "Nostr NIP-04 encrypted DM channel",
    },
    ChannelCheck {
        name: "http",
        env_key: "HTTP_WEBHOOK_ENABLED",
        description: "HTTP webhook channel",
    },
    ChannelCheck {
        name: "telegram",
        env_key: "TELEGRAM_BOT_TOKEN",
        description: "Telegram bot (WASM channel)",
    },
    ChannelCheck {
        name: "slack",
        env_key: "SLACK_BOT_TOKEN",
        description: "Slack bot (WASM tool)",
    },
    ChannelCheck {
        name: "discord",
        env_key: "DISCORD_BOT_TOKEN",
        description: "Discord bot (native Gateway WS + REST)",
    },
    ChannelCheck {
        name: "imessage",
        env_key: "IMESSAGE_ENABLED",
        description: "iMessage (macOS only, chat.db polling)",
    },
    ChannelCheck {
        name: "apple_mail",
        env_key: "APPLE_MAIL_ENABLED",
        description: "Apple Mail (macOS only, Envelope Index polling)",
    },
];

/// List all channels.
async fn list_channels(format: &str) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let _ = dotenvy::dotenv();
    crate::bootstrap::load_thinclaw_env();

    let mut channels: Vec<serde_json::Value> = Vec::new();

    for ch in KNOWN_CHANNELS {
        let configured = std::env::var(ch.env_key).is_ok();
        let status = if configured {
            "configured"
        } else {
            "not configured"
        };

        channels.push(serde_json::json!({
            "name": ch.name,
            "status": status,
            "configured": configured,
            "description": ch.description,
        }));
    }

    // Check for WASM channels directory.
    let wasm_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".thinclaw")
        .join("channels");
    if wasm_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&wasm_dir)
    {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == "wasm") {
                let name = entry
                    .path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                channels.push(serde_json::json!({
                    "name": name,
                    "status": "wasm",
                    "configured": true,
                    "description": "WASM channel plugin",
                }));
            }
        }
    }

    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&channels)?);
        return Ok(());
    }

    branding.print_banner("Channels", Some("Inspect active message surfaces"));
    println!(
        "{}",
        branding.body_bold(format!("{:<15}  {:<16}  DESCRIPTION", "CHANNEL", "STATUS"))
    );
    println!("{}", branding.separator(70));

    for ch in &channels {
        let icon = if ch["configured"].as_bool().unwrap_or(false) {
            "✅"
        } else {
            "⬜"
        };
        println!(
            "{} {:<13}  {:<16}  {}",
            icon,
            ch["name"].as_str().unwrap_or("?"),
            ch["status"].as_str().unwrap_or("?"),
            ch["description"].as_str().unwrap_or(""),
        );
    }

    let configured_count = channels
        .iter()
        .filter(|c| c["configured"].as_bool().unwrap_or(false))
        .count();
    println!();
    println!(
        "{}",
        branding.muted(format!(
            "{} channel(s) configured, {} not configured.",
            configured_count,
            channels.len() - configured_count
        ))
    );

    Ok(())
}

/// Show details for a specific channel.
async fn channel_info(channel: &str) -> anyhow::Result<()> {
    let branding = TerminalBranding::current();
    let _ = dotenvy::dotenv();
    crate::bootstrap::load_thinclaw_env();

    let known = KNOWN_CHANNELS.iter().find(|c| c.name == channel);

    match known {
        Some(ch) => {
            let configured = std::env::var(ch.env_key).is_ok();
            branding.print_banner("Channels", Some("Inspect a configured surface"));
            println!("{}", branding.key_value("Channel", ch.name));
            println!("{}", branding.key_value("Description", ch.description));
            println!(
                "{}",
                branding.key_value(
                    "Status",
                    if configured {
                        branding.good("configured")
                    } else {
                        branding.warn("not configured")
                    }
                )
            );
            println!("{}", branding.key_value("Config key", ch.env_key));

            // Show channel-specific details.
            match ch.name {
                "gateway" => {
                    let host =
                        std::env::var("GATEWAY_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
                    let port = std::env::var("GATEWAY_PORT").unwrap_or_else(|_| "3000".to_string());
                    println!(
                        "{}",
                        branding.key_value("Endpoint", format!("http://{}:{}/", host, port))
                    );
                }
                "signal" => {
                    if let Ok(url) = std::env::var("SIGNAL_HTTP_URL") {
                        // Redact URL for security.
                        let redacted = if url.len() > 20 {
                            // Safe char-boundary slicing to avoid UTF-8 panics
                            let prefix_end = url
                                .char_indices()
                                .nth(15)
                                .map(|(i, _)| i)
                                .unwrap_or(url.len());
                            let suffix_start =
                                url.char_indices().rev().nth(4).map(|(i, _)| i).unwrap_or(0);
                            format!("{}...{}", &url[..prefix_end], &url[suffix_start..])
                        } else {
                            url
                        };
                        println!("{}", branding.key_value("HTTP URL", redacted));
                    }
                }
                "telegram" => {
                    if std::env::var("TELEGRAM_BOT_TOKEN").is_ok() {
                        println!("{}", branding.key_value("Bot token", "••••••• (set)"));
                    }
                    if let Ok(owner) = std::env::var("TELEGRAM_OWNER_ID") {
                        println!("{}", branding.key_value("Owner ID", owner));
                    }
                }
                _ => {}
            }
        }
        None => {
            // Check WASM channels.
            let wasm_dir = dirs::home_dir()
                .unwrap_or_default()
                .join(".thinclaw")
                .join("channels");
            let wasm_path = wasm_dir.join(format!("{}.wasm", channel));

            if wasm_path.exists() {
                let metadata = std::fs::metadata(&wasm_path)?;
                branding.print_banner("Channels", Some("Inspect a WASM surface"));
                println!(
                    "{}",
                    branding.key_value("Channel", format!("{} (WASM plugin)", channel))
                );
                println!("{}", branding.key_value("Path", wasm_path.display()));
                println!(
                    "{}",
                    branding.key_value("Size", format!("{:.1} KB", metadata.len() as f64 / 1024.0))
                );
            } else {
                anyhow::bail!(
                    "Unknown channel '{}'. Use 'thinclaw channels list' to see available channels.",
                    channel
                );
            }
        }
    }

    Ok(())
}
