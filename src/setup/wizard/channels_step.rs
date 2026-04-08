//! Channel configuration wizard step.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

use crate::channels::wasm::ChannelCapabilitiesFile;
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::setup::channels::{
    SecretsContext, setup_http, setup_signal, setup_telegram, setup_tunnel, setup_wasm_channel,
};
use crate::setup::prompts::{
    confirm, input, optional_input, print_info, print_success, secret_input, select_many,
};

use super::helpers::{
    build_channel_options, capitalize_first, discover_wasm_channels,
    install_selected_bundled_channels, install_selected_registry_channels, mask_api_key,
};
use super::{SetupError, SetupWizard};

#[allow(unused_imports)]
use super::{
    CHANNEL_INDEX_DISCORD, CHANNEL_INDEX_GMAIL, CHANNEL_INDEX_HTTP, CHANNEL_INDEX_NOSTR,
    CHANNEL_INDEX_SIGNAL, CHANNEL_INDEX_SLACK,
};

#[cfg(target_os = "macos")]
#[allow(unused_imports)]
use super::{CHANNEL_INDEX_APPLE_MAIL, CHANNEL_INDEX_IMESSAGE};

impl SetupWizard {
    pub(super) async fn init_secrets_context(&mut self) -> Result<SecretsContext, SetupError> {
        // Get crypto (should be set from step 2, or load from keychain/env)
        let crypto = if let Some(ref c) = self.secrets_crypto {
            Arc::clone(c)
        } else {
            // Try to load master key from keychain or env
            let key = if let Ok(env_key) = std::env::var("SECRETS_MASTER_KEY") {
                env_key
            } else if let Ok(keychain_key) = crate::secrets::keychain::get_master_key().await {
                keychain_key.iter().map(|b| format!("{:02x}", b)).collect()
            } else {
                return Err(SetupError::Config(
                    "Secrets not configured. Run full setup or set SECRETS_MASTER_KEY.".to_string(),
                ));
            };

            let crypto = Arc::new(
                SecretsCrypto::new(SecretString::from(key))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            );
            self.secrets_crypto = Some(Arc::clone(&crypto));
            crypto
        };

        // Create backend-appropriate secrets store.
        // Respect the user's selected backend when both features are compiled,
        // so we don't accidentally use a postgres pool from DATABASE_URL when
        // libsql was chosen (or vice versa).
        let selected_backend = self
            .settings
            .database_backend
            .as_deref()
            .unwrap_or("postgres");

        #[cfg(all(feature = "libsql", feature = "postgres"))]
        {
            if selected_backend == "libsql" {
                if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
                if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
            } else {
                if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
                if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                    return Ok(SecretsContext::from_store(store, "default"));
                }
            }
        }

        #[cfg(all(feature = "postgres", not(feature = "libsql")))]
        {
            let _ = selected_backend;
            if let Some(store) = self.create_postgres_secrets_store(&crypto).await? {
                return Ok(SecretsContext::from_store(store, "default"));
            }
        }

        #[cfg(all(feature = "libsql", not(feature = "postgres")))]
        {
            let _ = selected_backend;
            if let Some(store) = self.create_libsql_secrets_store(&crypto)? {
                return Ok(SecretsContext::from_store(store, "default"));
            }
        }

