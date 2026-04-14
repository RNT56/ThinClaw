use crate::config::helpers::optional_env;
use crate::error::ConfigError;
use crate::settings::Settings;

// ── Tunnel data types ────────────────────────────────────────────────
//
// These config structs were originally defined in `crate::tunnel::mod.rs`.
// They live here now so that `config::TunnelConfig` can be resolved
// without compiling the `tunnel` runtime module, which in turn allows
// `pub mod tunnel` in lib.rs to be feature-gated behind `#[cfg(feature = "tunnel")]`.
//
// The `tunnel` module re-exports these types for backwards compatibility.

/// Provider-specific config for Cloudflare tunnels.
#[derive(Debug, Clone, Default)]
pub struct CloudflareTunnelConfig {
    /// Token from the Cloudflare Zero Trust dashboard.
    pub token: String,
}

/// Provider-specific config for Tailscale tunnels.
#[derive(Debug, Clone, Default)]
pub struct TailscaleTunnelConfig {
    /// Use `tailscale funnel` (public) instead of `tailscale serve` (tailnet).
    pub funnel: bool,
    /// Override the hostname (default: auto-detect from `tailscale status`).
    pub hostname: Option<String>,
}

/// Provider-specific config for ngrok tunnels.
#[derive(Debug, Clone, Default)]
pub struct NgrokTunnelConfig {
    /// ngrok auth token (required).
    pub auth_token: String,
    /// Custom domain (requires ngrok paid plan).
    pub domain: Option<String>,
}

/// Provider-specific config for custom tunnel commands.
#[derive(Debug, Clone, Default)]
pub struct CustomTunnelConfig {
    /// Shell command with `{port}` and `{host}` placeholders.
    pub start_command: String,
    /// HTTP endpoint to poll for health checks.
    pub health_url: Option<String>,
    /// Substring to match in stdout for URL extraction.
    pub url_pattern: Option<String>,
}

/// Full tunnel provider configuration.
#[derive(Debug, Clone, Default)]
pub struct TunnelProviderConfig {
    /// Provider name: "none", "cloudflare", "tailscale", "ngrok", "custom".
    pub provider: String,
    pub cloudflare: Option<CloudflareTunnelConfig>,
    pub tailscale: Option<TailscaleTunnelConfig>,
    pub ngrok: Option<NgrokTunnelConfig>,
    pub custom: Option<CustomTunnelConfig>,
}

/// Tunnel configuration for exposing the agent to the internet.
///
/// Used by channels and tools that need public webhook endpoints.
/// The tunnel URL is shared across all channels (Telegram, Slack, etc.).
///
/// Two modes:
/// - **Static URL** (`TUNNEL_URL`): set the public URL directly (manual tunnel)
/// - **Managed provider** (`TUNNEL_PROVIDER`): lifecycle-managed tunnel process
///
/// When a managed provider is configured _and_ no static URL is set,
/// the gateway starts the tunnel on boot and populates `public_url`.
#[derive(Debug, Clone, Default)]
pub struct TunnelConfig {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    /// Set statically via `TUNNEL_URL` or populated at runtime by a managed tunnel.
    pub public_url: Option<String>,
    /// Provider configuration for lifecycle-managed tunnels.
    /// `None` when using a static URL or no tunnel at all.
    pub provider: Option<TunnelProviderConfig>,
}

impl TunnelConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let public_url = optional_env("TUNNEL_URL")?
            .or_else(|| settings.tunnel.public_url.clone().filter(|s| !s.is_empty()));

        if let Some(ref url) = public_url
            && !url.starts_with("https://")
        {
            return Err(ConfigError::InvalidValue {
                key: "TUNNEL_URL".to_string(),
                message: "must start with https:// (webhooks require HTTPS)".to_string(),
            });
        }

        // Resolve managed tunnel provider config.
        // Priority: env var > settings > default (none).
        let provider_name = optional_env("TUNNEL_PROVIDER")?
            .or_else(|| settings.tunnel.provider.clone())
            .unwrap_or_default();

        let provider = if provider_name.is_empty() || provider_name == "none" {
            None
        } else {
            Some(TunnelProviderConfig {
                provider: provider_name.clone(),
                cloudflare: optional_env("TUNNEL_CF_TOKEN")?
                    .or_else(|| settings.tunnel.cf_token.clone())
                    .map(|token| CloudflareTunnelConfig { token }),
                tailscale: Some(TailscaleTunnelConfig {
                    funnel: optional_env("TUNNEL_TS_FUNNEL")?
                        .map(|s| s == "true" || s == "1")
                        .unwrap_or(settings.tunnel.ts_funnel),
                    hostname: optional_env("TUNNEL_TS_HOSTNAME")?
                        .or_else(|| settings.tunnel.ts_hostname.clone()),
                }),
                ngrok: {
                    let ngrok_domain = optional_env("TUNNEL_NGROK_DOMAIN")?
                        .or_else(|| settings.tunnel.ngrok_domain.clone());
                    optional_env("TUNNEL_NGROK_TOKEN")?
                        .or_else(|| settings.tunnel.ngrok_token.clone())
                        .map(|auth_token| NgrokTunnelConfig {
                            auth_token,
                            domain: ngrok_domain,
                        })
                },
                custom: {
                    let health_url = optional_env("TUNNEL_CUSTOM_HEALTH_URL")?
                        .or_else(|| settings.tunnel.custom_health_url.clone());
                    let url_pattern = optional_env("TUNNEL_CUSTOM_URL_PATTERN")?
                        .or_else(|| settings.tunnel.custom_url_pattern.clone());
                    optional_env("TUNNEL_CUSTOM_COMMAND")?
                        .or_else(|| settings.tunnel.custom_command.clone())
                        .map(|start_command| CustomTunnelConfig {
                            start_command,
                            health_url,
                            url_pattern,
                        })
                },
            })
        };

        Ok(Self {
            public_url,
            provider,
        })
    }

    /// Check if a tunnel is configured (static URL or managed provider).
    pub fn is_enabled(&self) -> bool {
        self.public_url.is_some() || self.provider.is_some()
    }

    /// Get the webhook URL for a given path.
    pub fn webhook_url(&self, path: &str) -> Option<String> {
        self.public_url.as_ref().map(|base| {
            let base = base.trim_end_matches('/');
            let path = path.trim_start_matches('/');
            format!("{}/{}", base, path)
        })
    }
}
