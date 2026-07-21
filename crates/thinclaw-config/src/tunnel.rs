//! Tunnel configuration for exposing local services to webhook providers.

use thinclaw_settings::Settings;
use thinclaw_types::error::ConfigError;

use crate::helpers::optional_env;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct CloudflareTunnelConfig {
    /// Token from the Cloudflare Zero Trust dashboard.
    pub token: String,
    /// Public HTTPS hostname configured for the named tunnel.
    pub hostname: String,
}

impl std::fmt::Debug for CloudflareTunnelConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CloudflareTunnelConfig")
            .field("token", &"[REDACTED]")
            .field("hostname", &self.hostname)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TailscaleTunnelConfig {
    /// Use `tailscale funnel` (public) instead of `tailscale serve` (tailnet).
    pub funnel: bool,
    /// Override the hostname.
    pub hostname: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct NgrokTunnelConfig {
    /// ngrok auth token.
    pub auth_token: String,
    /// Custom domain.
    pub domain: Option<String>,
}

impl std::fmt::Debug for NgrokTunnelConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NgrokTunnelConfig")
            .field("auth_token", &"[REDACTED]")
            .field("domain", &self.domain)
            .finish()
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct CustomTunnelConfig {
    /// Shell command with `{port}` and `{host}` placeholders.
    pub start_command: String,
    /// HTTP endpoint to poll for health checks.
    pub health_url: Option<String>,
    /// Substring to match in stdout for URL extraction.
    pub url_pattern: Option<String>,
}

impl std::fmt::Debug for CustomTunnelConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CustomTunnelConfig")
            .field("start_command", &"[REDACTED COMMAND]")
            .field(
                "health_url",
                &self.health_url.as_ref().map(|_| "[REDACTED URL]"),
            )
            .field("url_pattern", &self.url_pattern)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TunnelProviderConfig {
    /// Provider name: "none", "cloudflare", "tailscale", "ngrok", "custom".
    pub provider: String,
    pub cloudflare: Option<CloudflareTunnelConfig>,
    pub tailscale: Option<TailscaleTunnelConfig>,
    pub ngrok: Option<NgrokTunnelConfig>,
    pub custom: Option<CustomTunnelConfig>,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct TunnelConfig {
    /// Public URL from tunnel provider.
    pub public_url: Option<String>,
    /// Provider configuration for lifecycle-managed tunnels.
    pub provider: Option<TunnelProviderConfig>,
}

impl std::fmt::Debug for TunnelConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TunnelConfig")
            .field(
                "public_url",
                &self.public_url.as_ref().map(|_| "[REDACTED URL]"),
            )
            .field("provider", &self.provider)
            .finish()
    }
}

impl TunnelConfig {
    pub fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
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

        let provider_name = optional_env("TUNNEL_PROVIDER")?
            .or_else(|| settings.tunnel.provider.clone())
            .unwrap_or_default();

        let provider = if provider_name.is_empty() || provider_name == "none" {
            None
        } else {
            Some(TunnelProviderConfig {
                provider: provider_name.clone(),
                cloudflare: match optional_env("TUNNEL_CF_TOKEN")?
                    .or_else(|| settings.tunnel.cf_token.clone())
                {
                    Some(token) => {
                        let hostname = optional_env("TUNNEL_CF_HOSTNAME")?
                            .or_else(|| settings.tunnel.cf_hostname.clone())
                            .ok_or_else(|| ConfigError::InvalidValue {
                                key: "TUNNEL_CF_HOSTNAME".to_string(),
                                message: "is required for a managed Cloudflare named tunnel"
                                    .to_string(),
                            })?;
                        let parsed = url::Url::parse(&hostname).map_err(|error| {
                            ConfigError::InvalidValue {
                                key: "TUNNEL_CF_HOSTNAME".to_string(),
                                message: format!("must be a valid HTTPS URL: {error}"),
                            }
                        })?;
                        if parsed.scheme() != "https"
                            || parsed.host_str().is_none()
                            || !parsed.username().is_empty()
                            || parsed.password().is_some()
                            || parsed.query().is_some()
                            || parsed.fragment().is_some()
                        {
                            return Err(ConfigError::InvalidValue {
                                key: "TUNNEL_CF_HOSTNAME".to_string(),
                                message:
                                    "must be an HTTPS URL without credentials, query, or fragment"
                                        .to_string(),
                            });
                        }
                        Some(CloudflareTunnelConfig { token, hostname })
                    }
                    None => None,
                },
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

    pub fn is_enabled(&self) -> bool {
        self.public_url.is_some() || self.provider.is_some()
    }

    pub fn webhook_url(&self, path: &str) -> Option<String> {
        self.public_url.as_ref().map(|base| {
            let base = base.trim_end_matches('/');
            let path = path.trim_start_matches('/');
            format!("{}/{}", base, path)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_url_joins_base_and_path() {
        let config = TunnelConfig {
            public_url: Some("https://example.test/".to_string()),
            provider: None,
        };

        assert_eq!(
            config.webhook_url("/hooks/telegram").as_deref(),
            Some("https://example.test/hooks/telegram")
        );
    }
}
