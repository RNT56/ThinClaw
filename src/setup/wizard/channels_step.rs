//! Channel configuration wizard step.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(feature = "nostr")]
use nostr_sdk::ToBech32;
#[cfg(feature = "nostr")]
use nostr_sdk::prelude::Keys;
use secrecy::{ExposeSecret, SecretString};

use crate::channels::wasm::ChannelCapabilitiesFile;
use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::setup::channels::{
    SecretsContext, setup_http, setup_signal, setup_telegram, setup_tunnel, setup_wasm_channel,
};
use crate::setup::prompts::{
    confirm, input, optional_input, print_info, print_success, print_warning, secret_input,
    select_many, select_one,
};

use super::helpers::{
    build_channel_options, capitalize_first, discover_wasm_channels,
    install_selected_bundled_channels, install_selected_registry_channels, mask_api_key,
};
use super::{FollowupDraft, SetupError, SetupWizard};
use crate::settings::{OnboardingFollowupCategory, OnboardingFollowupStatus};

impl SetupWizard {
    fn reset_quick_channel_selection(&mut self) {
        self.settings.channels.http_enabled = false;
        self.settings.channels.signal_enabled = false;
        self.settings.channels.discord_enabled = false;
        self.settings.channels.slack_enabled = false;
        self.settings.channels.nostr_enabled = false;
        self.settings.channels.gmail_enabled = false;
        self.settings.channels.bluebubbles_enabled = false;
        #[cfg(target_os = "macos")]
        {
            self.settings.channels.imessage_enabled = false;
            self.settings.channels.apple_mail_enabled = false;
        }
        self.settings.channels.wasm_channels.clear();
        self.quick_primary_channel = None;
    }

    fn quick_channel_install_failure(&mut self, channel_name: &str, reason: &str) {
        print_warning(&format!(
            "{} could not be prepared during quick setup: {}",
            channel_name, reason
        ));
        print_info("Falling back to the Web Dashboard for now.");
        self.quick_primary_channel = Some("web".to_string());
        self.add_followup(FollowupDraft {
            id: format!("quick-channel-{}", channel_name.to_ascii_lowercase()),
            title: format!("Finish {} channel setup", channel_name),
            category: OnboardingFollowupCategory::Verification,
            status: OnboardingFollowupStatus::NeedsAttention,
            instructions: format!(
                "{} could not be fully prepared during quick setup. ThinClaw was left on the Web Dashboard so onboarding could continue.",
                channel_name
            ),
            action_hint: Some(
                "Open Guided Settings > Channels & Notifications to retry the channel setup."
                    .to_string(),
            ),
        });
    }

    #[cfg(feature = "nostr")]
    async fn configure_nostr_channel(
        &mut self,
        secrets: Option<&SecretsContext>,
        owner_required: bool,
    ) -> Result<bool, SetupError> {
        let default_relays = "wss://relay.damus.io,wss://nos.lol";
        let relays = optional_input("Relay URLs (comma-separated)", Some(default_relays))
            .map_err(SetupError::Io)?
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_relays.to_string());

        let private_key = secret_input("Nostr private key (nsec or hex)")
            .map_err(SetupError::Io)?
            .expose_secret()
            .trim()
            .to_string();
        if private_key.is_empty() {
            print_warning("Nostr private key is required.");
            return Ok(false);
        }

        let keys = match Keys::parse(&private_key) {
            Ok(keys) => keys,
            Err(err) => {
                print_warning(&format!(
                    "The Nostr private key could not be parsed: {}",
                    err
                ));
                return Ok(false);
            }
        };
        let bot_npub = keys
            .public_key()
            .to_bech32()
            .unwrap_or_else(|_| keys.public_key().to_hex());
        print_info(&format!("This agent will appear on Nostr as {}.", bot_npub));

        let owner_prompt = if owner_required {
            "Owner public key (hex or npub)"
        } else {
            "Owner public key (hex or npub, blank keeps command ingress disabled)"
        };
        let owner_hint = self.settings.channels.nostr_owner_pubkey.as_deref();
        let owner_pubkey = optional_input(owner_prompt, owner_hint)
            .map_err(SetupError::Io)?
            .filter(|value| !value.trim().is_empty())
            .map(|raw| crate::channels::nostr_runtime::normalize_public_key(&raw))
            .transpose()
            .map_err(SetupError::Config)?;

