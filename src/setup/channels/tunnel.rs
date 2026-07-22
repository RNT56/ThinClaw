//! Reverse-tunnel setup (ngrok, Cloudflare, Tailscale, custom, static).

use secrecy::ExposeSecret;
use thinclaw_channels::setup as channel_setup;

use crate::settings::{Settings, TunnelSettings};
use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_info, print_success, secret_input,
    select_one,
};

use super::{ChannelSetupError, SecretsContext};

const TUNNEL_NGROK_TOKEN_SECRET: &str = "tunnel_ngrok_token";
const TUNNEL_CF_TOKEN_SECRET: &str = "tunnel_cf_token";

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
        crate::setup::prompts::print_blank_line();
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
            Some(channel_setup::TUNNEL_PROVIDER_NGROK) => {
                print_info("  Provider:  ngrok");
                if let Some(ref domain) = t.ngrok_domain {
                    print_info(&format!("  Domain:    {}", domain));
                }
                if t.ngrok_token.is_some() || has_ngrok_secret {
                    print_info("  Auth:      token configured");
                }
            }
            Some(channel_setup::TUNNEL_PROVIDER_CLOUDFLARE) => {
                print_info("  Provider:  Cloudflare Tunnel");
                if t.cf_token.is_some() || has_cf_secret {
                    print_info("  Auth:      token configured");
                }
                if let Some(ref hostname) = t.cf_hostname {
                    print_info(&format!("  Hostname:  {hostname}"));
                }
            }
            Some(channel_setup::TUNNEL_PROVIDER_TAILSCALE) => {
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
            Some(channel_setup::TUNNEL_PROVIDER_CUSTOM) => {
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
        crate::setup::prompts::print_blank_line();
        if !confirm("Change tunnel configuration?", false)? {
            return Ok(settings.tunnel.clone());
        }
    }

    crate::setup::prompts::print_blank_line();
    print_info("Tunnel Configuration");
    crate::setup::prompts::print_blank_line();
    print_info("Without a tunnel, channels like Telegram use POLLING mode:");
    print_info("  Your agent asks Telegram \"any new messages?\" every ~5 seconds.");
    print_info("  This works reliably from anywhere (home WiFi, VPN, any network).");
    crate::setup::prompts::print_blank_line();
    print_info("With a tunnel, channels switch to WEBHOOK mode:");
    print_info("  Telegram pushes messages to your agent INSTANTLY (< 200ms).");
    print_info("  Also enables: Slack events, Discord interactions, GitHub webhooks.");
    crate::setup::prompts::print_blank_line();
    print_info("Why is a tunnel needed?");
    print_info("  Webhooks require a publicly reachable HTTPS URL. Most home networks");
    print_info("  use NAT/firewall — Telegram's servers simply cannot reach your machine");
    print_info("  without a tunnel creating a public entrypoint.");
    crate::setup::prompts::print_blank_line();
    print_info("Recommended: Tailscale Funnel (free, zero-config, persistent hostname)");
    print_info("  Alternatives: ngrok (free), Cloudflare Tunnel (free), or your own.");
    print_info("  If you're unsure, skip this — polling works perfectly for most users.");
    crate::setup::prompts::print_blank_line();

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
        2 => setup_tunnel_tailscale().await,
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
        crate::setup::prompts::print_blank_line();
        print_error("'ngrok' binary not found in PATH.");
        print_info("Install ngrok before starting the agent:");
        print_info("  macOS:   brew install ngrok");
        print_info("  Linux:   snap install ngrok  (or download from https://ngrok.com/download)");
        print_info("  Windows: choco install ngrok");
        crate::setup::prompts::print_blank_line();
        if !confirm(
            "Continue configuring ngrok anyway? (you can install it before starting the agent)",
            false,
        )? {
            return Ok(TunnelSettings::default());
        }
    }

    print_info("Get your auth token from: https://dashboard.ngrok.com/get-started/your-authtoken");
    crate::setup::prompts::print_blank_line();

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
        provider: Some(channel_setup::TUNNEL_PROVIDER_NGROK.to_string()),
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
        crate::setup::prompts::print_blank_line();
        print_error("'cloudflared' binary not found in PATH.");
        print_info("Install cloudflared before starting the agent:");
        print_info("  macOS:   brew install cloudflare/cloudflare/cloudflared");
        print_info(
            "  Linux:   See https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/downloads/",
        );
        print_info("  Windows: winget install Cloudflare.cloudflared");
        crate::setup::prompts::print_blank_line();
        if !confirm(
            "Continue configuring cloudflared anyway? (you can install it before starting the agent)",
            false,
        )? {
            return Ok(TunnelSettings::default());
        }
    }

    print_info("Get your tunnel token from the Cloudflare Zero Trust dashboard:");
    print_info("  https://one.dash.cloudflare.com/ > Networks > Tunnels");
    crate::setup::prompts::print_blank_line();

    let token = secret_input("Cloudflare tunnel token")?;
    let hostname = input(
        "Public HTTPS URL configured for this tunnel (for example https://agent.example.com)",
    )?;
    let parsed = url::Url::parse(hostname.trim()).map_err(|error| {
        ChannelSetupError::Validation(format!("invalid Cloudflare tunnel URL: {error}"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(ChannelSetupError::Validation(
            "Cloudflare tunnel URL must use HTTPS and include a hostname".to_string(),
        ));
    }
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
        provider: Some(channel_setup::TUNNEL_PROVIDER_CLOUDFLARE.to_string()),
        cf_token,
        cf_hostname: Some(hostname.trim_end_matches('/').to_string()),
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
async fn test_tailscale_cli() -> bool {
    let binary = crate::util::resolve_binary("tailscale");
    let mut command = tokio::process::Command::new(&binary);
    command.arg("version");
    let output = thinclaw_platform::bounded_command_output(
        &mut command,
        std::time::Duration::from_secs(15),
        64 * 1024,
        64 * 1024,
    )
    .await;

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

async fn setup_tunnel_tailscale() -> Result<TunnelSettings, ChannelSetupError> {
    // Check if tailscale CLI is installed AND working.
    // On macOS, the App Store version installs a CLI shim that crashes with
    // BundleIdentifier errors when spawned from another process.
    let cli_working = test_tailscale_cli().await;

    if !cli_working {
        crate::setup::prompts::print_blank_line();

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

            crate::setup::prompts::print_blank_line();

            // Check if Homebrew is available for auto-install
            let has_brew = thinclaw_platform::find_executable_in_path("brew").is_some();

            if has_brew {
                if confirm(
                    "Install Tailscale CLI via Homebrew? (brew install tailscale)",
                    true,
                )? {
                    print_info("Installing tailscale via Homebrew (this may take a minute)...");
                    let mut command = tokio::process::Command::new("brew");
                    command.args(["install", "tailscale"]);
                    let install_result = thinclaw_platform::bounded_command_output(
                        &mut command,
                        std::time::Duration::from_secs(30 * 60),
                        2 * 1024 * 1024,
                        2 * 1024 * 1024,
                    )
                    .await;

                    match install_result {
                        Ok(output) if output.status.success() => {
                            if test_tailscale_cli().await {
                                print_success("Tailscale CLI installed and working!");
                            } else {
                                print_success("Tailscale CLI installed.");
                                print_info(
                                    "You may need to start the service: brew services start tailscale",
                                );
                            }
                        }
                        Ok(_) => {
                            print_error(
                                "Homebrew install failed. Try manually: brew install tailscale",
                            );
                            crate::setup::prompts::print_blank_line();
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
                crate::setup::prompts::print_blank_line();
                print_info("  Option 1: Install Homebrew, then: brew install tailscale");
                print_info("  Option 2: Download from https://tailscale.com/download/mac");
                crate::setup::prompts::print_blank_line();
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
            crate::setup::prompts::print_blank_line();
            if !confirm(
                "Continue configuring anyway? (install before starting the agent)",
                false,
            )? {
                return Ok(TunnelSettings::default());
            }
        }
    }

    crate::setup::prompts::print_blank_line();
    print_info("Tailscale offers two modes:");
    crate::setup::prompts::print_blank_line();
    print_info("  Funnel (public)  — Makes your agent reachable from the public internet.");
    print_info("                     Required for Telegram/Slack/Discord webhooks.");
    print_info("                     Your hostname (e.g. my-mac.tail1234.ts.net) becomes");
    print_info("                     publicly resolvable with a valid HTTPS certificate.");
    crate::setup::prompts::print_blank_line();
    print_info("  Serve (tailnet)  — Only reachable from devices on YOUR Tailscale network.");
    print_info("                     Great for private Web UI access from your phone/laptop,");
    print_info("                     but Telegram's servers CANNOT reach it (webhooks won't work,");
    print_info("                     Telegram will fall back to polling mode).");
    crate::setup::prompts::print_blank_line();

    let funnel = confirm(
        "Use Tailscale Funnel (public internet — needed for webhooks)?",
        true,
    )?;
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
        print_info(
            "You can switch to Funnel later by re-running setup or setting TUNNEL_TS_FUNNEL=true.",
        );
    }
    if !is_binary_installed("tailscale") {
        print_info("⚠ Remember to install 'tailscale' before running 'thinclaw run'.");
    }

    Ok(TunnelSettings {
        provider: Some(channel_setup::TUNNEL_PROVIDER_TAILSCALE.to_string()),
        ts_funnel: funnel,
        ts_hostname: hostname,
        ..Default::default()
    })
}

fn setup_tunnel_custom() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter a shell command to start your tunnel.");
    print_info("Use {port} and {host} as placeholders.");
    print_info("Example: bore local {port} --to bore.pub");
    crate::setup::prompts::print_blank_line();

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
        provider: Some(channel_setup::TUNNEL_PROVIDER_CUSTOM.to_string()),
        custom_command: Some(command),
        custom_health_url: health_url,
        custom_url_pattern: url_pattern,
        ..Default::default()
    })
}

fn setup_tunnel_static() -> Result<TunnelSettings, ChannelSetupError> {
    print_info("Enter the public URL of your externally managed tunnel.");
    crate::setup::prompts::print_blank_line();

    let tunnel_url = input("Tunnel URL (e.g., https://abc123.ngrok.io)")?;

    let tunnel_url = match channel_setup::normalize_static_tunnel_url(&tunnel_url) {
        Ok(tunnel_url) => tunnel_url,
        Err(error) => {
            print_error("URL must start with https:// (webhooks require HTTPS)");
            return Err(ChannelSetupError::Validation(error));
        }
    };

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
    thinclaw_platform::find_executable_in_path(name).is_some()
        || std::path::Path::new(&crate::util::resolve_binary(name)).is_file()
}
