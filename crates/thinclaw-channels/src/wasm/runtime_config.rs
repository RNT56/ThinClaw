use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use thinclaw_secrets::{SecretAccessContext, SecretsStore};

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
    tailscale_serve_tailnet_only: bool,
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

    if tailscale_serve_tailnet_only {
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
    #[allow(clippy::too_many_arguments)]
    pub fn from_runtime_inputs(
        tunnel_url: Option<String>,
        telegram_owner_id: Option<i64>,
        telegram_stream_mode: Option<String>,
        telegram_transport_mode: String,
        discord_stream_mode: Option<String>,
        tailscale_serve_tailnet_only: bool,
    ) -> Self {
        let host_status =
            telegram_webhook_host_status(tunnel_url.as_deref(), tailscale_serve_tailnet_only);
        let host_webhook_capable = host_status.usable;
        let telegram_tunnel_url = if telegram_transport_mode == "polling" || !host_webhook_capable {
            None
        } else {
            tunnel_url.clone()
        };

        Self {
            tunnel_url,
            telegram_tunnel_url,
            telegram_owner_id,
            telegram_stream_mode,
            telegram_transport_mode,
            telegram_host_webhook_capable: host_webhook_capable,
            telegram_host_transport_reason: host_status.reason,
            discord_stream_mode,
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
                // Telegram only enters public webhook mode when its platform-native
                // secret token is available. Without it, keep the public tunnel in
                // diagnostics but force polling so ingress cannot silently become
                // unauthenticated.
                let webhook_secret_available = webhook_secret
                    .map(str::trim)
                    .is_some_and(|secret| !secret.is_empty());
                let host_webhook_capable =
                    self.telegram_host_webhook_capable && webhook_secret_available;
                let host_transport_reason = if self.telegram_host_webhook_capable
                    && !webhook_secret_available
                {
                    Some(
                        "telegram_webhook_secret is required before public webhook delivery can be enabled"
                            .to_string(),
                    )
                } else {
                    self.telegram_host_transport_reason.clone()
                };
                let telegram_tunnel_url =
                    if host_webhook_capable && self.telegram_transport_mode != "polling" {
                        self.telegram_tunnel_url.clone()
                    } else {
                        None
                    };

                updates.insert(
                    "transport_preference".to_string(),
                    serde_json::Value::String(self.telegram_transport_mode.clone()),
                );
                updates.insert(
                    "host_webhook_capable".to_string(),
                    serde_json::Value::Bool(host_webhook_capable),
                );
                updates.insert(
                    "host_transport_reason".to_string(),
                    host_transport_reason
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
                        _ => host_transport_reason
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
                    telegram_tunnel_url
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

/// Load the exact credential set declared by the channel manifest.
///
/// The snapshot is replaced atomically. If any required credential cannot be
/// loaded, the previous snapshot is cleared so a revoked or deleted secret
/// cannot remain usable by a running channel.
pub async fn inject_channel_credentials_from_secrets(
    channel: &Arc<WasmChannel>,
    secrets: &(dyn SecretsStore + Send + Sync),
    channel_name: &str,
    user_id: &str,
) -> Result<usize, String> {
    if channel.channel_name() != channel_name {
        channel.clear_credentials().await;
        return Err("Channel credential namespace does not match the active channel".to_string());
    }

    let mut secret_names = channel
        .capabilities()
        .tool_capabilities
        .http
        .as_ref()
        .map(|http| {
            http.credentials
                .values()
                .map(|mapping| mapping.secret_name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    secret_names.sort();
    secret_names.dedup();

    let mut snapshot = HashMap::with_capacity(secret_names.len());
    for secret_name in secret_names {
        let Some(placeholder) =
            crate::wasm::capabilities::credential_placeholder_name(&secret_name)
        else {
            channel.clear_credentials().await;
            return Err("Channel manifest contains an invalid credential name".to_string());
        };

        let decrypted = match secrets
            .get_for_injection(
                user_id,
                &secret_name,
                SecretAccessContext::new(
                    "wasm.channel_runtime_config",
                    "channel_credential_injection",
                ),
            )
            .await
        {
            Ok(d) => d,
            Err(_) => {
                channel.clear_credentials().await;
                return Err("Failed to load a required channel credential".to_string());
            }
        };
        snapshot.insert(placeholder, decrypted.expose().to_string());
    }

    channel.replace_credentials(snapshot).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use secrecy::SecretString;
    use thinclaw_secrets::{CreateSecretParams, InMemorySecretsStore, SecretsCrypto, SecretsStore};

    use super::{WasmChannelHostConfig, telegram_webhook_host_status};
    use crate::pairing::PairingStore;
    use crate::wasm::capabilities::{
        ChannelCapabilities, CredentialLocation, CredentialMapping, HttpCapability,
    };
    use crate::wasm::runtime::{
        PreparedChannelModule, WasmChannelRuntime, WasmChannelRuntimeConfig,
    };
    use crate::wasm::wrapper::WasmChannel;

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

    #[tokio::test]
    async fn credential_refresh_loads_only_manifest_secrets_and_clears_on_revocation() {
        let runtime =
            Arc::new(WasmChannelRuntime::new(WasmChannelRuntimeConfig::for_testing()).unwrap());
        let prepared = Arc::new(PreparedChannelModule::for_testing("test", "test channel"));
        let mut capabilities = ChannelCapabilities::for_channel("test");
        capabilities.tool_capabilities.http = Some(HttpCapability::default().with_credential(
            "service",
            CredentialMapping {
                secret_name: "test_service_token".to_string(),
                location: CredentialLocation::Bearer,
                host_patterns: vec!["api.example.com".to_string()],
            },
        ));
        let channel = Arc::new(WasmChannel::new(
            runtime,
            prepared,
            capabilities,
            "{}".to_string(),
            None,
            Arc::new(PairingStore::new()),
        ));
        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from(
                "0123456789abcdef0123456789abcdef".to_string(),
            ))
            .unwrap(),
        );
        let secrets = InMemorySecretsStore::new(crypto);
        secrets
            .create(
                "user",
                CreateSecretParams::new("test_service_token", "active-token"),
            )
            .await
            .unwrap();
        secrets
            .create(
                "user",
                CreateSecretParams::new("test_unmapped_secret", "must-not-load"),
            )
            .await
            .unwrap();

        let count =
            super::inject_channel_credentials_from_secrets(&channel, &secrets, "test", "user")
                .await
                .unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            channel.get_credentials().await,
            std::collections::HashMap::from([(
                "TEST_SERVICE_TOKEN".to_string(),
                "active-token".to_string(),
            )])
        );

        assert!(secrets.delete("user", "test_service_token").await.unwrap());
        assert!(
            super::inject_channel_credentials_from_secrets(&channel, &secrets, "test", "user",)
                .await
                .is_err()
        );
        assert!(channel.get_credentials().await.is_empty());
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
    fn telegram_without_webhook_secret_forces_safe_polling() {
        let config = WasmChannelHostConfig {
            tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_tunnel_url: Some("https://agent.example.com".to_string()),
            telegram_owner_id: None,
            telegram_stream_mode: None,
            telegram_transport_mode: "auto".to_string(),
            telegram_host_webhook_capable: true,
            telegram_host_transport_reason: None,
            discord_stream_mode: None,
        };

        let updates = config.updates_for_channel("telegram", None);

        assert_eq!(updates.get("tunnel_url"), Some(&serde_json::Value::Null));
        assert_eq!(
            updates
                .get("host_webhook_capable")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
        assert!(
            updates
                .get("transport_reason")
                .and_then(|value| value.as_str())
                .is_some_and(|reason| reason.contains("telegram_webhook_secret"))
        );
    }

    #[test]
    fn telegram_webhook_host_status_rejects_tailscale_serve() {
        let status = telegram_webhook_host_status(Some("https://agent.ts.net"), true);

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
        let status = telegram_webhook_host_status(Some("https://127.0.0.1"), false);
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
