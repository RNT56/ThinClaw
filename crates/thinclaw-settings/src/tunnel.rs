use super::*;

/// Tunnel settings for public webhook endpoints.
///
/// The tunnel URL is shared across all channels that need webhooks.
/// Two modes:
/// - **Static URL**: `public_url` set directly (manual tunnel management).
/// - **Managed provider**: `provider` is set and the agent starts/stops the
///   tunnel process automatically at boot/shutdown.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TunnelSettings {
    /// Public URL from tunnel provider (e.g., "https://abc123.ngrok.io").
    /// When set without a provider, treated as a static (externally managed) URL.
    #[serde(default)]
    pub public_url: Option<String>,

    /// Managed tunnel provider: "ngrok", "cloudflare", "tailscale", "custom".
    #[serde(default)]
    pub provider: Option<String>,

    /// Cloudflare tunnel token.
    #[serde(default)]
    pub cf_token: Option<String>,

    /// Public HTTPS hostname routed to the managed Cloudflare tunnel.
    #[serde(default)]
    pub cf_hostname: Option<String>,

    /// ngrok auth token.
    #[serde(default)]
    pub ngrok_token: Option<String>,

    /// ngrok custom domain (paid plans).
    #[serde(default)]
    pub ngrok_domain: Option<String>,

    /// Use Tailscale Funnel (public) instead of Serve (tailnet-only).
    #[serde(default)]
    pub ts_funnel: bool,

    /// Tailscale hostname override.
    #[serde(default)]
    pub ts_hostname: Option<String>,

    /// Shell command for custom tunnel (with `{port}` / `{host}` placeholders).
    #[serde(default)]
    pub custom_command: Option<String>,

    /// Health check URL for custom tunnel.
    #[serde(default)]
    pub custom_health_url: Option<String>,

    /// Substring pattern to extract URL from custom tunnel stdout.
    #[serde(default)]
    pub custom_url_pattern: Option<String>,
}

impl std::fmt::Debug for TunnelSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelSettings")
            .field("public_url_configured", &self.public_url.is_some())
            .field("provider", &self.provider)
            .field(
                "cf_token",
                &crate::redaction::RedactedOption(&self.cf_token),
            )
            .field("cf_hostname", &self.cf_hostname)
            .field(
                "ngrok_token",
                &crate::redaction::RedactedOption(&self.ngrok_token),
            )
            .field("ngrok_domain", &self.ngrok_domain)
            .field("ts_funnel", &self.ts_funnel)
            .field("ts_hostname", &self.ts_hostname)
            .field("custom_command_configured", &self.custom_command.is_some())
            .field(
                "custom_health_url_configured",
                &self.custom_health_url.is_some(),
            )
            .field(
                "custom_url_pattern_configured",
                &self.custom_url_pattern.is_some(),
            )
            .finish()
    }
}
