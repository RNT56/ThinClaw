//! Channel continuity messaging and non-destructive channel verification
//! (configuration + reachability checks per enabled channel).

use secrecy::ExposeSecret;
use thinclaw_tools_core::{OutboundUrlGuardOptions, validate_outbound_url_pinned_async};

use crate::settings::{OnboardingFollowupCategory, OnboardingFollowupStatus};
use crate::setup::prompts::{print_info, print_success, print_warning};

use super::{FollowupDraft, SetupError, SetupWizard};

const MAX_CHANNEL_VERIFICATION_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CHANNEL_VERIFICATION_URL_BYTES: usize = 16 * 1024;
const MAX_CHANNEL_AUTH_TOKEN_BYTES: usize = 64 * 1024;
const MAX_CHANNEL_DNS_ADDRESSES: usize = 64;
const MAX_NOSTR_VERIFICATION_RELAYS: usize = 32;
const MAX_NOSTR_RELAY_URL_BYTES: usize = 4 * 1024;
const MAX_NOSTR_RELAYS_CSV_BYTES: usize = 128 * 1024;

impl SetupWizard {
    pub(super) fn configured_channel_names(&self) -> Vec<String> {
        let mut channels = Vec::new();
        if self.settings.channels.http_enabled {
            channels.push("http".to_string());
        }
        if self.settings.channels.signal_enabled {
            channels.push("signal".to_string());
        }
        if self.settings.channels.discord_enabled {
            channels.push("discord".to_string());
        }
        if self.settings.channels.slack_enabled {
            channels.push("slack".to_string());
        }
        if self.settings.channels.nostr_enabled {
            channels.push("nostr".to_string());
        }
        if self.settings.channels.gmail_enabled {
            channels.push("gmail".to_string());
        }
        #[cfg(target_os = "macos")]
        if self.settings.channels.imessage_enabled {
            channels.push("imessage".to_string());
        }
        #[cfg(target_os = "macos")]
        if self.settings.channels.apple_mail_enabled {
            channels.push("apple_mail".to_string());
        }
        if self.settings.channels.bluebubbles_enabled {
            channels.push("bluebubbles".to_string());
        }
        channels.extend(self.settings.channels.wasm_channels.iter().cloned());
        channels
    }

    pub(super) fn step_channel_continuity(&self) -> Result<(), SetupError> {
        print_info(
            "ThinClaw keeps one canonical direct-message session per linked principal across channels and devices.",
        );
        print_info(
            "That means the same person can continue a direct conversation from another channel without losing context.",
        );
        print_info(
            "Group threads stay isolated so public or shared spaces do not bleed into direct sessions.",
        );
        Ok(())
    }

    pub(super) async fn step_channel_verification(&mut self) -> Result<usize, SetupError> {
        let mut issues = 0usize;
        self.verified_channels.clear();

        self.remove_followup("channel-verification");

        if self.configured_channel_names().is_empty() {
            if self.is_quick_setup() && self.quick_primary_channel.as_deref() == Some("web") {
                self.verified_channels.insert("web".to_string(), true);
                print_success("Web Dashboard selected as the verified quick-setup path.");
                return Ok(0);
            }

            self.add_followup(FollowupDraft {
                id: "channel-verification".to_string(),
                title: "No external channels verified yet".to_string(),
                category: OnboardingFollowupCategory::Verification,
                status: OnboardingFollowupStatus::Optional,
                instructions: "ThinClaw can still run locally, but no external messaging path is configured yet.".to_string(),
                action_hint: Some("Rerun `thinclaw onboard --channels-only` when you are ready to add a channel.".to_string()),
            });
            print_warning(
                "No external messaging channel is configured yet. ThinClaw will still work locally.",
            );
            return Ok(1);
        }

        let secrets = self.init_secrets_context().await.ok();

        if self.settings.channels.http_enabled {
            let host = self
                .settings
                .channels
                .http_host
                .as_deref()
                .unwrap_or("0.0.0.0");
            let port = self.settings.channels.http_port.unwrap_or(8080);
            let ready = !host.trim().is_empty() && port > 0;
            self.verified_channels.insert("http".to_string(), ready);
            if ready {
                print_success("HTTP channel configuration looks valid.");
            } else {
                issues += 1;
                print_warning("HTTP channel is enabled but host/port configuration is invalid.");
            }
        }

        if self.settings.channels.signal_enabled {
            let signal_ready = if let (Some(url), Some(account)) = (
                self.settings.channels.signal_http_url.as_deref(),
                self.settings.channels.signal_account.as_deref(),
            ) {
                if url.trim().is_empty() || account.trim().is_empty() {
                    false
                } else {
                    Self::verify_http_reachable(url).await
                }
            } else {
                false
            };
            self.verified_channels
                .insert("signal".to_string(), signal_ready);
            if signal_ready {
                print_success("Signal verification passed (configuration + reachability).");
            } else {
                issues += 1;
                print_warning(
                    "Signal verification failed. Check account configuration and signal-http-api reachability.",
                );
            }
        }

        if self.settings.channels.discord_enabled {
            let mut discord_token = self
                .settings
                .channels
                .discord_bot_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("DISCORD_BOT_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if discord_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("discord_bot_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    discord_token = Some(token);
                }
            }

            let discord_ready = if let Some(token) = discord_token {
                Self::verify_discord_auth(&token).await
            } else {
                false
            };
            self.verified_channels
                .insert("discord".to_string(), discord_ready);
            if discord_ready {
                print_success("Discord verification passed (bot token accepted).");
            } else {
                issues += 1;
                print_warning(
                    "Discord verification failed. Ensure the bot token is valid and reachable.",
                );
            }
        }

