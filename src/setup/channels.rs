//! Channel-specific setup flows.
//!
//! Each channel (Telegram, HTTP, etc.) has its own setup function that:
//! 1. Displays setup instructions
//! 2. Collects configuration (tokens, ports, etc.)
//! 3. Validates the configuration
//! 4. Saves secrets to the database

use std::sync::Arc;

use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use url::Url;
use uuid::Uuid;

#[cfg(feature = "postgres")]
use crate::secrets::SecretsCrypto;
use crate::secrets::{CreateSecretParams, SecretsStore};
use crate::settings::{Settings, TunnelSettings};
use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success, secret_input,
    select_one,
};

/// Typed errors for channel setup flows.
#[derive(Debug, thiserror::Error)]
pub enum ChannelSetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Network(String),

    #[error("{0}")]
    Secrets(String),

    #[error("{0}")]
    Validation(String),
}

/// Context for saving secrets during setup.
pub struct SecretsContext {
    store: Arc<dyn SecretsStore>,
    user_id: String,
}

impl SecretsContext {
    /// Create a new secrets context from a trait-object store.
    pub fn from_store(store: Arc<dyn SecretsStore>, user_id: &str) -> Self {
        Self {
            store,
            user_id: user_id.to_string(),
        }
    }

    /// Create a new secrets context from a PostgreSQL pool and crypto.
    #[cfg(feature = "postgres")]
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>, user_id: &str) -> Self {
        Self {
            store: Arc::new(crate::secrets::PostgresSecretsStore::new(pool, crypto)),
            user_id: user_id.to_string(),
        }
    }

    /// Save a secret to the database.
    pub async fn save_secret(
        &self,
        name: &str,
        value: &SecretString,
    ) -> Result<(), ChannelSetupError> {
        let params = CreateSecretParams::new(name, value.expose_secret());

        self.store
            .create(&self.user_id, params)
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to save secret: {}", e)))?;

        Ok(())
    }

    /// Check if a secret exists.
    pub async fn secret_exists(&self, name: &str) -> bool {
        match self.store.exists(&self.user_id, name).await {
            Ok(exists) => exists,
            Err(e) => {
                tracing::warn!(secret = name, error = %e, "Failed to check if secret exists, assuming absent");
                false
            }
        }
    }

    /// Read a secret from the database (decrypted).
    pub async fn get_secret(&self, name: &str) -> Result<SecretString, ChannelSetupError> {
        let decrypted = self
            .store
            .get_decrypted(&self.user_id, name)
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to read secret: {}", e)))?;
        Ok(SecretString::from(decrypted.expose().to_string()))
    }
}

const TUNNEL_NGROK_TOKEN_SECRET: &str = "tunnel_ngrok_token";
const TUNNEL_CF_TOKEN_SECRET: &str = "tunnel_cf_token";

/// Result of Telegram setup.
#[derive(Debug, Clone)]
pub struct TelegramSetupResult {
    pub enabled: bool,
    pub bot_username: Option<String>,
    pub webhook_secret: Option<String>,
    pub owner_id: Option<i64>,
}

/// Telegram Bot API response for getMe.
#[derive(Debug, Deserialize)]
struct TelegramGetMeResponse {
    ok: bool,
    result: Option<TelegramUser>,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    username: Option<String>,
    #[allow(dead_code)]
    first_name: String,
}

