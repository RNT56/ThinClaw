//! Channel continuity messaging and non-destructive channel verification
//! (configuration + reachability checks per enabled channel).

use secrecy::ExposeSecret;

use crate::settings::{OnboardingFollowupCategory, OnboardingFollowupStatus};
use crate::setup::prompts::{print_info, print_success, print_warning};

use super::{FollowupDraft, SetupError, SetupWizard};

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

        for wasm_channel in &self.settings.channels.wasm_channels {
            let channel_dir = dirs::home_dir().map(|home| home.join(".thinclaw/channels"));
            let wasm_path = channel_dir
                .as_ref()
                .map(|dir| dir.join(format!("{wasm_channel}.wasm")));
            let caps_path = channel_dir
                .as_ref()
                .map(|dir| dir.join(format!("{wasm_channel}.capabilities.json")));
            let artifact_ready = wasm_path.as_ref().is_some_and(|path| path.exists());
            let mut missing_required_secrets = Vec::new();
            if let Some(caps_path) = caps_path.as_ref()
                && let Ok(bytes) = tokio::fs::read(caps_path).await
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
        let parsed = match url::Url::parse(url) {
            Ok(parsed) => parsed,
            Err(_) => return false,
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            return false;
        }
        reqwest::Client::new()
            .get(parsed)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .is_ok()
    }

    async fn verify_discord_auth(bot_token: &str) -> bool {
        reqwest::Client::new()
            .get("https://discord.com/api/v10/users/@me")
            .header("Authorization", format!("Bot {}", bot_token.trim()))
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await
            .map(|response| response.status().is_success())
            .unwrap_or(false)
    }

    async fn verify_slack_bot_auth(bot_token: &str) -> bool {
        let response = reqwest::Client::new()
            .post("https://slack.com/api/auth.test")
            .bearer_auth(bot_token.trim())
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = response.json::<serde_json::Value>().await else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_slack_app_auth(app_token: &str) -> bool {
        let response = reqwest::Client::new()
            .post("https://slack.com/api/apps.connections.open")
            .bearer_auth(app_token.trim())
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await;

        let Ok(response) = response else {
            return false;
        };
        let Ok(payload) = response.json::<serde_json::Value>().await else {
            return false;
        };
        payload
            .get("ok")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    async fn verify_nostr_relays(relays_csv: &str) -> bool {
        for relay in relays_csv
            .split(',')
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let Ok(url) = url::Url::parse(relay) else {
                continue;
            };
            if !matches!(url.scheme(), "ws" | "wss") {
                continue;
            }
            let Some(host) = url.host_str() else {
                continue;
            };
            let port = url
                .port_or_known_default()
                .unwrap_or(if url.scheme() == "wss" { 443 } else { 80 });
            let reachable = tokio::time::timeout(
                std::time::Duration::from_secs(4),
                tokio::net::lookup_host((host, port)),
            )
            .await
            .ok()
            .and_then(|lookup| lookup.ok())
            .is_some_and(|mut addrs| addrs.next().is_some());
            if reachable {
                return true;
            }
        }
        false
    }
}
