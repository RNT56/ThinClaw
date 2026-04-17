//! Settings persistence: save/load wizard settings to/from database.

use crate::settings::Settings;

use super::{SetupError, SetupWizard};

impl SetupWizard {
    pub(super) async fn persist_settings(&self) -> Result<bool, SetupError> {
        let db_map = self.settings.to_db_map();
        let saved = false;

        #[cfg(feature = "postgres")]
        let saved = if !saved {
            if let Some(ref pool) = self.db_pool {
                let store = crate::history::Store::from_pool(pool.clone());
                store
                    .set_all_settings("default", &db_map)
                    .await
                    .map_err(|e| {
                        SetupError::Database(format!("Failed to save settings to database: {}", e))
                    })?;
                true
            } else {
                false
            }
        } else {
            saved
        };

        #[cfg(feature = "libsql")]
        let saved = if !saved {
            if let Some(ref backend) = self.db_backend {
                use crate::db::SettingsStore as _;
                backend
                    .set_all_settings("default", &db_map)
                    .await
                    .map_err(|e| {
                        SetupError::Database(format!("Failed to save settings to database: {}", e))
                    })?;
                true
            } else {
                false
            }
        } else {
            saved
        };

        Ok(saved)
    }

    /// Write bootstrap environment variables to `~/.thinclaw/.env`.
    ///
    /// These are the chicken-and-egg settings needed before the database is
    /// connected (DATABASE_BACKEND, DATABASE_URL, LLM_BACKEND, etc.).
    pub(super) fn write_bootstrap_env(&self) -> Result<(), SetupError> {
        let mut env_vars: Vec<(&str, String)> = Vec::new();

        if let Some(ref backend) = self.settings.database_backend {
            env_vars.push(("DATABASE_BACKEND", backend.clone()));
        }
        if let Some(ref url) = self.settings.database_url {
            env_vars.push(("DATABASE_URL", url.clone()));
        }
        if let Some(ref path) = self.settings.libsql_path {
            env_vars.push(("LIBSQL_PATH", path.clone()));
        }
        if let Some(ref url) = self.settings.libsql_url {
            env_vars.push(("LIBSQL_URL", url.clone()));
        }

        // LLM bootstrap vars: same chicken-and-egg problem as DATABASE_BACKEND.
        // Config::from_env() needs the backend before the DB is connected.
        if let Some(ref backend) = self.settings.llm_backend {
            env_vars.push(("LLM_BACKEND", backend.clone()));
        }
        if let Some(ref url) = self.settings.openai_compatible_base_url {
            env_vars.push(("LLM_BASE_URL", url.clone()));
        }
        if let Some(ref url) = self.settings.ollama_base_url {
            env_vars.push(("OLLAMA_BASE_URL", url.clone()));
        }

        // Always write ONBOARD_COMPLETED so that check_onboard_needed()
        // (which runs before the DB is connected) knows to skip re-onboarding.
        if self.settings.onboard_completed {
            env_vars.push(("ONBOARD_COMPLETED", "true".to_string()));
        }

        // Signal channel env vars (chicken-and-egg: config resolves before DB).
        if let Some(ref url) = self.settings.channels.signal_http_url {
            env_vars.push(("SIGNAL_HTTP_URL", url.clone()));
        }
        if let Some(ref account) = self.settings.channels.signal_account {
            env_vars.push(("SIGNAL_ACCOUNT", account.clone()));
        }
        if let Some(ref allow_from) = self.settings.channels.signal_allow_from {
            env_vars.push(("SIGNAL_ALLOW_FROM", allow_from.clone()));
        }
        if let Some(ref allow_from_groups) = self.settings.channels.signal_allow_from_groups
            && !allow_from_groups.is_empty()
        {
            env_vars.push(("SIGNAL_ALLOW_FROM_GROUPS", allow_from_groups.clone()));
        }
        if let Some(ref dm_policy) = self.settings.channels.signal_dm_policy {
            env_vars.push(("SIGNAL_DM_POLICY", dm_policy.clone()));
        }
        if let Some(ref group_policy) = self.settings.channels.signal_group_policy {
            env_vars.push(("SIGNAL_GROUP_POLICY", group_policy.clone()));
        }
        if let Some(ref group_allow_from) = self.settings.channels.signal_group_allow_from
            && !group_allow_from.is_empty()
        {
            env_vars.push(("SIGNAL_GROUP_ALLOW_FROM", group_allow_from.clone()));
        }

        // HTTP channel bootstrap vars
        if self.settings.channels.http_enabled {
            env_vars.push(("HTTP_ENABLED", "true".to_string()));
            if let Some(ref host) = self.settings.channels.http_host {
                env_vars.push(("HTTP_HOST", host.clone()));
            }
            if let Some(port) = self.settings.channels.http_port {
                env_vars.push(("HTTP_PORT", port.to_string()));
            }
        }

        // Discord channel env vars
        if self.settings.channels.discord_enabled {
            env_vars.push(("DISCORD_ENABLED", "true".to_string()));
            if let Some(ref token) = self.settings.channels.discord_bot_token {
                env_vars.push(("DISCORD_BOT_TOKEN", token.clone()));
            }
        }
        if let Some(ref guild_id) = self.settings.channels.discord_guild_id {
            env_vars.push(("DISCORD_GUILD_ID", guild_id.clone()));
        }
        if let Some(ref allow_from) = self.settings.channels.discord_allow_from {
            env_vars.push(("DISCORD_ALLOW_FROM", allow_from.clone()));
        }

        // Slack channel env vars
        if self.settings.channels.slack_enabled {
            env_vars.push(("SLACK_ENABLED", "true".to_string()));
            if let Some(ref token) = self.settings.channels.slack_bot_token {
                env_vars.push(("SLACK_BOT_TOKEN", token.clone()));
            }
            if let Some(ref token) = self.settings.channels.slack_app_token {
                env_vars.push(("SLACK_APP_TOKEN", token.clone()));
            }
        }
        if let Some(ref allow_from) = self.settings.channels.slack_allow_from {
            env_vars.push(("SLACK_ALLOW_FROM", allow_from.clone()));
        }

        // Nostr channel env vars
        if self.settings.channels.nostr_enabled {
            env_vars.push(("NOSTR_ENABLED", "true".to_string()));
        }
        if let Some(ref relays) = self.settings.channels.nostr_relays {
            env_vars.push(("NOSTR_RELAYS", relays.clone()));
        }
        if let Some(ref allow_from) = self.settings.channels.nostr_allow_from {
            env_vars.push(("NOSTR_ALLOW_FROM", allow_from.clone()));
        }

        // Gmail channel env vars
        if self.settings.channels.gmail_enabled {
            env_vars.push(("GMAIL_ENABLED", "true".to_string()));
        }
        if let Some(ref project_id) = self.settings.channels.gmail_project_id {
            env_vars.push(("GMAIL_PROJECT_ID", project_id.clone()));
        }
        if let Some(ref sub_id) = self.settings.channels.gmail_subscription_id {
            env_vars.push(("GMAIL_SUBSCRIPTION_ID", sub_id.clone()));
        }
        if let Some(ref topic_id) = self.settings.channels.gmail_topic_id {
            env_vars.push(("GMAIL_TOPIC_ID", topic_id.clone()));
        }
        if let Some(ref senders) = self.settings.channels.gmail_allowed_senders {
            env_vars.push(("GMAIL_ALLOWED_SENDERS", senders.clone()));
        }

        // iMessage channel env vars
        if self.settings.channels.imessage_enabled {
            env_vars.push(("IMESSAGE_ENABLED", "true".to_string()));
        }
        if let Some(ref allow_from) = self.settings.channels.imessage_allow_from {
            env_vars.push(("IMESSAGE_ALLOW_FROM", allow_from.clone()));
        }
        if let Some(ref interval) = self.settings.channels.imessage_poll_interval {
            env_vars.push(("IMESSAGE_POLL_INTERVAL", interval.to_string()));
        }

        // Apple Mail channel env vars
        if self.settings.channels.apple_mail_enabled {
            env_vars.push(("APPLE_MAIL_ENABLED", "true".to_string()));
        }
        if let Some(ref allow_from) = self.settings.channels.apple_mail_allow_from {
            env_vars.push(("APPLE_MAIL_ALLOW_FROM", allow_from.clone()));
        }
        if let Some(ref interval) = self.settings.channels.apple_mail_poll_interval {
            env_vars.push(("APPLE_MAIL_POLL_INTERVAL", interval.to_string()));
        }
        if !self.settings.channels.apple_mail_unread_only {
            env_vars.push(("APPLE_MAIL_UNREAD_ONLY", "false".to_string()));
        }
        if !self.settings.channels.apple_mail_mark_as_read {
            env_vars.push(("APPLE_MAIL_MARK_AS_READ", "false".to_string()));
        }

        // BlueBubbles iMessage bridge env vars
        if self.settings.channels.bluebubbles_enabled {
            env_vars.push(("BLUEBUBBLES_ENABLED", "true".to_string()));
        }
        if let Some(ref url) = self.settings.channels.bluebubbles_server_url {
            env_vars.push(("BLUEBUBBLES_SERVER_URL", url.clone()));
        }
        if let Some(ref password) = self.settings.channels.bluebubbles_password {
            env_vars.push(("BLUEBUBBLES_PASSWORD", password.clone()));
        }
        if let Some(ref host) = self.settings.channels.bluebubbles_webhook_host {
            env_vars.push(("BLUEBUBBLES_WEBHOOK_HOST", host.clone()));
        }
        if let Some(port) = self.settings.channels.bluebubbles_webhook_port {
            env_vars.push(("BLUEBUBBLES_WEBHOOK_PORT", port.to_string()));
        }
        if let Some(ref allow_from) = self.settings.channels.bluebubbles_allow_from {
            env_vars.push(("BLUEBUBBLES_ALLOW_FROM", allow_from.clone()));
        }
        if let Some(send_receipts) = self.settings.channels.bluebubbles_send_read_receipts {
            env_vars.push(("BLUEBUBBLES_SEND_READ_RECEIPTS", send_receipts.to_string()));
        }

        // Web Gateway env vars
        if let Some(ref port) = self.settings.channels.gateway_port {
            env_vars.push(("GATEWAY_PORT", port.to_string()));
        }
        if let Some(ref token) = self.settings.channels.gateway_auth_token {
            env_vars.push(("GATEWAY_AUTH_TOKEN", token.clone()));
        }

        // Smart Routing env vars
        if let Some(ref model) = self.settings.providers.cheap_model {
            env_vars.push(("LLM_CHEAP_MODEL", model.clone()));
        }

        // Web UI env vars
        if let Some(ref skin) = self.settings.webchat_skin {
            env_vars.push(("WEBCHAT_SKIN", skin.clone()));
        }
        if self.settings.webchat_theme != "system" {
            env_vars.push(("WEBCHAT_THEME", self.settings.webchat_theme.clone()));
        }
        if let Some(ref color) = self.settings.webchat_accent_color {
            env_vars.push(("WEBCHAT_ACCENT_COLOR", color.clone()));
        }
        if !self.settings.webchat_show_branding {
            env_vars.push(("WEBCHAT_SHOW_BRANDING", "false".to_string()));
        }

        // Observability env vars
        if self.settings.observability_backend != "none" {
            env_vars.push((
                "OBSERVABILITY_BACKEND",
                self.settings.observability_backend.clone(),
            ));
        }

        // Agent local tools
        if self.settings.agent.allow_local_tools {
            env_vars.push(("ALLOW_LOCAL_TOOLS", "true".to_string()));
        }

        // Agent workspace mode (unrestricted, sandboxed, project)
        if let Some(ref mode) = self.settings.agent.workspace_mode {
            env_vars.push(("WORKSPACE_MODE", mode.clone()));
        }

        if !env_vars.is_empty() {
            let pairs: Vec<(&str, &str)> = env_vars.iter().map(|(k, v)| (*k, v.as_str())).collect();
            crate::bootstrap::save_bootstrap_env(&pairs).map_err(|e| {
                SetupError::Io(std::io::Error::other(format!(
                    "Failed to save bootstrap env to .env: {}",
                    e
                )))
            })?;
        }

        Ok(())
    }