/// Telegram Bot API response for getUpdates.
#[derive(Debug, Deserialize)]
struct TelegramGetUpdatesResponse {
    ok: bool,
    result: Vec<TelegramUpdate>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramUpdateMessage>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateMessage {
    from: Option<TelegramUpdateUser>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdateUser {
    id: i64,
    first_name: String,
    username: Option<String>,
}

/// Set up Telegram bot channel.
///
/// Guides the user through:
/// 1. Creating a bot with @BotFather
/// 2. Entering the bot token
/// 3. Validating the token
/// 4. Saving the token to the database
pub async fn setup_telegram(
    secrets: &SecretsContext,
    settings: &Settings,
) -> Result<TelegramSetupResult, ChannelSetupError> {
    println!("Telegram Setup:");
    println!();
    print_info("To create a Telegram bot:");
    print_info("1. Open Telegram and message @BotFather");
    print_info("2. Send /newbot and follow the prompts");
    print_info("3. Copy the bot token (looks like 123456:ABC-DEF...)");
    println!();

    // Check if token already exists
    if secrets.secret_exists("telegram_bot_token").await {
        print_info("Existing Telegram token found in database.");
        if !confirm("Replace existing token?", false)? {
            // Still offer to configure webhook secret and owner binding
            let webhook_secret = setup_telegram_webhook_secret(secrets, &settings.tunnel).await?;
            let owner_id = bind_telegram_owner_flow(secrets, settings).await?;
            return Ok(TelegramSetupResult {
                enabled: true,
                bot_username: None,
                webhook_secret,
                owner_id,
            });
        }
    }

    loop {
        let token = secret_input("Bot token (from @BotFather)")?;

        // Validate the token
        print_info("Validating bot token...");

        match validate_telegram_token(&token).await {
            Ok(username) => {
                print_success(&format!(
                    "Bot validated: @{}",
                    username.as_deref().unwrap_or("unknown")
                ));

                // Save to database
                secrets.save_secret("telegram_bot_token", &token).await?;
                print_success("Token saved to database");

                // Bind bot to owner's Telegram account
                let owner_id = bind_telegram_owner(&token).await?;

                // Offer webhook secret configuration
                let webhook_secret =
                    setup_telegram_webhook_secret(secrets, &settings.tunnel).await?;

                return Ok(TelegramSetupResult {
                    enabled: true,
                    bot_username: username,
                    webhook_secret,
                    owner_id,
                });
            }
            Err(e) => {
                print_error(&format!("Token validation failed: {}", e));

                if !confirm("Try again?", true)? {
                    return Ok(TelegramSetupResult {
                        enabled: false,
                        bot_username: None,
                        webhook_secret: None,
                        owner_id: None,
                    });
                }
            }
        }
    }
}

/// Bind the bot to the owner's Telegram account by having them send a message.
///
/// Polls `getUpdates` until a message arrives, then captures the sender's user ID.
/// Returns `None` if the user declines or the flow times out.
async fn bind_telegram_owner(token: &SecretString) -> Result<Option<i64>, ChannelSetupError> {
    println!();
    print_info("Account Binding (recommended):");
    print_info("Binding restricts the bot so only YOU can use it.");
    print_info("Without this, anyone who finds your bot can send it messages.");
    println!();

    if !confirm("Bind bot to your Telegram account?", true)? {
        print_info("Skipping account binding. Bot will accept messages from all users.");
        return Ok(None);
    }

    print_info("Send any message (e.g. /start) to your bot in Telegram.");
    print_info("Waiting for your message (up to 120 seconds)...");

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(35))
        .build()
        .map_err(|e| ChannelSetupError::Network(format!("Failed to create HTTP client: {}", e)))?;

    // Clear any existing webhook so getUpdates works
    let delete_url = format!(
        "https://api.telegram.org/bot{}/deleteWebhook",
        token.expose_secret()
    );
    if let Err(e) = client.post(&delete_url).send().await {
        tracing::warn!("Failed to delete webhook (getUpdates may not work): {e}");
    }

    let updates_url = format!(
        "https://api.telegram.org/bot{}/getUpdates",
        token.expose_secret()
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);

    while std::time::Instant::now() < deadline {
        let response = client
            .get(&updates_url)
            .query(&[("timeout", "30"), ("allowed_updates", "[\"message\"]")])
            .send()
            .await
            .map_err(|e| ChannelSetupError::Network(format!("getUpdates request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(ChannelSetupError::Network(format!(
                "getUpdates returned status {}",
                response.status()
            )));
        }

        let body: TelegramGetUpdatesResponse = response.json().await.map_err(|e| {
            ChannelSetupError::Network(format!("Failed to parse getUpdates response: {}", e))
        })?;

        if !body.ok {
            return Err(ChannelSetupError::Network(
                "Telegram API returned error for getUpdates".to_string(),
            ));
        }

        // Find the first message with a sender
        for update in &body.result {
            if let Some(ref msg) = update.message
                && let Some(ref from) = msg.from
            {
                let display_name = from
                    .username
                    .as_ref()
                    .map(|u| format!("@{}", u))
                    .unwrap_or_else(|| from.first_name.clone());

                print_success(&format!(
                    "Received message from {} (ID: {})",
                    display_name, from.id
                ));

                // Acknowledge the update so it doesn't pile up
                let ack_url = format!(
                    "https://api.telegram.org/bot{}/getUpdates",
                    token.expose_secret()
                );
                if let Err(e) = client
                    .get(&ack_url)
                    .query(&[("offset", &(update.update_id + 1).to_string())])
                    .send()
                    .await
                {
                    tracing::warn!("Failed to acknowledge Telegram update: {e}");
                }

                return Ok(Some(from.id));
            }
        }
    }

    print_error("Timed out waiting for a message. You can re-run setup to try again.");
    print_info("Bot will accept messages from all users until owner is bound.");
    Ok(None)
}

/// Bind flow when the token already exists (reads from secrets store).
///
/// Retrieves the saved bot token and delegates to `bind_telegram_owner`.
async fn bind_telegram_owner_flow(
    secrets: &SecretsContext,
    settings: &Settings,
) -> Result<Option<i64>, ChannelSetupError> {
    if settings.channels.telegram_owner_id.is_some() {
        print_info("Bot is already bound to a Telegram account.");
        if !confirm("Re-bind to a different account?", false)? {
            return Ok(settings.channels.telegram_owner_id);
        }
    }

    // We need the token to poll getUpdates
    let token = secrets.get_secret("telegram_bot_token").await?;

    bind_telegram_owner(&token).await
}

