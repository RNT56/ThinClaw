use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use crate::secrets::SecretsStore;

use super::wrapper::WasmChannel;

/// Runtime values the host injects into WASM channels before startup.
///
/// These are not persisted in the channel artifact itself; they come from the
/// current ThinClaw process and must therefore be re-applied on cold start,
/// hot activation, and hot reload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WasmChannelHostConfig {
    pub tunnel_url: Option<String>,
    pub telegram_tunnel_url: Option<String>,
    pub telegram_owner_id: Option<i64>,
    pub telegram_stream_mode: Option<String>,
    pub telegram_transport_mode: String,
    pub telegram_host_webhook_capable: bool,
    pub telegram_host_transport_reason: Option<String>,
    pub discord_stream_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramWebhookHostStatus {
    usable: bool,
    reason: Option<String>,
}

impl TelegramWebhookHostStatus {
    fn usable() -> Self {
        Self {
            usable: true,
            reason: None,
        }
    }

    fn unusable(reason: impl Into<String>) -> Self {
        Self {
            usable: false,
            reason: Some(reason.into()),
        }
    }
}

fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || (v4.octets()[0] == 100 && v4.octets()[1] >= 64 && v4.octets()[1] <= 127)
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn telegram_webhook_host_status(
    public_url: Option<&str>,
    tunnel_provider: Option<&crate::config::tunnel::TunnelProviderConfig>,
) -> TelegramWebhookHostStatus {
    let Some(raw_public_url) = public_url.map(str::trim).filter(|value| !value.is_empty()) else {
        return TelegramWebhookHostStatus::unusable("no public HTTPS webhook URL is configured");
    };

    let parsed = match url::Url::parse(raw_public_url) {
        Ok(parsed) => parsed,
        Err(error) => {
            return TelegramWebhookHostStatus::unusable(format!(
                "public webhook URL is invalid: {}",
                error
            ));
        }
    };

    if parsed.scheme() != "https" {
        return TelegramWebhookHostStatus::unusable("public webhook URL must use https");
    }

    let Some(host) = parsed.host() else {
        return TelegramWebhookHostStatus::unusable("public webhook URL is missing a host");
    };

    if tunnel_provider
        .filter(|provider| provider.provider.eq_ignore_ascii_case("tailscale"))
        .and_then(|provider| provider.tailscale.as_ref())
        .is_some_and(|tailscale| !tailscale.funnel)
    {
        let looks_like_tailnet_url = match host {
            url::Host::Domain(domain) => domain.trim().to_ascii_lowercase().ends_with(".ts.net"),
            _ => false,
        };
        if looks_like_tailnet_url {
            return TelegramWebhookHostStatus::unusable(
                "managed Tailscale Serve is tailnet-only; Telegram webhooks require public ingress",
            );
        }
    }

    match host {
        url::Host::Domain(domain) => {
            let domain = domain.trim().to_ascii_lowercase();
            if domain == "localhost" || domain.ends_with(".localhost") || domain.ends_with(".local")
            {
                TelegramWebhookHostStatus::unusable(
                    "public webhook URL resolves to a local-only hostname",
                )
            } else {
                TelegramWebhookHostStatus::usable()
            }
        }
        url::Host::Ipv4(ip) => {
            let ip = IpAddr::V4(ip);
            if ip.is_loopback() || is_private_ip(&ip) {
                TelegramWebhookHostStatus::unusable(
                    "public webhook URL points to a private or loopback IP address",
                )
            } else {
                TelegramWebhookHostStatus::usable()
            }
        }
        url::Host::Ipv6(ip) => {
            let ip = IpAddr::V6(ip);
            if ip.is_loopback() || is_private_ip(&ip) {
                TelegramWebhookHostStatus::unusable(
                    "public webhook URL points to a private or loopback IP address",
                )
            } else {
                TelegramWebhookHostStatus::usable()
            }
        }
    }
}

impl WasmChannelHostConfig {
    pub fn as_core(&self) -> thinclaw_channels::wasm::WasmChannelHostConfig {
        thinclaw_channels::wasm::WasmChannelHostConfig {
            tunnel_url: self.tunnel_url.clone(),
            telegram_tunnel_url: self.telegram_tunnel_url.clone(),
            telegram_owner_id: self.telegram_owner_id,
            telegram_stream_mode: self.telegram_stream_mode.clone(),
            telegram_transport_mode: self.telegram_transport_mode.clone(),
            telegram_host_webhook_capable: self.telegram_host_webhook_capable,
            telegram_host_transport_reason: self.telegram_host_transport_reason.clone(),
            discord_stream_mode: self.discord_stream_mode.clone(),
        }
    }