    /// Persist settings to DB and bootstrap .env after each step.
    ///
    /// Silently ignores errors (e.g., DB not connected yet before step 1
    /// completes). This is best-effort incremental persistence.
    pub(super) async fn persist_after_step(&self) {
        // Write bootstrap .env (always possible)
        if let Err(e) = self.write_bootstrap_env() {
            tracing::debug!("Could not write bootstrap env after step: {}", e);
        }

        // Persist to DB
        match self.persist_settings().await {
            Ok(true) => tracing::debug!("Settings persisted to database after step"),
            Ok(false) => tracing::debug!("No DB connection yet, skipping settings persist"),
            Err(e) => tracing::debug!("Could not persist settings after step: {}", e),
        }
    }

    /// Load previously saved settings from the database after Step 1
    /// establishes a connection.
    ///
    /// This enables recovery from partial onboarding runs: if the user
    /// completed steps 1-4 previously but step 5 failed, re-running
    /// the wizard will pre-populate settings from the database.
    ///
    /// **Callers must re-apply any wizard choices made before this call**
    /// via `self.settings.merge_from(&step_settings)`, since `merge_from`
    /// prefers the `other` argument's non-default values. Without this,
    /// stale DB values would overwrite fresh user choices.
    pub(super) async fn try_load_existing_settings(&mut self) {
        let loaded = false;

        #[cfg(feature = "postgres")]
        let loaded = if !loaded {
            if let Some(ref pool) = self.db_pool {
                let store = crate::history::Store::from_pool(pool.clone());
                match store.get_all_settings("default").await {
                    Ok(db_map) if !db_map.is_empty() => {
                        let existing = Settings::from_db_map(&db_map);
                        self.settings.merge_from(&existing);
                        tracing::info!("Loaded {} existing settings from database", db_map.len());
                        true
                    }
                    Ok(_) => false,
                    Err(e) => {
                        tracing::debug!("Could not load existing settings: {}", e);
                        false
                    }
                }
            } else {
                false
            }
        } else {
            loaded
        };

        #[cfg(feature = "libsql")]
        let loaded = if !loaded {
            if let Some(ref backend) = self.db_backend {
                use crate::db::SettingsStore as _;
                match backend.get_all_settings("default").await {
                    Ok(db_map) if !db_map.is_empty() => {
                        let existing = Settings::from_db_map(&db_map);
                        self.settings.merge_from(&existing);
                        tracing::info!("Loaded {} existing settings from database", db_map.len());
                        true
                    }
                    Ok(_) => false,
                    Err(e) => {
                        tracing::debug!("Could not load existing settings: {}", e);
                        false
                    }
                }
            } else {
                false
            }
        } else {
            loaded
        };

        // Suppress unused variable warning when only one backend is compiled.
        let _ = loaded;
    }
}