/// Set up a tunnel for exposing the agent to the internet.
///
/// This is shared across all channels that need webhook endpoints.
/// Returns a `TunnelSettings` with provider config (managed tunnel)
/// or a static URL.
pub async fn setup_tunnel(
    settings: &Settings,
    secrets: Option<&SecretsContext>,
) -> Result<TunnelSettings, ChannelSetupError> {
    // Show existing config
    let has_existing = settings.tunnel.public_url.is_some() || settings.tunnel.provider.is_some();
    if has_existing {
        println!();
        print_info("Current tunnel configuration:");
        let t = &settings.tunnel;
        let has_ngrok_secret = if let Some(ctx) = secrets {
            ctx.secret_exists(TUNNEL_NGROK_TOKEN_SECRET).await
        } else {
            false
        };
        let has_cf_secret = if let Some(ctx) = secrets {
            ctx.secret_exists(TUNNEL_CF_TOKEN_SECRET).await
        } else {
            false
        };
        match t.provider.as_deref() {
            Some("ngrok") => {
                print_info("  Provider:  ngrok");
                if let Some(ref domain) = t.ngrok_domain {
                    print_info(&format!("  Domain:    {}", domain));
                }
                if t.ngrok_token.is_some() || has_ngrok_secret {
                    print_info("  Auth:      token configured");
                }
            }
            Some("cloudflare") => {
                print_info("  Provider:  Cloudflare Tunnel");
                if t.cf_token.is_some() || has_cf_secret {
                    print_info("  Auth:      token configured");
                }
            }
            Some("tailscale") => {
                let mode = if t.ts_funnel {
                    "Funnel (public)"
                } else {
                    "Serve (tailnet-only)"
                };
                print_info(&format!("  Provider:  Tailscale {}", mode));
                if let Some(ref hostname) = t.ts_hostname {
                    print_info(&format!("  Hostname:  {}", hostname));
                }
            }
            Some("custom") => {
                print_info("  Provider:  Custom command");
                if let Some(ref cmd) = t.custom_command {
                    print_info(&format!("  Command:   {}", cmd));
                }
                if let Some(ref url) = t.custom_health_url {
                    print_info(&format!("  Health:    {}", url));
                }
            }
            Some(other) => {
                print_info(&format!("  Provider:  {}", other));
            }
            None => {}
        }
        if let Some(ref url) = t.public_url {
            print_info(&format!("  URL:       {}", url));
        }
        println!();
        if !confirm("Change tunnel configuration?", false)? {
            return Ok(settings.tunnel.clone());
        }
    }

    println!();
    print_info("Tunnel Configuration");
    println!();
    print_info("Without a tunnel, channels like Telegram use POLLING mode:");
    print_info("  Your agent asks Telegram \"any new messages?\" every ~5 seconds.");
    print_info("  This works reliably from anywhere (home WiFi, VPN, any network).");
    println!();
    print_info("With a tunnel, channels switch to WEBHOOK mode:");
    print_info("  Telegram pushes messages to your agent INSTANTLY (< 200ms).");
    print_info("  Also enables: Slack events, Discord interactions, GitHub webhooks.");
    println!();
    print_info("Why is a tunnel needed?");
    print_info("  Webhooks require a publicly reachable HTTPS URL. Most home networks");
    print_info("  use NAT/firewall — Telegram's servers simply cannot reach your machine");
    print_info("  without a tunnel creating a public entrypoint.");
    println!();
    print_info("Recommended: Tailscale Funnel (free, zero-config, persistent hostname)");
    print_info("  Alternatives: ngrok (free), Cloudflare Tunnel (free), or your own.");
    print_info("  If you're unsure, skip this — polling works perfectly for most users.");
    println!();

    if !confirm("Configure a tunnel for instant webhook delivery?", false)? {
        print_info("No tunnel configured. Telegram and other channels will use polling mode.");
        return Ok(TunnelSettings::default());
    }

    let options = &[
        "ngrok         - managed tunnel, starts automatically",
        "Cloudflare    - cloudflared tunnel, starts automatically",
        "Tailscale     - Tailscale Funnel/Serve, starts automatically",
        "Custom        - your own tunnel command",
        "Static URL    - you manage the tunnel yourself",
    ];

    let choice = select_one("Select tunnel provider:", options)?;

    match choice {
        0 => setup_tunnel_ngrok(secrets).await,
        1 => setup_tunnel_cloudflare(secrets).await,
        2 => setup_tunnel_tailscale(),
        3 => setup_tunnel_custom(),
        4 => setup_tunnel_static(),
        _ => Ok(TunnelSettings::default()),
    }
}

