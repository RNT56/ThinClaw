//! Channel management CLI commands.
//!
//! Subcommands:
//! - `channels list` — list all configured channels and their status
//! - `channels info` — show channel details

use std::sync::Arc;

use clap::Subcommand;

use crate::app::{AppBuilder, AppBuilderFlags};
use crate::channels::web::log_layer::LogBroadcaster;
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
        env_key: "NOSTR_ENABLED + NOSTR_PRIVATE_KEY",
        description: "Nostr owner DM control + social actions",
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
    let resolved = load_resolved_config().await?;

    let mut channels: Vec<serde_json::Value> = Vec::new();

    for ch in KNOWN_CHANNELS {
        let configured = channel_is_configured(&resolved, ch.name);
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
    let wasm_dir = crate::platform::state_paths().channels_dir;
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
    let resolved = load_resolved_config().await?;

    let known = KNOWN_CHANNELS.iter().find(|c| c.name == channel);

    match known {
        Some(ch) => {
            let configured = channel_is_configured(&resolved, ch.name);
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
                    if let Some(signal) = resolved.channels.signal.as_ref() {
                        let url = &signal.http_url;
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
                            url.to_string()
                        };
                        println!("{}", branding.key_value("HTTP URL", redacted));
                    }
                }
                "nostr" => {
                    #[cfg(feature = "nostr")]
                    if let Some(nostr) = resolved.channels.nostr.as_ref() {
                        let channel = crate::channels::NostrChannel::new(nostr.clone())?;
                        let runtime = channel.runtime();
                        println!("{}", branding.key_value("Enabled", branding.good("yes")));
                        println!("{}", branding.key_value("Private key", "••••••• (set)"));
                        println!(
                            "{}",
                            branding.key_value("Public key", runtime.public_key_hex())
                        );
                        println!("{}", branding.key_value("npub", runtime.public_key_npub()));
                        println!(
                            "{}",
                            branding.key_value(
                                "Owner pubkey",
                                runtime
                                    .owner_pubkey_hex()
                                    .unwrap_or_else(|| "not configured".to_string())
                            )
                        );
                        println!(
                            "{}",
                            branding.key_value(
                                "Owner npub",
                                runtime
                                    .owner_pubkey_npub()
                                    .unwrap_or_else(|| "not configured".to_string())
                            )
                        );
                        println!(
                            "{}",
                            branding.key_value("Relay count", nostr.relays.len().to_string())
                        );
                        println!(
                            "{}",
                            branding.key_value(
                                "Control ready",
                                if nostr.owner_pubkey.is_some() {
                                    branding.good("yes")
                                } else {
                                    branding.warn("no")
                                }
                            )
                        );
                        println!(
                            "{}",
                            branding.key_value(
                                "Social DM reads",
                                if nostr.social_dm_enabled {
                                    branding.good("enabled")
                                } else {
                                    branding.warn("disabled")
                                }
                            )
                        );
                        if !nostr.allow_from.is_empty() {
                            println!(
                                "{}",
                                branding.key_value(
                                    "Legacy allow_from",
                                    format!("deprecated ({})", nostr.allow_from.join(", "))
                                )
                            );
                        }
                    }
                    #[cfg(not(feature = "nostr"))]
                    {
                        println!(
                            "{}",
                            branding.warn(
                                "Nostr support not compiled into this build (--features nostr)"
                            )
                        );
                    }
                }
                "telegram" => {
                    if resolved.channels.telegram.is_some() {
                        println!("{}", branding.key_value("Bot token", "••••••• (set)"));
                    }
                    if let Some(owner) = resolved.channels.telegram_owner_id {
                        println!("{}", branding.key_value("Owner ID", owner.to_string()));
                    }
                }
                _ => {}
            }
        }
        None => {
            // Check WASM channels.
            let wasm_dir = crate::platform::state_paths().channels_dir;
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

async fn load_resolved_config() -> anyhow::Result<crate::config::Config> {
    let config = crate::config::Config::from_env().await?;
    let mut builder = AppBuilder::new(
        config,
        AppBuilderFlags::default(),
        None,
        Arc::new(LogBroadcaster::new()),
    );

    if let Err(err) = builder.init_database().await {
        tracing::warn!(
            "Channels CLI could not initialize the database for secrets-backed channel detection: {}",
            err
        );
        return Ok(builder.config().clone());
    }

    if let Err(err) = builder.init_secrets().await {
        tracing::warn!(
            "Channels CLI could not initialize secrets-backed channel detection: {}",
            err
        );
    }

    Ok(builder.config().clone())
}

fn channel_is_configured(config: &crate::config::Config, name: &str) -> bool {
    match name {
        "gateway" => config.channels.gateway.is_some(),
        "cli" => config.channels.cli.enabled,
        "signal" => config.channels.signal.is_some(),
        "nostr" => config.channels.nostr.is_some(),
        "http" => config.channels.http.is_some(),
        "telegram" => config.channels.telegram.is_some(),
        "slack" => config.channels.slack.is_some(),
        "discord" => config.channels.discord.is_some(),
        "imessage" => config.channels.imessage.is_some(),
        "apple_mail" => config.channels.apple_mail.is_some(),
        _ => false,
    }
}