        Err(SetupError::Config(
            "No database backend available for secrets storage".to_string(),
        ))
    }

    /// Create a PostgreSQL secrets store from the current pool.
    #[cfg(feature = "postgres")]
    pub(super) async fn create_postgres_secrets_store(
        &mut self,
        crypto: &Arc<SecretsCrypto>,
    ) -> Result<Option<Arc<dyn SecretsStore>>, SetupError> {
        let pool = if let Some(ref p) = self.db_pool {
            p.clone()
        } else {
            // Fall back to creating one from settings/env
            let url = self
                .settings
                .database_url
                .clone()
                .or_else(|| std::env::var("DATABASE_URL").ok());

            if let Some(url) = url {
                self.test_database_connection_postgres(&url).await?;
                self.run_migrations_postgres().await?;
                match self.db_pool.clone() {
                    Some(pool) => pool,
                    None => {
                        return Err(SetupError::Database(
                            "Database pool not initialized after connection test".to_string(),
                        ));
                    }
                }
            } else {
                return Ok(None);
            }
        };

        let store: Arc<dyn SecretsStore> = Arc::new(crate::secrets::PostgresSecretsStore::new(
            pool,
            Arc::clone(crypto),
        ));
        Ok(Some(store))
    }

    /// Create a libSQL secrets store from the current backend.
    #[cfg(feature = "libsql")]
    pub(super) fn create_libsql_secrets_store(
        &self,
        crypto: &Arc<SecretsCrypto>,
    ) -> Result<Option<Arc<dyn SecretsStore>>, SetupError> {
        if let Some(ref backend) = self.db_backend {
            let store: Arc<dyn SecretsStore> = Arc::new(crate::secrets::LibSqlSecretsStore::new(
                backend.shared_db(),
                Arc::clone(crypto),
            ));
            Ok(Some(store))
        } else {
            Ok(None)
        }
    }

    /// Step 8: Channel configuration.
    pub(super) async fn step_channels(&mut self) -> Result<(), SetupError> {
        let secrets = self.init_secrets_context().await.ok();

        // First, configure tunnel (shared across all channels that need webhooks)
        match setup_tunnel(&self.settings, secrets.as_ref()).await {
            Ok(tunnel_settings) => {
                self.settings.tunnel = tunnel_settings;
            }
            Err(e) => {
                print_info(&format!("Tunnel setup skipped: {}", e));
            }
        }
        println!();

        // Discover available WASM channels
        let channels_dir = dirs::home_dir()
            .ok_or_else(|| SetupError::Config("Could not determine home directory".into()))?
            .join(".thinclaw/channels");

        let mut discovered_channels = discover_wasm_channels(&channels_dir).await;
        let installed_names: HashSet<String> = discovered_channels
            .iter()
            .map(|(name, _)| name.clone())
            .collect();

        // Build channel list from registry (if available) + bundled + discovered
        let wasm_channel_names = build_channel_options(&discovered_channels);

        // Build options list dynamically: native channels first, then WASM
        let mut options: Vec<(String, bool)> = vec![
            ("CLI/TUI (always enabled)".to_string(), true),
            (
                "HTTP webhook".to_string(),
                self.settings.channels.http_enabled,
            ),
            ("Signal".to_string(), self.settings.channels.signal_enabled),
            (
                "Discord".to_string(),
                self.settings.channels.discord_enabled,
            ),
            ("Slack".to_string(), self.settings.channels.slack_enabled),
            ("Nostr".to_string(), self.settings.channels.nostr_enabled),
            ("Gmail".to_string(), self.settings.channels.gmail_enabled),
        ];

        #[cfg(target_os = "macos")]
        options.push((
            "iMessage".to_string(),
            self.settings.channels.imessage_enabled,
        ));

        #[cfg(target_os = "macos")]
        options.push((
            "Apple Mail".to_string(),
            self.settings.channels.apple_mail_enabled,
        ));

        let native_count = options.len();

        // Add available WASM channels (installed + bundled + registry)
        for name in &wasm_channel_names {
            let is_enabled = self.settings.channels.wasm_channels.contains(name);
            let label = if installed_names.contains(name) {
                format!("{} (installed)", capitalize_first(name))
            } else {
                format!("{} (will install)", capitalize_first(name))
            };
            options.push((label, is_enabled));
        }

        let options_refs: Vec<(&str, bool)> =
            options.iter().map(|(s, b)| (s.as_str(), *b)).collect();

        let selected = select_many("Which channels do you want to enable?", &options_refs)
            .map_err(SetupError::Io)?;

        let selected_wasm_channels: Vec<String> = wasm_channel_names
            .iter()
            .enumerate()
            .filter_map(|(idx, name)| {
                if selected.contains(&(native_count + idx)) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();

        // Install selected channels that aren't already on disk
        let mut any_installed = false;

        // Try bundled channels first (pre-compiled artifacts from channels-src/)
        let bundled_result = install_selected_bundled_channels(
            &channels_dir,
            &selected_wasm_channels,
            &installed_names,
        )
        .await?;

        let bundled_installed: HashSet<String> = bundled_result
            .as_ref()
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default();

        if !bundled_installed.is_empty() {
            print_success(&format!(
                "Installed bundled channels: {}",
                bundled_result.as_ref().unwrap().join(", ")
            ));
            any_installed = true;
        }

        let installed_from_registry = install_selected_registry_channels(
            &channels_dir,
            &selected_wasm_channels,
            &installed_names,
            &bundled_installed,
        )
        .await;

        if !installed_from_registry.is_empty() {
            print_success(&format!(
                "Built from registry: {}",
                installed_from_registry.join(", ")
            ));
            any_installed = true;
        }

        // Re-discover after installs
        if any_installed {
            discovered_channels = discover_wasm_channels(&channels_dir).await;
        }

        let needs_secrets = selected.contains(&CHANNEL_INDEX_HTTP)
            || selected.contains(&CHANNEL_INDEX_DISCORD)
            || selected.contains(&CHANNEL_INDEX_SLACK)
            || !selected_wasm_channels.is_empty();
        if needs_secrets && secrets.is_none() {
            print_info(
                "Secrets not available. Channel tokens must be set via environment variables.",
            );
        }

        // HTTP channel
        if selected.contains(&CHANNEL_INDEX_HTTP) {
            println!();
            if let Some(ref ctx) = secrets {
                let result = setup_http(ctx).await?;
                self.settings.channels.http_enabled = result.enabled;
                self.settings.channels.http_port = Some(result.port);
                self.settings.channels.http_host = Some(result.host);
            } else {
                self.settings.channels.http_enabled = true;
                self.settings.channels.http_port = Some(8080);
                self.settings.channels.http_host = Some("0.0.0.0".to_string());
                print_info("HTTP webhook enabled on port 8080 (set HTTP_WEBHOOK_SECRET in env)");
            }
        } else {
            self.settings.channels.http_enabled = false;
        }

        // Signal channel
        if selected.contains(&CHANNEL_INDEX_SIGNAL) {
            println!();
            let result = setup_signal(&self.settings).await?;
            self.settings.channels.signal_enabled = result.enabled;
            self.settings.channels.signal_http_url = Some(result.http_url);
            self.settings.channels.signal_account = Some(result.account);
            self.settings.channels.signal_allow_from = Some(result.allow_from);
            self.settings.channels.signal_allow_from_groups = Some(result.allow_from_groups);
            self.settings.channels.signal_dm_policy = Some(result.dm_policy);
            self.settings.channels.signal_group_policy = Some(result.group_policy);
            self.settings.channels.signal_group_allow_from = Some(result.group_allow_from);
        } else {
            self.settings.channels.signal_enabled = false;
            self.settings.channels.signal_http_url = None;
            self.settings.channels.signal_account = None;
            self.settings.channels.signal_allow_from = None;
            self.settings.channels.signal_allow_from_groups = None;
            self.settings.channels.signal_dm_policy = None;
            self.settings.channels.signal_group_policy = None;
            self.settings.channels.signal_group_allow_from = None;
        }

        // Discord channel
        if selected.contains(&CHANNEL_INDEX_DISCORD) {
            println!();
            print_info(
                "Discord requires a Bot Token from https://discord.com/developers/applications",
            );
            println!();

            let token = if let Some(existing) = std::env::var("DISCORD_BOT_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
            {
                let masked = mask_api_key(&existing);
                if confirm(
                    &format!("Use existing DISCORD_BOT_TOKEN ({})?", masked),
                    true,
                )
                .map_err(SetupError::Io)?
                {
                    existing
                } else {
                    secret_input("Discord bot token")
                        .map_err(SetupError::Io)?
                        .expose_secret()
                        .to_string()
                }
            } else {
                secret_input("Discord bot token")
                    .map_err(SetupError::Io)?
                    .expose_secret()
                    .to_string()
            };

            // Store via secrets if available
            if let Some(ref ctx) = secrets {
                if let Err(e) = ctx
                    .save_secret(
                        "discord_bot_token",
                        &secrecy::SecretString::from(token.clone()),
                    )
                    .await
                {
                    print_info(&format!("Could not store token in secrets: {}", e));
                }
            }
            self.settings.channels.discord_bot_token =
                if secrets.is_some() { None } else { Some(token) };

            let guild_id =
                optional_input("Guild ID (restrict to single server, blank = all)", None)
                    .map_err(SetupError::Io)?;
            if let Some(ref gid) = guild_id {
                if !gid.is_empty() {
                    self.settings.channels.discord_guild_id = Some(gid.clone());
                }
            }

            let allow_from =
                optional_input("Allowed channel IDs (comma-separated, blank = all)", None)
                    .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from {
                if !af.is_empty() {
                    self.settings.channels.discord_allow_from = Some(af.clone());
                }
            }

            self.settings.channels.discord_enabled = true;
            print_success("Discord channel configured");
        } else {
            self.settings.channels.discord_enabled = false;
            self.settings.channels.discord_bot_token = None;
        }

        // Slack channel
        if selected.contains(&CHANNEL_INDEX_SLACK) {
            println!();
            print_info(
                "Slack requires both a Bot Token (xoxb-...) and an App-Level Token (xapp-...)",
            );
            print_info("Create these at https://api.slack.com/apps");
            println!();

            let bot_token = if let Some(existing) = std::env::var("SLACK_BOT_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
            {
                let masked = mask_api_key(&existing);
                if confirm(&format!("Use existing SLACK_BOT_TOKEN ({})?", masked), true)
                    .map_err(SetupError::Io)?
                {
                    existing
                } else {
                    secret_input("Slack bot token (xoxb-...)")
                        .map_err(SetupError::Io)?
                        .expose_secret()
                        .to_string()
                }
            } else {
                secret_input("Slack bot token (xoxb-...)")
                    .map_err(SetupError::Io)?
                    .expose_secret()
                    .to_string()
            };

            let app_token = if let Some(existing) = std::env::var("SLACK_APP_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
            {
                let masked = mask_api_key(&existing);
                if confirm(&format!("Use existing SLACK_APP_TOKEN ({})?", masked), true)
                    .map_err(SetupError::Io)?
                {
                    existing
                } else {
                    secret_input("Slack app-level token (xapp-...)")
                        .map_err(SetupError::Io)?
                        .expose_secret()
                        .to_string()
                }
            } else {
                secret_input("Slack app-level token (xapp-...)")
                    .map_err(SetupError::Io)?
                    .expose_secret()
                    .to_string()
            };

            // Store via secrets if available
            if let Some(ref ctx) = secrets {
                if let Err(e) = ctx
                    .save_secret(
                        "slack_bot_token",
                        &secrecy::SecretString::from(bot_token.clone()),
                    )
                    .await
                {
                    print_info(&format!("Could not store bot token in secrets: {}", e));
                }
                if let Err(e) = ctx
                    .save_secret(
                        "slack_app_token",
                        &secrecy::SecretString::from(app_token.clone()),
                    )
                    .await
                {
                    print_info(&format!("Could not store app token in secrets: {}", e));
                }
            }
            if secrets.is_some() {
                self.settings.channels.slack_bot_token = None;
                self.settings.channels.slack_app_token = None;
            } else {
                self.settings.channels.slack_bot_token = Some(bot_token);
                self.settings.channels.slack_app_token = Some(app_token);
            }

            let allow_from = optional_input(
                "Allowed Slack channel/DM IDs (comma-separated, blank = all)",
                None,
            )
            .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from {
                if !af.is_empty() {
                    self.settings.channels.slack_allow_from = Some(af.clone());
                }
            }

            self.settings.channels.slack_enabled = true;
            print_success("Slack channel configured");
        } else {
            self.settings.channels.slack_enabled = false;
            self.settings.channels.slack_bot_token = None;
            self.settings.channels.slack_app_token = None;
        }

        // Nostr channel
        if selected.contains(&CHANNEL_INDEX_NOSTR) {
            println!();
            print_info("Nostr connects to relay servers to receive and send messages.");
            println!();

            let default_relays = "wss://relay.damus.io,wss://nos.lol";
            let relays = optional_input("Relay URLs (comma-separated)", Some(default_relays))
                .map_err(SetupError::Io)?;
            let relay_str = relays
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default_relays.to_string());
            self.settings.channels.nostr_relays = Some(relay_str);

            let allow_from = optional_input(
                "Allowed public keys (comma-separated hex/npub, '*' = all, blank = all)",
                None,
            )
            .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from {
                if !af.is_empty() {
                    self.settings.channels.nostr_allow_from = Some(af.clone());
                }
            }

            self.settings.channels.nostr_enabled = true;
            print_success("Nostr channel configured");
            print_info(
                "Set NOSTR_SECRET_KEY env var with your nsec/hex private key before starting.",
            );
        } else {
            self.settings.channels.nostr_enabled = false;
        }

        // Gmail channel
        if selected.contains(&CHANNEL_INDEX_GMAIL) {
            println!();
            print_info("Gmail requires GCP project with Pub/Sub and Gmail API enabled.");
            print_info("Follow: https://developers.google.com/gmail/api/guides/push");
            println!();

            let project_id = input("GCP project ID").map_err(SetupError::Io)?;
            self.settings.channels.gmail_project_id = Some(project_id);

            let sub_id = input("Pub/Sub subscription ID").map_err(SetupError::Io)?;
            self.settings.channels.gmail_subscription_id = Some(sub_id);

            let topic_id = input("Pub/Sub topic ID").map_err(SetupError::Io)?;
            self.settings.channels.gmail_topic_id = Some(topic_id);

            let allowed_senders =
                optional_input("Allowed sender emails (comma-separated, blank = all)", None)
                    .map_err(SetupError::Io)?;
            if let Some(ref senders) = allowed_senders {
                if !senders.is_empty() {
                    self.settings.channels.gmail_allowed_senders = Some(senders.clone());
                }
            }

            self.settings.channels.gmail_enabled = true;
            print_success("Gmail channel configured");
            print_info(
                "Run `thinclaw auth gmail` to complete OAuth2 authentication before starting.",
            );
        } else {
            self.settings.channels.gmail_enabled = false;
        }

        // iMessage channel (macOS only)
        #[cfg(target_os = "macos")]
        if selected.contains(&CHANNEL_INDEX_IMESSAGE) {
            println!();
            print_info("iMessage uses the native macOS Messages database.");
            print_info("ThinClaw will need Full Disk Access in System Settings > Privacy.");
            println!();

            let allow_from = optional_input(
                "Allowed contacts (comma-separated phone/email, blank = all)",
                None,
            )
            .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from {
                if !af.is_empty() {
                    self.settings.channels.imessage_allow_from = Some(af.clone());
                }
            }

            let poll_interval =
                optional_input("Polling interval in seconds", Some("5")).map_err(SetupError::Io)?;
            if let Some(ref pi) = poll_interval {
                if let Ok(n) = pi.parse::<u64>() {
                    self.settings.channels.imessage_poll_interval = Some(n);
                }
            }

            self.settings.channels.imessage_enabled = true;
            print_success("iMessage channel configured");
        }
        #[cfg(target_os = "macos")]
        if !selected.contains(&CHANNEL_INDEX_IMESSAGE) {
            self.settings.channels.imessage_enabled = false;
        }

        // Apple Mail channel (macOS only)
        #[cfg(target_os = "macos")]
        if selected.contains(&CHANNEL_INDEX_APPLE_MAIL) {
            println!();
            print_info("Apple Mail uses the native macOS Mail.app Envelope Index database.");
            print_info("ThinClaw will need Full Disk Access in System Settings > Privacy.");
            print_info("Make sure Mail.app is configured and signed into your account.");
            print_info(
                "⚠️  IMPORTANT: If you leave this blank, ANY email sender can give instructions to your agent.",
            );
            print_info(
                "   For security, specify your email address(es) so only you can control it via email.",
            );
            println!();

            let allow_from = optional_input(
                "Your email address(es) to allow (comma-separated, ⚠️ blank = ANYONE can control agent)",
                None,
            )
            .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from {
                if !af.is_empty() {
                    self.settings.channels.apple_mail_allow_from = Some(af.clone());
                }
            }

            let poll_interval = optional_input("Polling interval in seconds", Some("10"))
                .map_err(SetupError::Io)?;
            if let Some(ref pi) = poll_interval {
                if let Ok(n) = pi.parse::<u64>() {
                    self.settings.channels.apple_mail_poll_interval = Some(n);
                }
            }

            let unread_only =
                confirm("Only process unread messages?", true).map_err(SetupError::Io)?;
            self.settings.channels.apple_mail_unread_only = unread_only;

            let mark_as_read =
                confirm("Mark messages as read after processing?", true).map_err(SetupError::Io)?;
            self.settings.channels.apple_mail_mark_as_read = mark_as_read;

            self.settings.channels.apple_mail_enabled = true;
            print_success("Apple Mail channel configured");
        }
        #[cfg(target_os = "macos")]
        if !selected.contains(&CHANNEL_INDEX_APPLE_MAIL) {
            self.settings.channels.apple_mail_enabled = false;
        }

        let discovered_by_name: HashMap<String, ChannelCapabilitiesFile> =
            discovered_channels.into_iter().collect();

        // Process selected WASM channels
        let mut enabled_wasm_channels = Vec::new();
        for channel_name in selected_wasm_channels {
            println!();
            if let Some(ref ctx) = secrets {
                let result = if let Some(cap_file) = discovered_by_name.get(&channel_name) {
                    if !cap_file.setup.required_secrets.is_empty() {
                        setup_wasm_channel(ctx, &channel_name, &cap_file.setup).await?
                    } else if channel_name == "telegram" {
                        let telegram_result = setup_telegram(ctx, &self.settings).await?;
                        if let Some(owner_id) = telegram_result.owner_id {
                            self.settings.channels.telegram_owner_id = Some(owner_id);
                        }
                        crate::setup::channels::WasmChannelSetupResult {
                            enabled: telegram_result.enabled,
                            channel_name: "telegram".to_string(),
                        }
                    } else {
                        print_info(&format!(
                            "No setup configuration found for {}",
                            channel_name
                        ));
                        crate::setup::channels::WasmChannelSetupResult {
                            enabled: true,
                            channel_name: channel_name.clone(),
                        }
                    }
                } else {
                    print_info(&format!(
                        "Channel '{}' is selected but not available on disk.",
                        channel_name
                    ));
                    continue;
                };

                if result.enabled {
                    enabled_wasm_channels.push(result.channel_name);
                }
            } else {
                // No secrets context, just enable the channel
                print_info(&format!(
                    "{} enabled (configure tokens via environment)",
                    capitalize_first(&channel_name)
                ));
                enabled_wasm_channels.push(channel_name.clone());
            }
        }

        self.settings.channels.wasm_channels = enabled_wasm_channels;

        Ok(())
    }
}