async fn setup_tunnel_ngrok(
    secrets: Option<&SecretsContext>,
) -> Result<TunnelSettings, ChannelSetupError> {
    // Check if ngrok is installed
    if !is_binary_installed("ngrok") {
        println!();
        print_error("'ngrok' binary not found in PATH.");
        print_info("Install ngrok before starting the agent:");
        print_info("  macOS:   brew install ngrok");
        print_info("  Linux:   snap install ngrok  (or download from https://ngrok.com/download)");
        print_info("  Windows: choco install ngrok");
        println!();
        if !confirm(
            "Continue configuring ngrok anyway? (you can install it before starting the agent)",
            false,
        )? {
            return Ok(TunnelSettings::default());
        }
    }

    print_info("Get your auth token from: https://dashboard.ngrok.com/get-started/your-authtoken");
    println!();

    let token = secret_input("ngrok auth token")?;
    let domain = optional_input("Custom domain", Some("leave empty for auto-assigned"))?;
    let ngrok_token = if let Some(ctx) = secrets {
        ctx.save_secret(TUNNEL_NGROK_TOKEN_SECRET, &token).await?;
        None
    } else {
        Some(token.expose_secret().to_string())
    };

    print_success("ngrok configured. Tunnel will start automatically at boot.");
    if !is_binary_installed("ngrok") {
        print_info("⚠ Remember to install 'ngrok' before running 'thinclaw run'.");
    }

    Ok(TunnelSettings {
        provider: Some("ngrok".to_string()),
        ngrok_token,
        ngrok_domain: domain,
        ..Default::default()
    })
}

async fn setup_tunnel_cloudflare(
    secrets: Option<&SecretsContext>,
) -> Result<TunnelSettings, ChannelSetupError> {
    // Check if cloudflared is installed
    if !is_binary_installed("cloudflared") {
        println!();
        print_error("'cloudflared' binary not found in PATH.");
        print_info("Install cloudflared before starting the agent:");
        print_info("  macOS:   brew install cloudflare/cloudflare/cloudflared");
        print_info(
            "  Linux:   See https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/",
        );
        print_info("  Windows: winget install Cloudflare.cloudflared");
        println!();
        if !confirm(
            "Continue configuring cloudflared anyway? (you can install it before starting the agent)",
            false,
        )? {
            return Ok(TunnelSettings::default());
        }
    }

    print_info("Get your tunnel token from the Cloudflare Zero Trust dashboard:");
    print_info("  https://one.dash.cloudflare.com/ > Networks > Tunnels");
    println!();

    let token = secret_input("Cloudflare tunnel token")?;
    let cf_token = if let Some(ctx) = secrets {
        ctx.save_secret(TUNNEL_CF_TOKEN_SECRET, &token).await?;
        None
    } else {
        Some(token.expose_secret().to_string())
    };

    print_success("Cloudflare tunnel configured. Tunnel will start automatically at boot.");
    if !is_binary_installed("cloudflared") {
        print_info("⚠ Remember to install 'cloudflared' before running 'thinclaw run'.");
    }

    Ok(TunnelSettings {
        provider: Some("cloudflare".to_string()),
        cf_token,
        ..Default::default()
    })
}