        if self.settings.channels.slack_enabled {
            let mut bot_token = self
                .settings
                .channels
                .slack_bot_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("SLACK_BOT_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if bot_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("slack_bot_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    bot_token = Some(token);
                }
            }

            let mut app_token = self
                .settings
                .channels
                .slack_app_token
                .clone()
                .filter(|token| !token.trim().is_empty())
                .or_else(|| {
                    std::env::var("SLACK_APP_TOKEN")
                        .ok()
                        .filter(|token| !token.trim().is_empty())
                });
            if app_token.is_none()
                && let Some(ref ctx) = secrets
                && let Ok(secret) = ctx.get_secret("slack_app_token").await
            {
                let token = secret.expose_secret().trim().to_string();
                if !token.is_empty() {
                    app_token = Some(token);
                }
            }

            let slack_ready = if let (Some(bot), Some(app)) = (bot_token, app_token) {
                Self::verify_slack_bot_auth(&bot).await && Self::verify_slack_app_auth(&app).await
            } else {
                false
            };
            self.verified_channels
                .insert("slack".to_string(), slack_ready);
            if slack_ready {
                print_success("Slack verification passed (bot + app tokens accepted).");
            } else {
                issues += 1;
                print_warning(
                    "Slack verification failed. Check bot/app tokens and workspace connectivity.",
                );
            }
        }

        if self.settings.channels.nostr_enabled {
            let relays = self
                .settings
                .channels
                .nostr_relays
                .clone()
                .unwrap_or_default();
            let nostr_ready = Self::verify_nostr_relays(&relays).await;
            self.verified_channels
                .insert("nostr".to_string(), nostr_ready);
            if nostr_ready {
                print_success("Nostr relay verification passed.");
            } else {
                issues += 1;
                print_warning(
                    "Nostr verification failed. Ensure at least one relay URL is valid and reachable.",
                );
            }
        }

        if self.settings.channels.gmail_enabled {
            let gmail_ready = self
                .settings
                .channels
                .gmail_project_id
                .as_ref()
                .is_some_and(|v| !v.trim().is_empty())
                && self
                    .settings
                    .channels
                    .gmail_subscription_id
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty())
                && self
                    .settings
                    .channels
                    .gmail_topic_id
                    .as_ref()
                    .is_some_and(|v| !v.trim().is_empty());
            self.verified_channels
                .insert("gmail".to_string(), gmail_ready);
            if gmail_ready {
                print_success("Gmail verification passed (required Pub/Sub fields present).");
            } else {
                issues += 1;
                print_warning(
                    "Gmail verification failed. Project, subscription, and topic are required.",
                );
            }
        }

        if self.settings.channels.bluebubbles_enabled {
            let bb_ready = if let Some(ref url) = self.settings.channels.bluebubbles_server_url {
                if url.trim().is_empty() {
                    false
                } else {
                    let ping_url = format!("{}/api/v1/ping", url.trim_end_matches('/'));
                    Self::verify_http_reachable(&ping_url).await
                }
            } else {
                false
            };
            self.verified_channels
                .insert("bluebubbles".to_string(), bb_ready);
            if bb_ready {
                print_success("BlueBubbles verification passed (server reachable).");
            } else {
                issues += 1;
                print_warning(
                    "BlueBubbles verification failed. Ensure the server URL is correct and reachable.",
                );
            }
        }

        if self.settings.channels.wasm_channels.len() > 64 {
            issues += 1;
            print_warning("Only the first 64 configured WASM channels can be verified.");
        }
        for wasm_channel in self.settings.channels.wasm_channels.iter().take(64) {
            if wasm_channel.is_empty()
                || wasm_channel.len() > 128
                || !wasm_channel
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                issues += 1;
                print_warning("Ignored an invalid WASM channel identifier.");
                continue;
            }
            let channel_dir = dirs::home_dir().map(|home| home.join(".thinclaw/channels"));
            let wasm_path = channel_dir
                .as_ref()
                .map(|dir| dir.join(format!("{wasm_channel}.wasm")));
            let caps_path = channel_dir
                .as_ref()
                .map(|dir| dir.join(format!("{wasm_channel}.capabilities.json")));
            let artifact_ready = wasm_path.as_ref().is_some_and(|path| {
                std::fs::symlink_metadata(path)
                    .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            });
            let mut missing_required_secrets = Vec::new();
            if let Some(caps_path) = caps_path.as_ref()
                && let Ok(bytes) = thinclaw_platform::read_regular_file_bounded_single_link_async(
                    caps_path.clone(),
                    1024 * 1024,
                )
                .await
                && let Ok(cap_file) =
                    crate::channels::wasm::ChannelCapabilitiesFile::from_bytes(&bytes)
            {
                for secret in &cap_file.setup.required_secrets {
                    if secret.optional {
                        continue;
                    }
                    let present = if let Some(secrets_ctx) = secrets.as_ref() {
                        secrets_ctx
                            .get_secret(&secret.name)
                            .await
                            .map(|value| !value.expose_secret().trim().is_empty())
                            .unwrap_or(false)
                    } else {
                        false
                    };
                    if !present {
                        missing_required_secrets.push(secret.name.clone());
                    }
                }
            }
            let ready = artifact_ready && missing_required_secrets.is_empty();
            self.verified_channels.insert(wasm_channel.clone(), ready);
            if ready {
                print_success(&format!(
                    "WASM channel '{}' is installed and setup-ready.",
                    wasm_channel
                ));
            } else {
                issues += 1;
                if artifact_ready {
                    print_warning(&format!(
                        "WASM channel '{}' is enabled but required setup secrets are missing: {}.",
                        wasm_channel,
                        missing_required_secrets.join(", ")
                    ));
                } else {
                    print_warning(&format!(
                        "WASM channel '{}' is enabled but the wasm artifact is missing.",
                        wasm_channel
                    ));
                }
            }
        }

        if issues > 0 {
            self.add_followup(FollowupDraft {
                id: "channel-verification".to_string(),
                title: "Review incomplete channel configuration".to_string(),
                category: OnboardingFollowupCategory::Verification,
                status: OnboardingFollowupStatus::NeedsAttention,
                instructions: "At least one enabled channel is still missing required configuration details or verification signals.".to_string(),
                action_hint: Some("Use `thinclaw onboard --channels-only` to revisit the channel flow.".to_string()),
            });
            print_warning(&format!(
                "Channel verification found {} configuration gap(s). The completion screen will keep a follow-up for you.",
                issues
            ));
        } else {
            print_success(
                "Channel verification found at least one ready messaging path and no obvious configuration gaps.",
            );
        }

        Ok(issues)
    }

    async fn verify_http_reachable(url: &str) -> bool {
        if url.is_empty() || url.len() > MAX_CHANNEL_VERIFICATION_URL_BYTES {
            return false;
        }
        let parsed = match url::Url::parse(url) {
            Ok(parsed) => parsed,
            Err(_) => return false,
        };
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return false;
        }
        let Some(host) = parsed.host_str() else {
            return false;
        };
        let Some(port) = parsed.port_or_known_default() else {
            return false;
        };
        let addresses = match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            tokio::net::lookup_host((host, port)),
        )
        .await
        {
            Ok(Ok(addresses)) => addresses,
            _ => return false,
        };
        let mut addresses = addresses.collect::<Vec<_>>();
        addresses.sort_unstable();
        addresses.dedup();
        if addresses.is_empty()
            || addresses.len() > MAX_CHANNEL_DNS_ADDRESSES
            || addresses.iter().any(|address| {
                let ip = address.ip();
                ip.is_unspecified()
                    || ip.is_multicast()
                    || matches!(ip, std::net::IpAddr::V4(ip) if ip.is_broadcast() || ip.is_link_local())
                    || matches!(ip, std::net::IpAddr::V6(ip) if ip.is_unicast_link_local())
                    || parsed.scheme() == "http"
                        && thinclaw_tools_core::is_public_outbound_ip(ip)
            })
        {
            return false;
        }
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .connect_timeout(std::time::Duration::from_secs(3))
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .build()
        {
            Ok(client) => client,
            Err(_) => return false,
        };
        client.get(parsed).send().await.is_ok()
    }

    async fn verify_discord_auth(bot_token: &str) -> bool {
        if !valid_channel_auth_token(bot_token) {
            return false;
        }
        let Some((client, endpoint)) = fixed_public_client(
            "https://discord.com/api/v10/users/@me",
            "discord.com",
            std::time::Duration::from_secs(8),
        )
        .await
        else {
            return false;
        };
        client
            .get(endpoint)
            .header("Authorization", format!("Bot {}", bot_token.trim()))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    async fn verify_slack_bot_auth(bot_token: &str) -> bool {
        if !valid_channel_auth_token(bot_token) {
            return false;
        }
        let Some((client, endpoint)) = fixed_public_client(
            "https://slack.com/api/auth.test",
            "slack.com",
            std::time::Duration::from_secs(8),
        )
        .await
        else {
            return false;
        };
        let response = client
            .post(endpoint)
            .bearer_auth(bot_token.trim())
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = crate::http_response::bounded_json::<serde_json::Value>(
            response,
            MAX_CHANNEL_VERIFICATION_RESPONSE_BYTES,
        )
        .await
        else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_slack_app_auth(app_token: &str) -> bool {
        if !valid_channel_auth_token(app_token) {
            return false;
        }
        let Some((client, endpoint)) = fixed_public_client(
            "https://slack.com/api/apps.connections.open",
            "slack.com",
            std::time::Duration::from_secs(8),
        )
        .await
        else {
            return false;
        };
        let response = client
            .post(endpoint)
            .bearer_auth(app_token.trim())
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = crate::http_response::bounded_json::<serde_json::Value>(
            response,
            MAX_CHANNEL_VERIFICATION_RESPONSE_BYTES,
        )
        .await
        else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_nostr_relays(relays_csv: &str) -> bool {
        if relays_csv.is_empty() || relays_csv.len() > MAX_NOSTR_RELAYS_CSV_BYTES {
            return false;
        }
        let relays = relays_csv
            .split(',')
            .map(str::trim)
            .filter(|relay| !relay.is_empty())
            .collect::<Vec<_>>();
        if relays.is_empty() || relays.len() > MAX_NOSTR_VERIFICATION_RELAYS {
            return false;
        }

        tokio::time::timeout(std::time::Duration::from_secs(8), async {
            for relay in relays {
                if verify_one_nostr_relay(relay).await {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    }
}

async fn verify_one_nostr_relay(relay: &str) -> bool {
    if relay.is_empty()
        || relay.len() > MAX_NOSTR_RELAY_URL_BYTES
        || relay.chars().any(char::is_control)
    {
        return false;
    }
    let Ok(url) = url::Url::parse(relay) else {
        return false;
    };
    if !matches!(url.scheme(), "ws" | "wss")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let Some(port) = url.port_or_known_default() else {
        return false;
    };
    let Ok(Ok(addresses)) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::lookup_host((host, port)),
    )
    .await
    else {
        return false;
    };
    let mut addresses = addresses.collect::<Vec<_>>();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() || addresses.len() > MAX_CHANNEL_DNS_ADDRESSES {
        return false;
    }

    let local_wss = url.scheme() == "wss" && nostr_host_is_explicitly_local(host);
    let valid_boundary = if url.scheme() == "ws" || local_wss {
        addresses
            .iter()
            .all(|address| is_safe_private_nostr_ip(address.ip()))
    } else {
        addresses
            .iter()
            .all(|address| thinclaw_tools_core::is_public_outbound_ip(address.ip()))
    };
    if !valid_boundary {
        return false;
    }

    for address in addresses {
        if matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                tokio::net::TcpStream::connect(address),
            )
            .await,
            Ok(Ok(_))
        ) {
            return true;
        }
    }
    false
}

fn nostr_host_is_explicitly_local(host: &str) -> bool {
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return is_safe_private_nostr_ip(ip);
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || !host.contains('.')
}

fn is_safe_private_nostr_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        std::net::IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
    }
}

fn valid_channel_auth_token(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= MAX_CHANNEL_AUTH_TOKEN_BYTES
        && !value.chars().any(char::is_control)
}

async fn fixed_public_client(
    endpoint: &str,
    allowed_host: &str,
    timeout: std::time::Duration,
) -> Option<(reqwest::Client, reqwest::Url)> {
    let guarded = validate_outbound_url_pinned_async(
        endpoint,
        &OutboundUrlGuardOptions {
            require_https: true,
            upgrade_http_to_https: false,
            allowlist: vec![allowed_host.to_string()],
        },
    )
    .await
    .ok()?;
    let host = guarded.url.host_str()?.to_string();
    let mut builder = reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout.min(std::time::Duration::from_secs(5)))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy();
    if !guarded.pinned_addrs.is_empty() {
        builder = builder.resolve_to_addrs(&host, &guarded.pinned_addrs);
    }
    Some((builder.build().ok()?, guarded.url))
}