    pub fn from_config(config: &crate::config::Config) -> Self {
        let telegram_transport_mode = config.channels.telegram_transport_mode.clone();
        let host_status = telegram_webhook_host_status(
            config.tunnel.public_url.as_deref(),
            config.tunnel.provider.as_ref(),
        );
        let host_webhook_capable = host_status.usable;
        let telegram_tunnel_url = if telegram_transport_mode == "polling" || !host_webhook_capable {
            None
        } else {
            config.tunnel.public_url.clone()
        };

        Self {
            tunnel_url: config.tunnel.public_url.clone(),
            telegram_tunnel_url,
            telegram_owner_id: config.channels.telegram_owner_id,
            telegram_stream_mode: config.channels.telegram_stream_mode.clone(),
            telegram_transport_mode,
            telegram_host_webhook_capable: host_webhook_capable,
            telegram_host_transport_reason: host_status.reason,
            discord_stream_mode: config.channels.discord_stream_mode.clone(),
        }
    }

    pub fn updates_for_channel(
        &self,
        channel_name: &str,
        webhook_secret: Option<&str>,
    ) -> HashMap<String, serde_json::Value> {
        let mut updates = HashMap::new();

        if let Some(secret) = webhook_secret {
            updates.insert(
                "webhook_secret".to_string(),
                serde_json::Value::String(secret.to_string()),
            );
        }

        match channel_name {
            "telegram" => {
                updates.insert(
                    "transport_preference".to_string(),
                    serde_json::Value::String(self.telegram_transport_mode.clone()),
                );
                updates.insert(
                    "host_webhook_capable".to_string(),
                    serde_json::Value::Bool(self.telegram_host_webhook_capable),
                );
                updates.insert(
                    "host_transport_reason".to_string(),
                    self.telegram_host_transport_reason
                        .clone()
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                );
                updates.insert(
                    "transport_reason".to_string(),
                    match self.telegram_transport_mode.as_str() {
                        "polling" => {
                            serde_json::Value::String("operator forced polling".to_string())
                        }
                        _ => self
                            .telegram_host_transport_reason
                            .clone()
                            .map(serde_json::Value::String)
                            .unwrap_or(serde_json::Value::Null),
                    },
                );
                updates.insert(
                    "host_tunnel_url".to_string(),
                    self.tunnel_url
                        .clone()
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                );
                updates.insert(
                    "tunnel_url".to_string(),
                    self.telegram_tunnel_url
                        .clone()
                        .map(serde_json::Value::String)
                        .unwrap_or(serde_json::Value::Null),
                );

                if let Some(owner_id) = self.telegram_owner_id {
                    updates.insert("owner_id".to_string(), serde_json::json!(owner_id));
                }

                if let Some(ref stream_mode) = self.telegram_stream_mode
                    && !stream_mode.is_empty()
                {
                    updates.insert(
                        "stream_mode".to_string(),
                        serde_json::Value::String(stream_mode.clone()),
                    );
                }
            }
            "discord" => {
                if let Some(ref tunnel_url) = self.tunnel_url {
                    updates.insert(
                        "tunnel_url".to_string(),
                        serde_json::Value::String(tunnel_url.clone()),
                    );
                }

                if let Some(ref stream_mode) = self.discord_stream_mode
                    && !stream_mode.is_empty()
                {
                    updates.insert(
                        "stream_mode".to_string(),
                        serde_json::Value::String(stream_mode.clone()),
                    );
                }
            }
            _ => {
                if let Some(ref tunnel_url) = self.tunnel_url {
                    updates.insert(
                        "tunnel_url".to_string(),
                        serde_json::Value::String(tunnel_url.clone()),
                    );
                }
            }
        }

        updates
    }
}

pub async fn apply_channel_host_config(
    channel: &Arc<WasmChannel>,
    channel_name: &str,
    host_config: &WasmChannelHostConfig,
    webhook_secret: Option<&str>,
) -> usize {
    let updates = host_config.updates_for_channel(channel_name, webhook_secret);
    let update_count = updates.len();
    if update_count > 0 {
        channel.update_config(updates).await;
    }
    update_count
}