/// Test whether the `tailscale` CLI can actually run without crashing.
///
/// On macOS, the App Store version's CLI wrapper crashes with a
/// `BundleIdentifier` error when spawned from another process.
/// This function catches that by running `tailscale version` and
/// checking for a clean exit.
///
/// Uses `resolve_binary` so that Homebrew-installed CLIs at
/// `/opt/homebrew/bin/tailscale` are found even when that directory
/// is not in `$PATH` (common for processes spawned by launchd/IDEs).
fn test_tailscale_cli() -> bool {
    let binary = crate::util::resolve_binary("tailscale");
    let output = std::process::Command::new(&binary)
        .arg("version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();

    match output {
        Ok(o) => {
            if o.status.success() {
                return true;
            }
            // Check if it crashed with the known macOS issue
            let stderr = String::from_utf8_lossy(&o.stderr);
            if stderr.contains("BundleIdentifier") || stderr.contains("Fatal error") {
                return false;
            }
            // Other non-zero exit might still mean it's installed (e.g. not logged in)
            // but at least it didn't crash
            true
        }
        Err(_) => false, // binary not found
    }
}

fn setup_tunnel_tailscale() -> Result<TunnelSettings, ChannelSetupError> {
    // Check if tailscale CLI is installed AND working.
    // On macOS, the App Store version installs a CLI shim that crashes with
    // BundleIdentifier errors when spawned from another process.
    let cli_working = test_tailscale_cli();

    if !cli_working {
        println!();

        #[cfg(target_os = "macos")]
        {
            if is_binary_installed("tailscale") {
                // CLI exists but crashes — the App Store BundleIdentifier issue
                print_error("Tailscale CLI is installed but crashes when called from ThinClaw.");
                print_info("This is a known issue with the macOS App Store version's CLI.");
                print_info("The standalone Homebrew CLI fixes this and works alongside the app.");
            } else if std::path::Path::new("/Applications/Tailscale.app").exists() {
                print_info("Tailscale app is installed, but the CLI is not available.");
            } else {
                print_error("Tailscale is not installed.");
            }

            println!();

            // Check if Homebrew is available for auto-install
            let has_brew = std::process::Command::new("brew")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if has_brew {
                if confirm(
                    "Install Tailscale CLI via Homebrew? (brew install tailscale)",
                    true,
                )? {
                    print_info("Installing tailscale via Homebrew (this may take a minute)...");
                    let install_result = std::process::Command::new("brew")
                        .args(["install", "tailscale"])
                        .status();

                    match install_result {
                        Ok(status) if status.success() => {
                            if test_tailscale_cli() {
                                print_success("Tailscale CLI installed and working!");
                            } else {
                                print_success("Tailscale CLI installed.");
                                print_info(
                                    "You may need to start the service: brew services start tailscale",
                                );
                            }
                        }
                        Ok(_) => {
                            print_error("Homebrew install failed. Try manually: brew install tailscale");
                            println!();
                            if !confirm("Continue configuring anyway?", false)? {
                                return Ok(TunnelSettings::default());
                            }
                        }
                        Err(e) => {
                            print_error(&format!("Could not run brew: {}", e));
                            if !confirm("Continue configuring anyway?", false)? {
                                return Ok(TunnelSettings::default());
                            }
                        }
                    }
                } else if !confirm(
                    "Continue without installing? (install before starting the agent)",
                    false,
                )? {
                    return Ok(TunnelSettings::default());
                }
            } else {
                // No Homebrew
                print_info("Homebrew is not installed. Install the Tailscale CLI manually:");
                println!();
                print_info("  Option 1: Install Homebrew, then: brew install tailscale");
                print_info("  Option 2: Download from https://tailscale.com/download/mac");
                println!();
                if !confirm(
                    "Continue configuring anyway? (install before starting the agent)",
                    false,
                )? {
                    return Ok(TunnelSettings::default());
                }
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            print_error("'tailscale' CLI not found in PATH.");
            print_info("Install Tailscale before starting the agent:");
            print_info("  Linux:   curl -fsSL https://tailscale.com/install.sh | sh");
            print_info("  Windows: Download from https://tailscale.com/download/windows");
            println!();
            if !confirm(
                "Continue configuring anyway? (install before starting the agent)",
                false,
            )? {
                return Ok(TunnelSettings::default());
            }
        }
    }

    println!();
    print_info("Tailscale offers two modes:");
    println!();
    print_info("  Funnel (public)  — Makes your agent reachable from the public internet.");
    print_info("                     Required for Telegram/Slack/Discord webhooks.");
    print_info("                     Your hostname (e.g. my-mac.tail1234.ts.net) becomes");
    print_info("                     publicly resolvable with a valid HTTPS certificate.");
    println!();
    print_info("  Serve (tailnet)  — Only reachable from devices on YOUR Tailscale network.");
    print_info("                     Great for private Web UI access from your phone/laptop,");
    print_info("                     but Telegram's servers CANNOT reach it (webhooks won't work,");
    print_info("                     Telegram will fall back to polling mode).");
    println!();

    let funnel = confirm("Use Tailscale Funnel (public internet — needed for webhooks)?", true)?;
    let hostname = optional_input("Hostname override", Some("leave empty for auto-detect"))?;

    let mode = if funnel {
        "Funnel (public)"
    } else {
        "Serve (tailnet-only)"
    };
    print_success(&format!("Tailscale {} configured.", mode));
    if funnel {
        print_info("Make sure Funnel is enabled in your Tailscale admin console:");
        print_info("  1. Visit https://login.tailscale.com/admin/dns → enable HTTPS");
        print_info("  2. Ensure your ACL policy allows Funnel for this machine");
    } else {
        print_info("Note: Telegram and other webhook channels will use polling mode.");
        print_info("You can switch to Funnel later by re-running setup or setting TUNNEL_TS_FUNNEL=true.");
    }
    if !is_binary_installed("tailscale") {
        print_info("⚠ Remember to install 'tailscale' before running 'thinclaw run'.");
    }

    Ok(TunnelSettings {
        provider: Some("tailscale".to_string()),
        ts_funnel: funnel,
        ts_hostname: hostname,
        ..Default::default()
    })
}

fn setup_tunnel_custom() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter a shell command to start your tunnel.");
    print_info("Use {port} and {host} as placeholders.");
    print_info("Example: bore local {port} --to bore.pub");
    println!();

    let command = input("Tunnel command")?;
    if command.is_empty() {
        return Err(ChannelSetupError::Validation(
            "Tunnel command cannot be empty".to_string(),
        ));
    }

    let health_url = optional_input("Health check URL", Some("optional"))?;
    let url_pattern = optional_input(
        "URL pattern (substring to match in stdout)",
        Some("optional"),
    )?;

    print_success("Custom tunnel configured.");

    Ok(TunnelSettings {
        provider: Some("custom".to_string()),
        custom_command: Some(command),
        custom_health_url: health_url,
        custom_url_pattern: url_pattern,
        ..Default::default()
    })
}

fn setup_tunnel_static() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter the public URL of your externally managed tunnel.");
    println!();

    let tunnel_url = input("Tunnel URL (e.g., https://abc123.ngrok.io)")?;

    if !tunnel_url.starts_with("https://") {
        print_error("URL must start with https:// (webhooks require HTTPS)");
        return Err(ChannelSetupError::Validation(
            "Invalid tunnel URL: must use HTTPS".to_string(),
        ));
    }

    let tunnel_url = tunnel_url.trim_end_matches('/').to_string();

    print_success(&format!("Static tunnel URL configured: {}", tunnel_url));
    print_info("Make sure your tunnel is running before starting the agent.");

    Ok(TunnelSettings {
        public_url: Some(tunnel_url),
        ..Default::default()
    })
}