        if owner_required && owner_pubkey.is_none() {
            print_warning(
                "An owner public key is required so the agent knows whose DMs may control it.",
            );
            return Ok(false);
        }
        if owner_pubkey.is_none() {
            print_warning(
                "No Nostr owner public key was set, so inbound Nostr commands will stay disabled until you add one.",
            );
        }

        let social_dm_enabled = confirm(
            "Allow non-owner social DMs to be readable through nostr_actions?",
            self.settings.channels.nostr_social_dm_enabled,
        )
        .map_err(SetupError::Io)?;

        if let Some(ctx) = secrets {
            if let Err(err) = ctx
                .save_secret(
                    "nostr_private_key",
                    &SecretString::from(private_key.clone()),
                )
                .await
            {
                print_warning(&format!(
                    "Could not save the Nostr private key to the secrets store: {}",
                    err
                ));
                print_info(
                    "Set NOSTR_PRIVATE_KEY manually before starting if you do not retry secret storage.",
                );
            }
        } else {
            print_warning(
                "Secrets store not available during onboarding. Set NOSTR_PRIVATE_KEY manually before starting.",
            );
        }

        self.settings.channels.nostr_enabled = true;
        self.settings.channels.nostr_relays = Some(relays);
        self.settings.channels.nostr_owner_pubkey = owner_pubkey;
        self.settings.channels.nostr_social_dm_enabled = social_dm_enabled;
        self.settings.channels.nostr_allow_from = None;