/// Inject credentials for a channel based on naming convention.
///
/// Looks for secrets matching the pattern `{channel_name}_*` and injects them
/// as credential placeholders (e.g., `telegram_bot_token` -> `{TELEGRAM_BOT_TOKEN}`).
///
/// Returns the number of credentials injected.
pub async fn inject_channel_credentials_from_secrets(
    channel: &Arc<WasmChannel>,
    secrets: &(dyn SecretsStore + Send + Sync),
    channel_name: &str,
    user_id: &str,
) -> Result<usize, String> {
    let all_secrets = secrets
        .list(user_id)
        .await
        .map_err(|e| format!("Failed to list secrets: {}", e))?;

    let prefix = format!("{}_", channel_name);
    let mut count = 0;

    for secret_meta in all_secrets {
        if !secret_meta.name.starts_with(&prefix) {
            continue;
        }

        let decrypted = match secrets
            .get_for_injection(
                user_id,
                &secret_meta.name,
                crate::secrets::SecretAccessContext::new(
                    "wasm.channel_runtime_config",
                    "channel_credential_injection",
                ),
            )
            .await
        {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(
                    secret = %secret_meta.name,
                    error = %e,
                    "Failed to decrypt secret for channel credential injection"
                );
                continue;
            }
        };

        let placeholder = secret_meta.name.to_uppercase();
        channel
            .set_credential(&placeholder, decrypted.expose().to_string())
            .await;
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::{WasmChannelHostConfig, telegram_webhook_host_status};

    #[test]
    fn telegram_updates_include_owner_stream_and_shared_runtime_values() {
        let config = WasmChannelHostConfig {
            tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_owner_id: Some(684480568),
            telegram_stream_mode: Some("streaming".to_string()),
            telegram_transport_mode: "auto".to_string(),
            telegram_host_webhook_capable: true,
            telegram_host_transport_reason: None,
            discord_stream_mode: Some("chunks".to_string()),
        };

        let updates = config.updates_for_channel("telegram", Some("secret-123"));

        assert_eq!(
            updates.get("tunnel_url").and_then(|value| value.as_str()),
            Some("https://agent.example.com")
        );
        assert_eq!(
            updates
                .get("host_tunnel_url")
                .and_then(|value| value.as_str()),
            Some("https://agent.example.com")
        );
        assert_eq!(
            updates
                .get("webhook_secret")
                .and_then(|value| value.as_str()),
            Some("secret-123")
        );
        assert_eq!(
            updates.get("owner_id").and_then(|value| value.as_i64()),
            Some(684480568)
        );
        assert_eq!(
            updates.get("stream_mode").and_then(|value| value.as_str()),
            Some("streaming")
        );
        assert_eq!(
            updates
                .get("transport_preference")
                .and_then(|value| value.as_str()),
            Some("auto")
        );
        assert_eq!(
            updates
                .get("host_webhook_capable")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn discord_updates_use_discord_stream_mode_only() {
        let config = WasmChannelHostConfig {
            tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_owner_id: Some(42),
            telegram_stream_mode: Some("streaming".to_string()),
            telegram_transport_mode: "auto".to_string(),
            telegram_host_webhook_capable: true,
            telegram_host_transport_reason: None,
            discord_stream_mode: Some("chunks".to_string()),
        };

        let updates = config.updates_for_channel("discord", None);

        assert_eq!(
            updates.get("tunnel_url").and_then(|value| value.as_str()),
            Some("https://agent.example.com")
        );
        assert_eq!(
            updates.get("stream_mode").and_then(|value| value.as_str()),
            Some("chunks")
        );
        assert!(!updates.contains_key("owner_id"));
        assert!(!updates.contains_key("webhook_secret"));
    }

    #[test]
    fn unrelated_channels_only_get_shared_runtime_values() {
        let config = WasmChannelHostConfig {
            tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_owner_id: Some(42),
            telegram_stream_mode: Some("streaming".to_string()),
            telegram_transport_mode: "auto".to_string(),
            telegram_host_webhook_capable: true,
            telegram_host_transport_reason: None,
            discord_stream_mode: Some("chunks".to_string()),
        };

        let updates = config.updates_for_channel("slack", Some("secret-123"));

        assert_eq!(updates.len(), 2);
        assert_eq!(
            updates.get("tunnel_url").and_then(|value| value.as_str()),
            Some("https://agent.example.com")
        );
        assert_eq!(
            updates
                .get("webhook_secret")
                .and_then(|value| value.as_str()),
            Some("secret-123")
        );
        assert!(!updates.contains_key("owner_id"));
        assert!(!updates.contains_key("stream_mode"));
    }

    #[test]
    fn telegram_polling_preference_suppresses_webhook_url_but_keeps_host_url() {
        let config = WasmChannelHostConfig {
            tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_tunnel_url: None,
            telegram_owner_id: None,
            telegram_stream_mode: None,
            telegram_transport_mode: "polling".to_string(),
            telegram_host_webhook_capable: true,
            telegram_host_transport_reason: None,
            discord_stream_mode: None,
        };

        let updates = config.updates_for_channel("telegram", None);

        assert_eq!(updates.get("tunnel_url"), Some(&serde_json::Value::Null));
        assert_eq!(
            updates
                .get("host_tunnel_url")
                .and_then(|value| value.as_str()),
            Some("https://agent.example.com")
        );
        assert_eq!(
            updates
                .get("transport_reason")
                .and_then(|value| value.as_str()),
            Some("operator forced polling")
        );
    }

    #[test]
    fn telegram_webhook_host_status_rejects_tailscale_serve() {
        let provider = crate::config::tunnel::TunnelProviderConfig {
            provider: "tailscale".to_string(),
            tailscale: Some(crate::config::tunnel::TailscaleTunnelConfig {
                funnel: false,
                hostname: None,
            }),
            ..Default::default()
        };

        let status = telegram_webhook_host_status(Some("https://agent.ts.net"), Some(&provider));

        assert!(!status.usable);
        assert!(
            status
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("tailnet-only")
        );
    }

    #[test]
    fn telegram_webhook_host_status_rejects_private_addresses() {
        let status = telegram_webhook_host_status(Some("https://127.0.0.1"), None);
        assert!(!status.usable);
        assert!(
            status
                .reason
                .as_deref()
                .unwrap_or_default()
                .contains("private or loopback")
        );
    }
}