/// Check if a binary is available in PATH or at a known fallback location.
///
/// Delegates to `resolve_binary` from the tunnel module, which checks
/// PATH first and then falls back to known macOS Homebrew paths
/// (including `/opt/homebrew/bin/tailscale` and `/usr/local/bin/tailscale`).
fn is_binary_installed(name: &str) -> bool {
    let resolved = crate::util::resolve_binary(name);

    // resolve_binary returns the bare name if nothing was found;
    // if it returned an absolute path, the binary exists at that path.
    if resolved != name {
        return true;
    }

    // resolve_binary returned the bare name — check if it's on PATH.
    #[cfg(unix)]
    {
        std::process::Command::new("which")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Set up Telegram webhook secret for signature validation.
///
/// Returns the webhook secret if configured.
async fn setup_telegram_webhook_secret(
    secrets: &SecretsContext,
    tunnel: &TunnelSettings,
) -> Result<Option<String>, ChannelSetupError> {
    if tunnel.public_url.is_none() {
        println!();
        print_info("No tunnel configured — Telegram will use polling mode (~5s message delay).");
        print_info("This works perfectly for most users. To switch to instant webhook delivery,");
        print_info("configure a tunnel (Tailscale Funnel, ngrok, or Cloudflare) in setup.");
        return Ok(None);
    }

    println!();
    print_info("Telegram Webhook Security:");
    print_info("A webhook secret adds an extra layer of security by validating");
    print_info("that requests actually come from Telegram's servers.");

    if !confirm("Generate a webhook secret?", true)? {
        return Ok(None);
    }

    let secret = generate_webhook_secret();
    secrets
        .save_secret(
            "telegram_webhook_secret",
            &SecretString::from(secret.clone()),
        )
        .await?;
    print_success("Webhook secret generated and saved");

    Ok(Some(secret))
}

/// Validate a Telegram bot token by calling the getMe API.
///
/// Returns the bot's username if valid.
pub async fn validate_telegram_token(
    token: &SecretString,
) -> Result<Option<String>, ChannelSetupError> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| ChannelSetupError::Network(format!("Failed to create HTTP client: {}", e)))?;

    let url = format!(
        "https://api.telegram.org/bot{}/getMe",
        token.expose_secret()
    );

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ChannelSetupError::Network(format!("Request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(ChannelSetupError::Network(format!(
            "API returned status {}",
            response.status()
        )));
    }

    let body: TelegramGetMeResponse = response
        .json()
        .await
        .map_err(|e| ChannelSetupError::Network(format!("Failed to parse response: {}", e)))?;

    if body.ok {
        Ok(body.result.and_then(|u| u.username))
    } else {
        Err(ChannelSetupError::Network(
            "Telegram API returned error".to_string(),
        ))
    }
}

/// Result of HTTP webhook setup.
#[derive(Debug, Clone)]
pub struct HttpSetupResult {
    pub enabled: bool,
    pub port: u16,
    pub host: String,
}

/// Result of Signal channel setup.
#[derive(Debug, Clone)]
pub struct SignalSetupResult {
    pub enabled: bool,
    pub http_url: String,
    pub account: String,
    pub allow_from: String,
    pub allow_from_groups: String,
    pub dm_policy: String,
    pub group_policy: String,
    pub group_allow_from: String,
}

/// Set up HTTP webhook channel.
pub async fn setup_http(secrets: &SecretsContext) -> Result<HttpSetupResult, ChannelSetupError> {
    println!("HTTP Webhook Setup:");
    println!();
    print_info("The HTTP webhook allows external services to send messages to the agent.");
    println!();

    let port_str = optional_input("Port", Some("default: 8080"))?;
    let port: u16 = port_str
        .as_deref()
        .unwrap_or("8080")
        .parse()
        .map_err(|e| ChannelSetupError::Validation(format!("Invalid port: {}", e)))?;

    if port < 1024 {
        print_info("Note: Ports below 1024 may require root privileges");
    }

    let host =
        optional_input("Host", Some("default: 0.0.0.0"))?.unwrap_or_else(|| "0.0.0.0".to_string());

    // Generate a webhook secret
    if confirm("Generate a webhook secret for authentication?", true)? {
        let secret = generate_webhook_secret();
        secrets
            .save_secret("http_webhook_secret", &SecretString::from(secret))
            .await?;
        print_success("Webhook secret generated and saved to database");
        print_info("Retrieve it later with: thinclaw secret get http_webhook_secret");
    }

    print_success(&format!("HTTP webhook will listen on {}:{}", host, port));

    Ok(HttpSetupResult {
        enabled: true,
        port,
        host,
    })
}