        Ok(true)
    }

    fn ensure_wasm_channel_enabled(&mut self, channel_name: &str) {
        if !self
            .settings
            .channels
            .wasm_channels
            .iter()
            .any(|existing| existing == channel_name)
        {
            self.settings
                .channels
                .wasm_channels
                .push(channel_name.to_string());
        }
    }

    async fn install_and_discover_quick_wasm_channel(
        &mut self,
        channel_name: &str,
        display_name: &str,
    ) -> Result<Option<ChannelCapabilitiesFile>, SetupError> {
        let channels_dir = dirs::home_dir()
            .ok_or_else(|| SetupError::Config("Could not determine home directory".into()))?
            .join(".thinclaw/channels");

        let mut discovered = discover_wasm_channels(&channels_dir).await;
        let installed_names: HashSet<String> =
            discovered.iter().map(|(name, _)| name.clone()).collect();

        if !installed_names.contains(channel_name) {
            print_info(&format!("Preparing {}...", display_name));
            let selection = vec![channel_name.to_string()];
            let bundled_installed =
                install_selected_bundled_channels(&channels_dir, &selection, &installed_names)
                    .await?
                    .unwrap_or_default();
            let bundled_installed_set: HashSet<String> = bundled_installed.into_iter().collect();
            let _ = install_selected_registry_channels(
                &channels_dir,
                &selection,
                &installed_names,
                &bundled_installed_set,
            )
            .await;
            discovered = discover_wasm_channels(&channels_dir).await;
        }

        Ok(discovered
            .into_iter()
            .find(|(name, _)| name == channel_name)
            .map(|(_, caps)| caps))
    }

    async fn maybe_prepare_quick_channel_tunnel(
        &mut self,
        caps: &ChannelCapabilitiesFile,
        secrets: Option<&SecretsContext>,
    ) -> Result<(), SetupError> {
        if caps
            .capabilities
            .channel
            .as_ref()
            .is_some_and(|channel| !channel.allow_polling)
        {
            print_info("This channel needs a webhook endpoint, so ThinClaw is preparing a tunnel.");
            match setup_tunnel(&self.settings, secrets).await {
                Ok(tunnel_settings) => {
                    self.settings.tunnel = tunnel_settings;
                }
                Err(e) => {
                    return Err(SetupError::Channel(format!(
                        "Tunnel setup is required for this channel: {}",
                        e
                    )));
                }
            }
        }

        Ok(())
    }

    async fn configure_quick_telegram_channel(&mut self) -> Result<(), SetupError> {
        let caps = match self
            .install_and_discover_quick_wasm_channel("telegram", "Telegram")
            .await?
        {
            Some(caps) => caps,
            None => {
                self.quick_channel_install_failure("Telegram", "the channel package is missing");
                return Ok(());
            }
        };

        let secrets = self.init_secrets_context().await?;
        self.maybe_prepare_quick_channel_tunnel(&caps, Some(&secrets))
            .await?;

        let telegram_result = setup_telegram(&secrets, &self.settings).await?;
        if !telegram_result.enabled {
            self.quick_channel_install_failure(
                "Telegram",
                "setup was cancelled before the bot could be configured",
            );
            return Ok(());
        }

        if let Some(owner_id) = telegram_result.owner_id {
            self.settings.channels.telegram_owner_id = Some(owner_id);
        }
        self.ensure_wasm_channel_enabled("telegram");
        print_success("Telegram is ready as your primary channel.");
        Ok(())
    }

    async fn configure_quick_wasm_channel(
        &mut self,
        channel_name: &str,
        display_name: &str,
    ) -> Result<(), SetupError> {
        let caps = match self
            .install_and_discover_quick_wasm_channel(channel_name, display_name)
            .await?
        {
            Some(caps) => caps,
            None => {
                self.quick_channel_install_failure(
                    display_name,
                    "the channel package could not be installed",
                );
                return Ok(());
            }
        };

        let secrets = self.init_secrets_context().await?;
        self.maybe_prepare_quick_channel_tunnel(&caps, Some(&secrets))
            .await?;

        let setup_result = setup_wasm_channel(&secrets, channel_name, &caps.setup).await?;
        if !setup_result.enabled {
            self.quick_channel_install_failure(
                display_name,
                "setup was cancelled before the channel could be enabled",
            );
            return Ok(());
        }

        self.ensure_wasm_channel_enabled(&setup_result.channel_name);
        print_success(&format!(
            "{} is ready as your primary channel.",
            display_name
        ));
        Ok(())
    }

    pub(super) async fn step_primary_channel_quick(&mut self) -> Result<(), SetupError> {
        self.reset_quick_channel_selection();

        let channels_dir = dirs::home_dir()
            .ok_or_else(|| SetupError::Config("Could not determine home directory".into()))?
            .join(".thinclaw/channels");
        let discovered_channels = discover_wasm_channels(&channels_dir).await;
        let wasm_channel_names = build_channel_options(&discovered_channels);

        let mut options: Vec<(&str, String)> = vec![
            (
                "web",
                "Web Dashboard  - fastest start, built in, no extra install".to_string(),
            ),
            (
                "telegram",
                if wasm_channel_names.iter().any(|name| name == "telegram") {
                    "Telegram       - bot chat on Telegram".to_string()
                } else {
                    "Telegram       - bot chat on Telegram (will install)".to_string()
                },
            ),
            (
                "signal",
                "Signal         - direct messages through signal-cli".to_string(),
            ),
            (
                "discord",
                "Discord        - bot in Discord DMs or servers".to_string(),
            ),
            (
                "slack",
                "Slack          - workspace chat via bot/app tokens".to_string(),
            ),
            (
                "gmail",
                "Gmail          - email-triggered agent access".to_string(),
            ),
            (
                "bluebubbles",
                "BlueBubbles    - iMessage bridge from a Mac".to_string(),
            ),
            (
                "nostr",
                "Nostr          - relay-based messaging".to_string(),
            ),
        ];

        if wasm_channel_names.iter().any(|name| name == "whatsapp") {
            options.insert(
                2,
                (
                    "whatsapp",
                    "WhatsApp       - WhatsApp integration (will install)".to_string(),
                ),
            );
        }

        #[cfg(target_os = "macos")]
        {
            options.push((
                "imessage",
                "iMessage       - native macOS Messages access".to_string(),
            ));
            options.push((
                "apple_mail",
                "Apple Mail     - native macOS Mail access".to_string(),
            ));
        }

        let labels: Vec<&str> = options.iter().map(|(_, label)| label.as_str()).collect();
        print_info("Choose the main place people will use to reach ThinClaw.");
        print_info("Quick setup configures only the essentials for that channel.");
        crate::setup::prompts::print_blank_line();
        let choice = select_one("Primary channel", &labels).map_err(SetupError::Io)?;
        let selected = options
            .get(choice)
            .map(|(key, _)| *key)
            .unwrap_or("web")
            .to_string();

        self.quick_primary_channel = Some(selected.clone());
        match selected.as_str() {
            "web" => {
                print_success("Web Dashboard selected as your primary channel.");
            }
            "telegram" => {
                self.configure_quick_telegram_channel().await?;
            }
            "whatsapp" => {
                self.configure_quick_wasm_channel("whatsapp", "WhatsApp")
                    .await?;
            }
            "signal" => {
                print_info("Configuring Signal with the minimum required fields.");
                let result = setup_signal(&self.settings).await?;
                self.settings.channels.signal_http_url = Some(result.http_url);
                self.settings.channels.signal_account = Some(result.account.clone());
                self.settings.channels.signal_allow_from = Some(result.allow_from);
                self.settings.channels.signal_allow_from_groups = Some(result.allow_from_groups);
                self.settings.channels.signal_dm_policy = Some(result.dm_policy);
                self.settings.channels.signal_group_policy = Some(result.group_policy);
                self.settings.channels.signal_group_allow_from = Some(result.group_allow_from);
                self.settings.channels.signal_enabled = result.enabled;
            }
            "discord" => {
                print_info("Discord needs a bot token to connect.");
                let token = secret_input("Discord bot token").map_err(SetupError::Io)?;
                let token = token.expose_secret().trim().to_string();
                if token.is_empty() {
                    self.quick_channel_install_failure(
                        "Discord",
                        "a bot token is required to make the channel usable",
                    );
                } else {
                    self.settings.channels.discord_bot_token = Some(token);
                    self.settings.channels.discord_enabled = true;
                    print_success("Discord is ready as your primary channel.");
                }
            }
            "slack" => {
                print_info("Slack needs both a bot token and an app token for Socket Mode.");
                let bot_token = secret_input("Slack bot token (xoxb-...)")
                    .map_err(SetupError::Io)?
                    .expose_secret()
                    .trim()
                    .to_string();
                let app_token = secret_input("Slack app token (xapp-...)")
                    .map_err(SetupError::Io)?
                    .expose_secret()
                    .trim()
                    .to_string();
                if bot_token.is_empty() || app_token.is_empty() {
                    self.quick_channel_install_failure(
                        "Slack",
                        "both the bot token and app token are required",
                    );
                } else {
                    self.settings.channels.slack_bot_token = Some(bot_token);
                    self.settings.channels.slack_app_token = Some(app_token);
                    self.settings.channels.slack_enabled = true;
                    print_success("Slack is ready as your primary channel.");
                }
            }
            "gmail" => {
                print_info("Gmail needs the Pub/Sub project details for push notifications.");
                let project_id = input("GCP project ID").map_err(SetupError::Io)?;
                let subscription_id = input("Pub/Sub subscription ID").map_err(SetupError::Io)?;
                let topic_id = input("Pub/Sub topic ID").map_err(SetupError::Io)?;
                if project_id.trim().is_empty()
                    || subscription_id.trim().is_empty()
                    || topic_id.trim().is_empty()
                {
                    self.quick_channel_install_failure(
                        "Gmail",
                        "project, subscription, and topic are all required",
                    );
                } else {
                    self.settings.channels.gmail_project_id = Some(project_id);
                    self.settings.channels.gmail_subscription_id = Some(subscription_id);
                    self.settings.channels.gmail_topic_id = Some(topic_id);
                    self.settings.channels.gmail_enabled = true;
                    print_success("Gmail is ready as your primary channel.");
                }
            }
            "bluebubbles" => {
                print_info("BlueBubbles needs the server URL and password from your Mac bridge.");
                let server_url = input("BlueBubbles server URL").map_err(SetupError::Io)?;
                let password = secret_input("BlueBubbles server password")
                    .map_err(SetupError::Io)?
                    .expose_secret()
                    .trim()
                    .to_string();
                if server_url.trim().is_empty() || password.is_empty() {
                    self.quick_channel_install_failure(
                        "BlueBubbles",
                        "both the server URL and password are required",
                    );
                } else {
                    self.settings.channels.bluebubbles_server_url = Some(server_url);
                    self.settings.channels.bluebubbles_password = Some(password);
                    self.settings.channels.bluebubbles_webhook_host = Some("127.0.0.1".to_string());
                    self.settings.channels.bluebubbles_webhook_port = Some(8645);
                    self.settings.channels.bluebubbles_enabled = true;
                    print_success("BlueBubbles is ready as your primary channel.");
                }
            }
            #[cfg(feature = "nostr")]
            "nostr" => {
                let secrets = self.init_secrets_context().await.ok();
                if self.configure_nostr_channel(secrets.as_ref(), true).await? {
                    print_success("Nostr is ready as your primary channel.");
                } else {
                    self.quick_channel_install_failure(
                        "Nostr",
                        "a valid private key and owner public key are required",
                    );
                }
            }
            #[cfg(not(feature = "nostr"))]
            "nostr" => {
                self.quick_channel_install_failure(
                    "Nostr",
                    "this build was compiled without Nostr support (--features nostr)",
                );
            }
            #[cfg(target_os = "macos")]
            "imessage" => {
                self.settings.channels.imessage_enabled = true;
                self.settings.channels.imessage_poll_interval = Some(5);
                print_success("iMessage is ready as your primary channel.");
            }
            #[cfg(target_os = "macos")]
            "apple_mail" => {
                self.settings.channels.apple_mail_enabled = true;
                self.settings.channels.apple_mail_poll_interval = Some(10);
                self.settings.channels.apple_mail_unread_only = true;
                self.settings.channels.apple_mail_mark_as_read = true;
                print_success("Apple Mail is ready as your primary channel.");
            }
            _ => {
                self.quick_channel_install_failure(
                    "Selected channel",
                    "this quick-setup path is not available yet",
                );
            }
        }

        Ok(())
    }

    pub(super) async fn init_secrets_context(&mut self) -> Result<SecretsContext, SetupError> {
        // Get crypto (should be set from step 2, or load from OS secure store/env)
        let crypto = if let Some(ref c) = self.secrets_crypto {
            Arc::clone(c)
        } else {
            // Try to load master key from the OS secure store or env
            let key = if let Ok(env_key) = std::env::var("SECRETS_MASTER_KEY") {
                env_key
            } else if let Ok(keychain_key) = crate::platform::secure_store::get_master_key().await {
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
        crate::setup::prompts::print_blank_line();

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

        print_info("Choose the channels ThinClaw should listen on.");
        print_info("Channels are grouped into Common, Native, WASM, and Advanced.");
        print_info(
            "Bundled or registry-backed WASM channels are installed automatically when selected.",
        );
        print_info(
            "A verification pass runs after setup and does not send live messages unless you test later.",
        );
        crate::setup::prompts::print_blank_line();

        print_info("Common channels");
        let common_options = [("HTTP webhook", self.settings.channels.http_enabled)];
        let common_selected =
            select_many("Enable common channels", &common_options).map_err(SetupError::Io)?;
        let enable_http = common_selected.contains(&0);
        crate::setup::prompts::print_blank_line();

        print_info("Native channels");
        #[allow(unused_mut)]
        let mut native_options: Vec<(&str, bool)> = vec![
            ("Signal", self.settings.channels.signal_enabled),
            ("Discord", self.settings.channels.discord_enabled),
            ("Slack", self.settings.channels.slack_enabled),
            ("Gmail", self.settings.channels.gmail_enabled),
            (
                "BlueBubbles (iMessage bridge)",
                self.settings.channels.bluebubbles_enabled,
            ),
        ];
        #[allow(unused_mut)]
        let mut native_keys: Vec<&str> = vec!["signal", "discord", "slack", "gmail", "bluebubbles"];
        #[cfg(target_os = "macos")]
        {
            native_options.push(("iMessage", self.settings.channels.imessage_enabled));
            native_options.push(("Apple Mail", self.settings.channels.apple_mail_enabled));
            native_keys.push("imessage");
            native_keys.push("apple_mail");
        }
        let native_selected =
            select_many("Enable native channels", &native_options).map_err(SetupError::Io)?;
        let pick_native = |key: &str| {
            native_keys
                .iter()
                .position(|existing| *existing == key)
                .is_some_and(|idx| native_selected.contains(&idx))
        };
        let enable_signal = pick_native("signal");
        let enable_discord = pick_native("discord");
        let enable_slack = pick_native("slack");
        let enable_gmail = pick_native("gmail");
        let enable_bluebubbles = pick_native("bluebubbles");
        #[cfg(target_os = "macos")]
        let enable_imessage = pick_native("imessage");
        #[cfg(target_os = "macos")]
        let enable_apple_mail = pick_native("apple_mail");
        crate::setup::prompts::print_blank_line();

        print_info("Advanced channels");
        let advanced_options = [("Nostr", self.settings.channels.nostr_enabled)];
        let advanced_selected =
            select_many("Enable advanced channels", &advanced_options).map_err(SetupError::Io)?;
        let enable_nostr = advanced_selected.contains(&0);
        crate::setup::prompts::print_blank_line();

        let selected_wasm_channels = if wasm_channel_names.is_empty() {
            Vec::new()
        } else {
            print_info("WASM channels");
            let wasm_options: Vec<(String, bool)> = wasm_channel_names
                .iter()
                .map(|name| {
                    let enabled = self.settings.channels.wasm_channels.contains(name);
                    let label = if installed_names.contains(name) {
                        format!("{} (installed)", capitalize_first(name))
                    } else {
                        format!("{} (will install)", capitalize_first(name))
                    };
                    (label, enabled)
                })
                .collect();
            let wasm_option_refs: Vec<(&str, bool)> = wasm_options
                .iter()
                .map(|(label, enabled)| (label.as_str(), *enabled))
                .collect();
            let wasm_selected =
                select_many("Enable WASM channels", &wasm_option_refs).map_err(SetupError::Io)?;
            crate::setup::prompts::print_blank_line();
            wasm_selected
                .into_iter()
                .filter_map(|idx| wasm_channel_names.get(idx).cloned())
                .collect()
        };

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

        let needs_secrets =
            enable_http || enable_discord || enable_slack || !selected_wasm_channels.is_empty();
        if needs_secrets && secrets.is_none() {
            print_info(
                "Secrets not available. Channel tokens must be set via environment variables.",
            );
        }

        // HTTP channel
        if enable_http {
            crate::setup::prompts::print_blank_line();
            if let Some(ref ctx) = secrets {
                let result = setup_http(ctx).await?;
                self.settings.channels.http_enabled = result.enabled;
                self.settings.channels.http_port = Some(result.port);
                self.settings.channels.http_host = Some(result.host);
            } else {
                self.settings.channels.http_enabled = true;
                self.settings.channels.http_port = Some(8080);
                self.settings.channels.http_host = Some("0.0.0.0".to_string());
                print_info(
                    "HTTP webhook enabled on port 8080. Set HTTP_WEBHOOK_SECRET in your environment.",
                );
            }
        } else {
            self.settings.channels.http_enabled = false;
        }

        // Signal channel
        if enable_signal {
            crate::setup::prompts::print_blank_line();
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
        if enable_discord {
            crate::setup::prompts::print_blank_line();
            print_info(
                "Discord needs a bot token from https://discord.com/developers/applications",
            );
            crate::setup::prompts::print_blank_line();

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
            if let Some(ref ctx) = secrets
                && let Err(e) = ctx
                    .save_secret(
                        "discord_bot_token",
                        &secrecy::SecretString::from(token.clone()),
                    )
                    .await
            {
                print_info(&format!("Could not store token in secrets: {}", e));
            }
            self.settings.channels.discord_bot_token =
                if secrets.is_some() { None } else { Some(token) };

            let guild_id = optional_input("Guild ID (limit to one server, blank = all)", None)
                .map_err(SetupError::Io)?;
            if let Some(ref gid) = guild_id
                && !gid.is_empty()
            {
                self.settings.channels.discord_guild_id = Some(gid.clone());
            }

            let allow_from =
                optional_input("Allowed channel IDs (comma-separated, blank = all)", None)
                    .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from
                && !af.is_empty()
            {
                self.settings.channels.discord_allow_from = Some(af.clone());
            }

            self.settings.channels.discord_enabled = true;
            print_success("Discord channel configured");
        } else {
            self.settings.channels.discord_enabled = false;
            self.settings.channels.discord_bot_token = None;
        }

        // Slack channel
        if enable_slack {
            crate::setup::prompts::print_blank_line();
            print_info(
                "Slack needs both a bot token (xoxb-...) and an app-level token (xapp-...).",
            );
            print_info("Create them at https://api.slack.com/apps");
            crate::setup::prompts::print_blank_line();

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
            if let Some(ref af) = allow_from
                && !af.is_empty()
            {
                self.settings.channels.slack_allow_from = Some(af.clone());
            }

            self.settings.channels.slack_enabled = true;
            print_success("Slack channel configured");
        } else {
            self.settings.channels.slack_enabled = false;
            self.settings.channels.slack_bot_token = None;
            self.settings.channels.slack_app_token = None;
        }

        // Nostr channel
        if enable_nostr {
            #[cfg(feature = "nostr")]
            {
                crate::setup::prompts::print_blank_line();
                print_info("Nostr connects to relay servers to receive and send messages.");
                crate::setup::prompts::print_blank_line();
                if self
                    .configure_nostr_channel(secrets.as_ref(), false)
                    .await?
                {
                    print_success("Nostr channel configured");
                } else {
                    self.settings.channels.nostr_enabled = false;
                    self.settings.channels.nostr_owner_pubkey = None;
                    self.settings.channels.nostr_social_dm_enabled = false;
                    print_warning(
                        "Nostr channel was left disabled because the key or owner configuration was incomplete.",
                    );
                }
            }
            #[cfg(not(feature = "nostr"))]
            {
                print_warning("Nostr support is not available in this build (--features nostr).");
                self.settings.channels.nostr_enabled = false;
            }
        } else {
            self.settings.channels.nostr_enabled = false;
            self.settings.channels.nostr_owner_pubkey = None;
            self.settings.channels.nostr_social_dm_enabled = false;
        }

        // Gmail channel
        if enable_gmail {
            crate::setup::prompts::print_blank_line();
            print_info("Gmail requires GCP project with Pub/Sub and Gmail API enabled.");
            print_info("Follow: https://developers.google.com/gmail/api/guides/push");
            crate::setup::prompts::print_blank_line();

            let project_id = input("GCP project ID").map_err(SetupError::Io)?;
            self.settings.channels.gmail_project_id = Some(project_id);

            let sub_id = input("Pub/Sub subscription ID").map_err(SetupError::Io)?;
            self.settings.channels.gmail_subscription_id = Some(sub_id);

            let topic_id = input("Pub/Sub topic ID").map_err(SetupError::Io)?;
            self.settings.channels.gmail_topic_id = Some(topic_id);

            let allowed_senders =
                optional_input("Allowed sender emails (comma-separated, blank = all)", None)
                    .map_err(SetupError::Io)?;
            if let Some(ref senders) = allowed_senders
                && !senders.is_empty()
            {
                self.settings.channels.gmail_allowed_senders = Some(senders.clone());
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
        if enable_imessage {
            crate::setup::prompts::print_blank_line();
            print_info("iMessage uses the native macOS Messages database.");
            print_info("Grant Full Disk Access in System Settings > Privacy.");
            crate::setup::prompts::print_blank_line();

            let allow_from = optional_input(
                "Allowed contacts (comma-separated phone/email, blank = all)",
                None,
            )
            .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from
                && !af.is_empty()
            {
                self.settings.channels.imessage_allow_from = Some(af.clone());
            }

            let poll_interval =
                optional_input("Polling interval in seconds", Some("5")).map_err(SetupError::Io)?;
            if let Some(ref pi) = poll_interval
                && let Ok(n) = pi.parse::<u64>()
            {
                self.settings.channels.imessage_poll_interval = Some(n);
            }

            self.settings.channels.imessage_enabled = true;
            print_success("iMessage channel configured");
        }
        #[cfg(target_os = "macos")]
        if !enable_imessage {
            self.settings.channels.imessage_enabled = false;
        }

        // Apple Mail channel (macOS only)
        #[cfg(target_os = "macos")]
        if enable_apple_mail {
            crate::setup::prompts::print_blank_line();
            print_info("Apple Mail uses the native macOS Mail.app Envelope Index database.");
            print_info("Grant Full Disk Access in System Settings > Privacy.");
            print_info("Make sure Mail.app is signed in and already configured.");
            print_info(
                "Important: if you leave this blank, any email sender can give instructions to your agent.",
            );
            print_info(
                "For safety, enter your own email addresses so only you can control it through email.",
            );
            crate::setup::prompts::print_blank_line();

            let allow_from = optional_input(
                "Allowed email addresses (comma-separated, blank = anyone can control the agent)",
                None,
            )
            .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from
                && !af.is_empty()
            {
                self.settings.channels.apple_mail_allow_from = Some(af.clone());
            }

            let poll_interval = optional_input("Polling interval in seconds", Some("10"))
                .map_err(SetupError::Io)?;
            if let Some(ref pi) = poll_interval
                && let Ok(n) = pi.parse::<u64>()
            {
                self.settings.channels.apple_mail_poll_interval = Some(n);
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
        if !enable_apple_mail {
            self.settings.channels.apple_mail_enabled = false;
        }

        // BlueBubbles iMessage bridge (cross-platform)
        if enable_bluebubbles {
            crate::setup::prompts::print_blank_line();
            #[cfg(target_os = "macos")]
            {
                print_info("BlueBubbles bridges iMessage via a dedicated macOS server app.");
                print_info("Unlike the native iMessage channel (which polls chat.db read-only),");
                print_info(
                    "BlueBubbles adds: typing indicators, read receipts, tapback reactions,",
                );
                print_info("group chat management, and cross-platform access from Linux/Windows.");
                print_info("Both channels can coexist — native for lightweight local use,");
                print_info("BlueBubbles for full-featured or remote deployments.");
            }
            #[cfg(not(target_os = "macos"))]
            {
                print_info(
                    "BlueBubbles bridges iMessage from a Mac running the BlueBubbles server.",
                );
                print_info(
                    "It works from any platform (Linux, Windows, macOS) over REST API + webhooks.",
                );
            }
            print_info("Download the server: https://bluebubbles.app/");
            crate::setup::prompts::print_blank_line();

            let server_url = input("BlueBubbles server URL (e.g. http://192.168.1.50:1234)")
                .map_err(SetupError::Io)?;
            self.settings.channels.bluebubbles_server_url = Some(server_url);

            let password = secret_input("BlueBubbles server password").map_err(SetupError::Io)?;
            self.settings.channels.bluebubbles_password =
                Some(password.expose_secret().to_string());

            let webhook_host =
                optional_input("Webhook listen host (this machine)", Some("127.0.0.1"))
                    .map_err(SetupError::Io)?;
            if let Some(ref host) = webhook_host
                && !host.is_empty()
            {
                self.settings.channels.bluebubbles_webhook_host = Some(host.clone());
            }

            let webhook_port =
                optional_input("Webhook listen port", Some("8645")).map_err(SetupError::Io)?;
            if let Some(ref port) = webhook_port
                && !port.is_empty()
                && let Ok(p) = port.parse::<u16>()
            {
                self.settings.channels.bluebubbles_webhook_port = Some(p);
            }

            let allow_from = optional_input(
                "Allowed contacts (comma-separated phone/email, blank = all)",
                None,
            )
            .map_err(SetupError::Io)?;
            if let Some(ref af) = allow_from
                && !af.is_empty()
            {
                self.settings.channels.bluebubbles_allow_from = Some(af.clone());
            }

            let send_receipts =
                confirm("Send read receipts (requires Private API on server)?", true)
                    .map_err(SetupError::Io)?;
            self.settings.channels.bluebubbles_send_read_receipts = Some(send_receipts);

            self.settings.channels.bluebubbles_enabled = true;
            print_success("BlueBubbles iMessage bridge configured");
            print_info(
                "Make sure the BlueBubbles server is reachable from this machine before starting.",
            );
        } else {
            self.settings.channels.bluebubbles_enabled = false;
        }

        let discovered_by_name: HashMap<String, ChannelCapabilitiesFile> =
            discovered_channels.into_iter().collect();

        // Process selected WASM channels
        let mut enabled_wasm_channels = Vec::new();
        for channel_name in selected_wasm_channels {
            crate::setup::prompts::print_blank_line();
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
                            "No setup instructions are available for {}",
                            channel_name
                        ));
                        crate::setup::channels::WasmChannelSetupResult {
                            enabled: true,
                            channel_name: channel_name.clone(),
                        }
                    }
                } else {
                    print_info(&format!(
                        "Selected channel '{}' is not available on disk.",
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
                    "{} enabled; configure tokens through environment variables.",
                    capitalize_first(&channel_name)
                ));
                enabled_wasm_channels.push(channel_name.clone());
            }
        }

        self.settings.channels.wasm_channels = enabled_wasm_channels;

        Ok(())
    }
}
