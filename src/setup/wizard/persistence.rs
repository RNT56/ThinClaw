//! Settings persistence: save/load wizard settings to/from database.

use crate::settings::Settings;

use super::{SetupError, SetupWizard};
use thinclaw_app::{
    SetupBootstrapAgentInput, SetupBootstrapChannelInput, SetupBootstrapEnvInput,
    SetupBootstrapProviderInput, SetupBootstrapWebUiInput, SetupRuntimeProfile,
    setup_bootstrap_env_plan,
};

impl SetupWizard {
    pub(super) async fn persist_settings(&self) -> Result<bool, SetupError> {
        let db_map: std::collections::HashMap<_, _> = self
            .settings
            .to_db_map()
            .into_iter()
            .filter(|(key, _)| !super::transaction::credential_shaped_key(key))
            .collect();
        let saved = false;

        #[cfg(feature = "postgres")]
        let saved = if !saved {
            if let Some(ref pool) = self.db_pool {
                use crate::db::SettingsStore as _;
                let store = crate::db::postgres::PgBackend::from_pool(pool.clone());
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
        let input = self.bootstrap_env_input();
        let plan = setup_bootstrap_env_plan(&input);

        if !plan.is_empty() {
            let pairs: Vec<(&str, &str)> = plan
                .variables()
                .iter()
                .map(|var| (var.key, var.value.as_str()))
                .collect();
            crate::bootstrap::save_bootstrap_env(&pairs).map_err(|e| {
                SetupError::Io(std::io::Error::other(format!(
                    "Failed to save bootstrap env to .env: {}",
                    e
                )))
            })?;
        }

        Ok(())
    }

    fn bootstrap_env_input(&self) -> SetupBootstrapEnvInput {
        let channels = &self.settings.channels;
        let env_key_source =
            self.settings.secrets_master_key_source == crate::settings::KeySource::Env;

        SetupBootstrapEnvInput {
            database_backend: self.settings.database_backend.clone(),
            database_url: self
                .settings
                .database_url
                .as_deref()
                .and_then(credential_free_database_url),
            libsql_path: self.settings.libsql_path.clone(),
            libsql_url: self.settings.libsql_url.clone(),
            // Master keys and channel/provider credentials never enter the
            // setup-owned dotenv file. Environment mode is operator-owned.
            secrets_master_key: None,
            allow_env_master_key: env_key_source && self.settings.secrets.allow_env_master_key,
            llm_backend: self.settings.llm_backend.clone(),
            llm_base_url: self.settings.openai_compatible_base_url.clone(),
            ollama_base_url: self.settings.ollama_base_url.clone(),
            onboard_completed: self.settings.onboard_completed,
            runtime_profile: match self.selected_profile.runtime_profile_env_value() {
                Some("remote") => Some(SetupRuntimeProfile::Remote),
                Some("pi-os-lite-64") => Some(SetupRuntimeProfile::PiOsLite64),
                _ => None,
            },
            channels: SetupBootstrapChannelInput {
                signal_http_url: channels.signal_http_url.clone(),
                signal_account: channels.signal_account.clone(),
                signal_allow_from: channels.signal_allow_from.clone(),
                signal_allow_from_groups: channels.signal_allow_from_groups.clone(),
                signal_dm_policy: channels.signal_dm_policy.clone(),
                signal_group_policy: channels.signal_group_policy.clone(),
                signal_group_allow_from: channels.signal_group_allow_from.clone(),
                http_enabled: channels.http_enabled,
                http_host: channels.http_host.clone(),
                http_port: channels.http_port,
                discord_enabled: channels.discord_enabled,
                discord_bot_token: None,
                discord_guild_id: channels.discord_guild_id.clone(),
                discord_allow_from: channels.discord_allow_from.clone(),
                slack_enabled: channels.slack_enabled,
                slack_bot_token: None,
                slack_app_token: None,
                slack_allow_from: channels.slack_allow_from.clone(),
                nostr_enabled: channels.nostr_enabled,
                nostr_relays: channels.nostr_relays.clone(),
                nostr_owner_pubkey: channels.nostr_owner_pubkey.clone(),
                nostr_social_dm_enabled: channels.nostr_social_dm_enabled,
                nostr_allow_from: channels.nostr_allow_from.clone(),
                gmail_enabled: channels.gmail_enabled,
                gmail_project_id: channels.gmail_project_id.clone(),
                gmail_subscription_id: channels.gmail_subscription_id.clone(),
                gmail_topic_id: channels.gmail_topic_id.clone(),
                gmail_allowed_senders: channels.gmail_allowed_senders.clone(),
                imessage_enabled: channels.imessage_enabled,
                imessage_allow_from: channels.imessage_allow_from.clone(),
                imessage_poll_interval: channels.imessage_poll_interval,
                apple_mail_enabled: channels.apple_mail_enabled,
                apple_mail_allow_from: channels.apple_mail_allow_from.clone(),
                apple_mail_poll_interval: channels.apple_mail_poll_interval,
                apple_mail_unread_only: channels.apple_mail_unread_only,
                apple_mail_mark_as_read: channels.apple_mail_mark_as_read,
                bluebubbles_enabled: channels.bluebubbles_enabled,
                bluebubbles_server_url: channels.bluebubbles_server_url.clone(),
                bluebubbles_password: None,
                bluebubbles_webhook_host: channels.bluebubbles_webhook_host.clone(),
                bluebubbles_webhook_port: channels.bluebubbles_webhook_port,
                bluebubbles_allow_from: channels.bluebubbles_allow_from.clone(),
                bluebubbles_send_read_receipts: channels.bluebubbles_send_read_receipts,
                gateway_enabled: channels.gateway_enabled,
                gateway_host: channels.gateway_host.clone(),
                gateway_port: channels.gateway_port,
                gateway_auth_token: None,
                cli_enabled: channels.cli_enabled,
            },
            providers: SetupBootstrapProviderInput {
                cheap_model: self.settings.providers.cheap_model.clone(),
            },
            web_ui: SetupBootstrapWebUiInput {
                skin: self.settings.webchat_skin.clone(),
                theme: self.settings.webchat_theme.clone(),
                accent_color: self.settings.webchat_accent_color.clone(),
                show_branding: self.settings.webchat_show_branding,
            },
            observability_backend: self.settings.observability_backend.clone(),
            agent: SetupBootstrapAgentInput {
                allow_local_tools: self.settings.agent.allow_local_tools,
                workspace_mode: self.settings.agent.workspace_mode.clone(),
            },
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
                use crate::db::SettingsStore as _;
                let store = crate::db::postgres::PgBackend::from_pool(pool.clone());
                match store.get_all_settings("default").await {
                    Ok(db_map) if !db_map.is_empty() => {
                        self.capture_plaintext_credentials_from_map(&db_map);
                        let safe_map = db_map
                            .into_iter()
                            .filter(|(key, _)| !super::transaction::credential_shaped_key(key))
                            .collect();
                        let existing = Settings::from_db_map(&safe_map);
                        self.baseline_settings = existing.clone();
                        self.settings.merge_from(&existing);
                        tracing::info!("Loaded existing non-secret settings from database");
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
                        self.capture_plaintext_credentials_from_map(&db_map);
                        let safe_map = db_map
                            .into_iter()
                            .filter(|(key, _)| !super::transaction::credential_shaped_key(key))
                            .collect();
                        let existing = Settings::from_db_map(&safe_map);
                        self.baseline_settings = existing.clone();
                        self.settings.merge_from(&existing);
                        tracing::info!("Loaded existing non-secret settings from database");
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

fn credential_free_database_url(value: &str) -> Option<String> {
    let parsed = url::Url::parse(value).ok()?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::OnboardingProfile;
    use super::*;

    fn value_for<'a>(plan: &'a thinclaw_app::SetupBootstrapEnvPlan, key: &str) -> Option<&'a str> {
        plan.variables()
            .iter()
            .find(|var| var.key == key)
            .map(|var| var.value.as_str())
    }

    #[test]
    fn bootstrap_env_input_maps_root_wizard_state_to_crate_plan() {
        let mut wizard = SetupWizard::new();
        wizard.selected_profile = OnboardingProfile::RemoteServer;
        wizard.settings.onboard_completed = true;
        wizard.settings.database_backend = Some("libsql".to_string());
        wizard.settings.libsql_path = Some("/tmp/thinclaw.db".to_string());
        wizard.settings.secrets_master_key_source = crate::settings::KeySource::Env;
        wizard.settings.secrets.allow_env_master_key = true;
        wizard.settings.channels.gateway_enabled = Some(true);
        wizard.settings.channels.gateway_host = Some("127.0.0.1".to_string());
        wizard.settings.channels.gateway_port = Some(3000);
        wizard.settings.channels.cli_enabled = Some(false);
        wizard.settings.agent.allow_local_tools = true;
        wizard.settings.agent.workspace_mode = Some("unrestricted".to_string());

        let input = wizard.bootstrap_env_input();
        let plan = setup_bootstrap_env_plan(&input);

        assert_eq!(value_for(&plan, "DATABASE_BACKEND"), Some("libsql"));
        assert_eq!(value_for(&plan, "LIBSQL_PATH"), Some("/tmp/thinclaw.db"));
        assert_eq!(value_for(&plan, "SECRETS_MASTER_KEY"), None);
        assert_eq!(value_for(&plan, "THINCLAW_ALLOW_ENV_MASTER_KEY"), Some("1"));
        assert_eq!(value_for(&plan, "ONBOARD_COMPLETED"), Some("true"));
        assert_eq!(value_for(&plan, "THINCLAW_RUNTIME_PROFILE"), Some("remote"));
        assert_eq!(value_for(&plan, "THINCLAW_HEADLESS"), Some("true"));
        assert_eq!(value_for(&plan, "GATEWAY_ENABLED"), Some("true"));
        assert_eq!(value_for(&plan, "GATEWAY_HOST"), Some("127.0.0.1"));
        assert_eq!(value_for(&plan, "GATEWAY_PORT"), Some("3000"));
        assert_eq!(value_for(&plan, "CLI_ENABLED"), Some("false"));
        assert_eq!(value_for(&plan, "ALLOW_LOCAL_TOOLS"), Some("true"));
        assert_eq!(value_for(&plan, "WORKSPACE_MODE"), Some("unrestricted"));
    }
}