/// Generate a random webhook secret.
pub fn generate_webhook_secret() -> String {
    generate_secret_with_length(32)
}

fn validate_e164(account: &str) -> Result<(), String> {
    if !account.starts_with('+') {
        return Err("E.164 account must start with '+'".to_string());
    }
    let digits = &account[1..];
    if digits.is_empty() {
        return Err("E.164 account must have digits after '+'".to_string());
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err("E.164 account must contain only digits after '+'".to_string());
    }
    if digits.len() < 7 || digits.len() > 15 {
        return Err("E.164 account must be 7-15 digits after '+'".to_string());
    }
    Ok(())
}

fn validate_allow_from_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for (i, item) in list.split(',').enumerate() {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "*" {
            continue;
        }
        if let Some(uuid_part) = trimmed.strip_prefix("uuid:") {
            if Uuid::parse_str(uuid_part).is_err() {
                return Err(format!(
                    "allow_from[{}]: '{}' is not a valid UUID (after 'uuid:' prefix)",
                    i, trimmed
                ));
            }
            continue;
        }
        if validate_e164(trimmed).is_ok() {
            continue;
        }
        if Uuid::parse_str(trimmed).is_ok() {
            continue;
        }
        return Err(format!(
            "allow_from[{}]: '{}' must be '*', E.164 phone number, UUID, or 'uuid:<id>'",
            i, trimmed
        ));
    }
    Ok(())
}

fn validate_allow_from_groups_list(list: &str) -> Result<(), String> {
    if list.is_empty() {
        return Ok(());
    }
    for item in list.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() || trimmed == "*" {
            continue;
        }
    }
    Ok(())
}

/// Set up Signal channel.
/// `Settings` is reserved for future use
pub async fn setup_signal(_settings: &Settings) -> Result<SignalSetupResult, ChannelSetupError> {
    println!("Signal Channel Setup:");
    println!();
    print_info("Signal channel connects to a signal-cli daemon running in HTTP mode.");
    println!();

    let http_url = input("Signal-cli HTTP URL")?;
    match Url::parse(&http_url) {
        Ok(url) if url.scheme() == "http" || url.scheme() == "https" => {}
        Ok(_) => {
            print_error("URL must use http or https scheme");
            return Err(ChannelSetupError::Validation(
                "Invalid HTTP URL: must use http or https scheme".to_string(),
            ));
        }
        Err(e) => {
            print_error(&format!("Invalid URL: {}", e));
            return Err(ChannelSetupError::Validation(format!(
                "Invalid HTTP URL: {}",
                e
            )));
        }
    }

    let account = input("Signal account (E.164)")?;
    if let Err(e) = validate_e164(&account) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    let allow_from = optional_input(
        "Allow from (comma-separated: E.164 numbers, '*' for anyone, UUIDs or 'uuid:<id>'; empty for self-only)",
        Some(&format!("default: {} (self-only)", account)),
    )?
    .unwrap_or_else(|| account.clone());

    let dm_policy = optional_input(
        "DM policy (open, allowlist, pairing)",
        Some("default: pairing"),
    )?
    .unwrap_or_else(|| "pairing".to_string());

    let allow_from_groups = optional_input(
        "Allow from groups (comma-separated group IDs, '*' for any group; empty for none)",
        Some("default: (none)"),
    )?
    .unwrap_or_default();

    let group_policy = optional_input(
        "Group policy (allowlist, open, disabled)",
        Some("default: allowlist"),
    )?
    .unwrap_or_else(|| "allowlist".to_string());

    let group_allow_from = optional_input(
        "Group allow from (comma-separated member IDs; empty to inherit from allow_from)",
        Some("default: (inherit from allow_from)"),
    )?
    .unwrap_or_default();

    if let Err(e) = validate_allow_from_list(&allow_from) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    if let Err(e) = validate_allow_from_groups_list(&allow_from_groups) {
        print_error(&e);
        return Err(ChannelSetupError::Validation(e));
    }

    println!();
    print_success(&format!(
        "Signal channel configured for account: {}",
        account
    ));
    print_info(&format!("HTTP URL: {}", http_url));
    if allow_from == account {
        print_info("Allow from: self-only");
    } else {
        print_info(&format!("Allow from: {}", allow_from));
    }
    print_info(&format!("DM policy: {}", dm_policy));
    if allow_from_groups.is_empty() {
        print_info("Allow from groups: (none)");
    } else {
        print_info(&format!("Allow from groups: {}", allow_from_groups));
    }
    print_info(&format!("Group policy: {}", group_policy));
    if group_allow_from.is_empty() {
        print_info("Group allow from: (inherits from allow_from)");
    } else {
        print_info(&format!("Group allow from: {}", group_allow_from));
    }

    Ok(SignalSetupResult {
        enabled: true,
        http_url,
        account,
        allow_from,
        allow_from_groups,
        dm_policy,
        group_policy,
        group_allow_from,
    })
}

/// Result of WASM channel setup.
#[derive(Debug, Clone)]
pub struct WasmChannelSetupResult {
    pub enabled: bool,
    pub channel_name: String,
}

/// Set up a WASM channel using its capabilities file setup schema.
///
/// Reads setup requirements from the channel's capabilities file and
/// prompts the user for each required secret.
pub async fn setup_wasm_channel(
    secrets: &SecretsContext,
    channel_name: &str,
    setup: &crate::channels::wasm::SetupSchema,
) -> Result<WasmChannelSetupResult, ChannelSetupError> {
    println!("{} Setup:", channel_name);
    println!();

    for secret_config in &setup.required_secrets {
        // Check if this secret already exists
        if secrets.secret_exists(&secret_config.name).await {
            print_info(&format!(
                "Existing {} found in database.",
                secret_config.name
            ));
            if !confirm("Replace existing value?", false)? {
                continue;
            }
        }

        // Get the value from user or auto-generate
        let value = if secret_config.optional {
            let input_value =
                optional_input(&secret_config.prompt, Some("leave empty to auto-generate"))?;

            if let Some(v) = input_value {
                if !v.is_empty() {
                    SecretString::from(v)
                } else if let Some(ref auto_gen) = secret_config.auto_generate {
                    let generated = generate_secret_with_length(auto_gen.length);
                    print_info(&format!(
                        "Auto-generated {} ({} bytes)",
                        secret_config.name, auto_gen.length
                    ));
                    SecretString::from(generated)
                } else {
                    continue; // Skip optional secret with no auto-generate
                }
            } else if let Some(ref auto_gen) = secret_config.auto_generate {
                let generated = generate_secret_with_length(auto_gen.length);
                print_info(&format!(
                    "Auto-generated {} ({} bytes)",
                    secret_config.name, auto_gen.length
                ));
                SecretString::from(generated)
            } else {
                continue; // Skip optional secret with no auto-generate
            }
        } else {
            // Required secret
            let input_value = secret_input(&secret_config.prompt)?;

            // Validate if pattern is provided
            if let Some(ref pattern) = secret_config.validation {
                let re = regex::Regex::new(pattern).map_err(|e| {
                    ChannelSetupError::Validation(format!("Invalid validation pattern: {}", e))
                })?;
                if !re.is_match(input_value.expose_secret()) {
                    print_error(&format!(
                        "Value does not match expected format: {}",
                        pattern
                    ));
                    return Err(ChannelSetupError::Validation(
                        "Validation failed".to_string(),
                    ));
                }
            }

            input_value
        };

        // Save the secret
        secrets.save_secret(&secret_config.name, &value).await?;
        print_success(&format!("{} saved to database", secret_config.name));
    }

    // Validate configured credentials by substituting secrets into the
    // validation URL and making a GET request to verify they work.
    if let Some(ref validation_endpoint) = setup.validation_endpoint {
        let mut url = validation_endpoint.clone();

        // Substitute secret placeholders: {{secret_name}} → actual value
        for secret_config in &setup.required_secrets {
            let placeholder = format!("{{{{{}}}}}", secret_config.name);
            if url.contains(&placeholder) {
                match secrets.get_secret(&secret_config.name).await {
                    Ok(value) => {
                        url = url.replace(&placeholder, value.expose_secret());
                    }
                    Err(_) => {
                        // Secret not found — skip validation
                        print_info(&format!(
                            "Skipping validation: secret '{}' not available",
                            secret_config.name
                        ));
                        url.clear();
                        break;
                    }
                }
            }
        }

        if !url.is_empty() {
            print_info("Validating credentials...");
            let client = Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .ok();

            if let Some(client) = client {
                match client.get(&url).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        print_success("Credentials validated successfully");
                    }
                    Ok(resp) => {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        print_error(&format!(
                            "Credential validation failed (HTTP {}): {}",
                            status,
                            body.chars().take(200).collect::<String>()
                        ));
                        print_info(
                            "The channel will still be configured, but credentials may be invalid.",
                        );
                    }
                    Err(e) => {
                        print_info(&format!(
                            "Could not reach validation endpoint: {} (channel configured anyway)",
                            e
                        ));
                    }
                }
            }
        }
    }

    print_success(&format!("{} channel configured", channel_name));

    Ok(WasmChannelSetupResult {
        enabled: true,
        channel_name: channel_name.to_string(),
    })
}

/// Generate a random secret of specified length (in bytes).
fn generate_secret_with_length(length: usize) -> String {
    use rand::RngCore;
    let mut rng = rand::thread_rng();
    let mut bytes = vec![0u8; length];
    rng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use crate::setup::channels::generate_webhook_secret;

    #[test]
    fn test_generate_webhook_secret() {
        let secret = generate_webhook_secret();
        assert_eq!(secret.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_generate_secret_with_length() {
        use super::generate_secret_with_length;

        let s = generate_secret_with_length(16);
        assert_eq!(s.len(), 32); // 16 bytes = 32 hex chars
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));

        let s2 = generate_secret_with_length(1);
        assert_eq!(s2.len(), 2);
    }
}
