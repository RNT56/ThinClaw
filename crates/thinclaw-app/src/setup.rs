//! Root-independent setup/onboarding planning contracts.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupRuntimeProfile {
    Remote,
    PiOsLite64,
}

impl SetupRuntimeProfile {
    pub const fn env_value(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::PiOsLite64 => "pi-os-lite-64",
        }
    }

    pub const fn is_headless(self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapChannelInput {
    pub signal_http_url: Option<String>,
    pub signal_account: Option<String>,
    pub signal_allow_from: Option<String>,
    pub signal_allow_from_groups: Option<String>,
    pub signal_dm_policy: Option<String>,
    pub signal_group_policy: Option<String>,
    pub signal_group_allow_from: Option<String>,
    pub http_enabled: bool,
    pub http_host: Option<String>,
    pub http_port: Option<u16>,
    pub discord_enabled: bool,
    pub discord_bot_token: Option<String>,
    pub discord_guild_id: Option<String>,
    pub discord_allow_from: Option<String>,
    pub slack_enabled: bool,
    pub slack_bot_token: Option<String>,
    pub slack_app_token: Option<String>,
    pub slack_allow_from: Option<String>,
    pub nostr_enabled: bool,
    pub nostr_relays: Option<String>,
    pub nostr_owner_pubkey: Option<String>,
    pub nostr_social_dm_enabled: bool,
    pub nostr_allow_from: Option<String>,
    pub gmail_enabled: bool,
    pub gmail_project_id: Option<String>,
    pub gmail_subscription_id: Option<String>,
    pub gmail_topic_id: Option<String>,
    pub gmail_allowed_senders: Option<String>,
    pub imessage_enabled: bool,
    pub imessage_allow_from: Option<String>,
    pub imessage_poll_interval: Option<u64>,
    pub apple_mail_enabled: bool,
    pub apple_mail_allow_from: Option<String>,
    pub apple_mail_poll_interval: Option<u64>,
    pub apple_mail_unread_only: bool,
    pub apple_mail_mark_as_read: bool,
    pub bluebubbles_enabled: bool,
    pub bluebubbles_server_url: Option<String>,
    pub bluebubbles_password: Option<String>,
    pub bluebubbles_webhook_host: Option<String>,
    pub bluebubbles_webhook_port: Option<u16>,
    pub bluebubbles_allow_from: Option<String>,
    pub bluebubbles_send_read_receipts: Option<bool>,
    pub gateway_enabled: Option<bool>,
    pub gateway_host: Option<String>,
    pub gateway_port: Option<u16>,
    pub gateway_auth_token: Option<String>,
    pub cli_enabled: Option<bool>,
}

impl Default for SetupBootstrapChannelInput {
    fn default() -> Self {
        Self {
            signal_http_url: None,
            signal_account: None,
            signal_allow_from: None,
            signal_allow_from_groups: None,
            signal_dm_policy: None,
            signal_group_policy: None,
            signal_group_allow_from: None,
            http_enabled: false,
            http_host: None,
            http_port: None,
            discord_enabled: false,
            discord_bot_token: None,
            discord_guild_id: None,
            discord_allow_from: None,
            slack_enabled: false,
            slack_bot_token: None,
            slack_app_token: None,
            slack_allow_from: None,
            nostr_enabled: false,
            nostr_relays: None,
            nostr_owner_pubkey: None,
            nostr_social_dm_enabled: false,
            nostr_allow_from: None,
            gmail_enabled: false,
            gmail_project_id: None,
            gmail_subscription_id: None,
            gmail_topic_id: None,
            gmail_allowed_senders: None,
            imessage_enabled: false,
            imessage_allow_from: None,
            imessage_poll_interval: None,
            apple_mail_enabled: false,
            apple_mail_allow_from: None,
            apple_mail_poll_interval: None,
            apple_mail_unread_only: true,
            apple_mail_mark_as_read: true,
            bluebubbles_enabled: false,
            bluebubbles_server_url: None,
            bluebubbles_password: None,
            bluebubbles_webhook_host: None,
            bluebubbles_webhook_port: None,
            bluebubbles_allow_from: None,
            bluebubbles_send_read_receipts: None,
            gateway_enabled: None,
            gateway_host: None,
            gateway_port: None,
            gateway_auth_token: None,
            cli_enabled: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupBootstrapProviderInput {
    pub cheap_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapWebUiInput {
    pub skin: Option<String>,
    pub theme: String,
    pub accent_color: Option<String>,
    pub show_branding: bool,
}

impl Default for SetupBootstrapWebUiInput {
    fn default() -> Self {
        Self {
            skin: None,
            theme: "system".to_string(),
            accent_color: None,
            show_branding: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupBootstrapAgentInput {
    pub allow_local_tools: bool,
    pub workspace_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapEnvInput {
    pub database_backend: Option<String>,
    pub database_url: Option<String>,
    pub libsql_path: Option<String>,
    pub libsql_url: Option<String>,
    pub secrets_master_key: Option<String>,
    pub allow_env_master_key: bool,
    pub llm_backend: Option<String>,
    pub llm_base_url: Option<String>,
    pub ollama_base_url: Option<String>,
    pub onboard_completed: bool,
    pub runtime_profile: Option<SetupRuntimeProfile>,
    pub channels: SetupBootstrapChannelInput,
    pub providers: SetupBootstrapProviderInput,
    pub web_ui: SetupBootstrapWebUiInput,
    pub observability_backend: String,
    pub agent: SetupBootstrapAgentInput,
}

impl Default for SetupBootstrapEnvInput {
    fn default() -> Self {
        Self {
            database_backend: None,
            database_url: None,
            libsql_path: None,
            libsql_url: None,
            secrets_master_key: None,
            allow_env_master_key: false,
            llm_backend: None,
            llm_base_url: None,
            ollama_base_url: None,
            onboard_completed: false,
            runtime_profile: None,
            channels: SetupBootstrapChannelInput::default(),
            providers: SetupBootstrapProviderInput::default(),
            web_ui: SetupBootstrapWebUiInput::default(),
            observability_backend: "none".to_string(),
            agent: SetupBootstrapAgentInput::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupBootstrapEnvVar {
    pub key: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupBootstrapEnvPlan {
    variables: Vec<SetupBootstrapEnvVar>,
}

impl SetupBootstrapEnvPlan {
    pub fn variables(&self) -> &[SetupBootstrapEnvVar] {
        &self.variables
    }

    pub fn is_empty(&self) -> bool {
        self.variables.is_empty()
    }

    fn push(&mut self, key: &'static str, value: impl Into<String>) {
        self.variables.push(SetupBootstrapEnvVar {
            key,
            value: value.into(),
        });
    }

    fn push_optional(&mut self, key: &'static str, value: &Option<String>) {
        if let Some(value) = value {
            self.push(key, value.clone());
        }
    }

    fn push_non_empty_optional(&mut self, key: &'static str, value: &Option<String>) {
        if let Some(value) = value
            && !value.is_empty()
        {
            self.push(key, value.clone());
        }
    }
}

pub fn setup_bootstrap_env_plan(input: &SetupBootstrapEnvInput) -> SetupBootstrapEnvPlan {
    let mut plan = SetupBootstrapEnvPlan::default();

    plan.push_optional("DATABASE_BACKEND", &input.database_backend);
    plan.push_optional("DATABASE_URL", &input.database_url);
    plan.push_optional("LIBSQL_PATH", &input.libsql_path);
    plan.push_optional("LIBSQL_URL", &input.libsql_url);
    plan.push_optional("SECRETS_MASTER_KEY", &input.secrets_master_key);
    if input.allow_env_master_key {
        plan.push("THINCLAW_ALLOW_ENV_MASTER_KEY", "1");
    }

    plan.push_optional("LLM_BACKEND", &input.llm_backend);
    plan.push_optional("LLM_BASE_URL", &input.llm_base_url);
    plan.push_optional("OLLAMA_BASE_URL", &input.ollama_base_url);

    if input.onboard_completed {
        plan.push("ONBOARD_COMPLETED", "true");
    }
    if let Some(profile) = input.runtime_profile {
        plan.push("THINCLAW_RUNTIME_PROFILE", profile.env_value());
        if profile.is_headless() {
            plan.push("THINCLAW_HEADLESS", "true");
        }
    }

    let channels = &input.channels;
    plan.push_optional("SIGNAL_HTTP_URL", &channels.signal_http_url);
    plan.push_optional("SIGNAL_ACCOUNT", &channels.signal_account);
    plan.push_optional("SIGNAL_ALLOW_FROM", &channels.signal_allow_from);
    plan.push_non_empty_optional(
        "SIGNAL_ALLOW_FROM_GROUPS",
        &channels.signal_allow_from_groups,
    );
    plan.push_optional("SIGNAL_DM_POLICY", &channels.signal_dm_policy);
    plan.push_optional("SIGNAL_GROUP_POLICY", &channels.signal_group_policy);
    plan.push_non_empty_optional("SIGNAL_GROUP_ALLOW_FROM", &channels.signal_group_allow_from);

    if channels.http_enabled {
        plan.push("HTTP_ENABLED", "true");
        plan.push_optional("HTTP_HOST", &channels.http_host);
        if let Some(port) = channels.http_port {
            plan.push("HTTP_PORT", port.to_string());
        }
    }

    if channels.discord_enabled {
        plan.push("DISCORD_ENABLED", "true");
        plan.push_optional("DISCORD_BOT_TOKEN", &channels.discord_bot_token);
    }
    plan.push_optional("DISCORD_GUILD_ID", &channels.discord_guild_id);
    plan.push_optional("DISCORD_ALLOW_FROM", &channels.discord_allow_from);

    if channels.slack_enabled {
        plan.push("SLACK_ENABLED", "true");
        plan.push_optional("SLACK_BOT_TOKEN", &channels.slack_bot_token);
        plan.push_optional("SLACK_APP_TOKEN", &channels.slack_app_token);
    }
    plan.push_optional("SLACK_ALLOW_FROM", &channels.slack_allow_from);

    if channels.nostr_enabled {
        plan.push("NOSTR_ENABLED", "true");
    }
    plan.push_optional("NOSTR_RELAYS", &channels.nostr_relays);
    plan.push_optional("NOSTR_OWNER_PUBKEY", &channels.nostr_owner_pubkey);
    if channels.nostr_social_dm_enabled {
        plan.push("NOSTR_SOCIAL_DM_ENABLED", "true");
    }
    plan.push_optional("NOSTR_ALLOW_FROM", &channels.nostr_allow_from);

    if channels.gmail_enabled {
        plan.push("GMAIL_ENABLED", "true");
    }
    plan.push_optional("GMAIL_PROJECT_ID", &channels.gmail_project_id);
    plan.push_optional("GMAIL_SUBSCRIPTION_ID", &channels.gmail_subscription_id);
    plan.push_optional("GMAIL_TOPIC_ID", &channels.gmail_topic_id);
    plan.push_optional("GMAIL_ALLOWED_SENDERS", &channels.gmail_allowed_senders);

    if channels.imessage_enabled {
        plan.push("IMESSAGE_ENABLED", "true");
    }
    plan.push_optional("IMESSAGE_ALLOW_FROM", &channels.imessage_allow_from);
    if let Some(interval) = channels.imessage_poll_interval {
        plan.push("IMESSAGE_POLL_INTERVAL", interval.to_string());
    }

    if channels.apple_mail_enabled {
        plan.push("APPLE_MAIL_ENABLED", "true");
    }
    plan.push_optional("APPLE_MAIL_ALLOW_FROM", &channels.apple_mail_allow_from);
    if let Some(interval) = channels.apple_mail_poll_interval {
        plan.push("APPLE_MAIL_POLL_INTERVAL", interval.to_string());
    }
    if !channels.apple_mail_unread_only {
        plan.push("APPLE_MAIL_UNREAD_ONLY", "false");
    }
    if !channels.apple_mail_mark_as_read {
        plan.push("APPLE_MAIL_MARK_AS_READ", "false");
    }

    if channels.bluebubbles_enabled {
        plan.push("BLUEBUBBLES_ENABLED", "true");
    }
    plan.push_optional("BLUEBUBBLES_SERVER_URL", &channels.bluebubbles_server_url);
    plan.push_optional("BLUEBUBBLES_PASSWORD", &channels.bluebubbles_password);
    plan.push_optional(
        "BLUEBUBBLES_WEBHOOK_HOST",
        &channels.bluebubbles_webhook_host,
    );
    if let Some(port) = channels.bluebubbles_webhook_port {
        plan.push("BLUEBUBBLES_WEBHOOK_PORT", port.to_string());
    }
    plan.push_optional("BLUEBUBBLES_ALLOW_FROM", &channels.bluebubbles_allow_from);
    if let Some(send_receipts) = channels.bluebubbles_send_read_receipts {
        plan.push("BLUEBUBBLES_SEND_READ_RECEIPTS", send_receipts.to_string());
    }

    if let Some(enabled) = channels.gateway_enabled {
        plan.push("GATEWAY_ENABLED", enabled.to_string());
    }
    plan.push_optional("GATEWAY_HOST", &channels.gateway_host);
    if let Some(port) = channels.gateway_port {
        plan.push("GATEWAY_PORT", port.to_string());
    }
    plan.push_optional("GATEWAY_AUTH_TOKEN", &channels.gateway_auth_token);
    if let Some(enabled) = channels.cli_enabled {
        plan.push("CLI_ENABLED", enabled.to_string());
    }

    plan.push_optional("LLM_CHEAP_MODEL", &input.providers.cheap_model);

    plan.push_optional("WEBCHAT_SKIN", &input.web_ui.skin);
    if input.web_ui.theme != "system" {
        plan.push("WEBCHAT_THEME", input.web_ui.theme.clone());
    }
    plan.push_optional("WEBCHAT_ACCENT_COLOR", &input.web_ui.accent_color);
    if !input.web_ui.show_branding {
        plan.push("WEBCHAT_SHOW_BRANDING", "false");
    }

    if input.observability_backend != "none" {
        plan.push("OBSERVABILITY_BACKEND", input.observability_backend.clone());
    }

    if input.agent.allow_local_tools {
        plan.push("ALLOW_LOCAL_TOOLS", "true");
    }
    plan.push_optional("WORKSPACE_MODE", &input.agent.workspace_mode);

    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value_for<'a>(plan: &'a SetupBootstrapEnvPlan, key: &str) -> Option<&'a str> {
        plan.variables()
            .iter()
            .find(|var| var.key == key)
            .map(|var| var.value.as_str())
    }

    #[test]
    fn default_input_has_empty_plan() {
        let plan = setup_bootstrap_env_plan(&SetupBootstrapEnvInput::default());

        assert!(plan.is_empty());
    }

    #[test]
    fn runtime_profile_writes_headless_markers_after_onboard_marker() {
        let input = SetupBootstrapEnvInput {
            onboard_completed: true,
            runtime_profile: Some(SetupRuntimeProfile::PiOsLite64),
            ..SetupBootstrapEnvInput::default()
        };

        let plan = setup_bootstrap_env_plan(&input);

        assert_eq!(value_for(&plan, "ONBOARD_COMPLETED"), Some("true"));
        assert_eq!(
            value_for(&plan, "THINCLAW_RUNTIME_PROFILE"),
            Some("pi-os-lite-64")
        );
        assert_eq!(value_for(&plan, "THINCLAW_HEADLESS"), Some("true"));
        let keys: Vec<&str> = plan.variables().iter().map(|var| var.key).collect();
        assert_eq!(
            keys,
            vec![
                "ONBOARD_COMPLETED",
                "THINCLAW_RUNTIME_PROFILE",
                "THINCLAW_HEADLESS"
            ]
        );
    }

    #[test]
    fn channel_mapping_preserves_existing_enabled_and_false_values() {
        let input = SetupBootstrapEnvInput {
            channels: SetupBootstrapChannelInput {
                signal_allow_from_groups: Some(String::new()),
                signal_group_allow_from: Some("group-a".to_string()),
                http_enabled: true,
                http_host: Some("0.0.0.0".to_string()),
                http_port: Some(8080),
                apple_mail_unread_only: false,
                apple_mail_mark_as_read: false,
                gateway_enabled: Some(false),
                cli_enabled: Some(false),
                ..SetupBootstrapChannelInput::default()
            },
            web_ui: SetupBootstrapWebUiInput {
                show_branding: false,
                ..SetupBootstrapWebUiInput::default()
            },
            ..SetupBootstrapEnvInput::default()
        };

        let plan = setup_bootstrap_env_plan(&input);

        assert_eq!(value_for(&plan, "HTTP_ENABLED"), Some("true"));
        assert_eq!(value_for(&plan, "HTTP_HOST"), Some("0.0.0.0"));
        assert_eq!(value_for(&plan, "HTTP_PORT"), Some("8080"));
        assert_eq!(value_for(&plan, "SIGNAL_ALLOW_FROM_GROUPS"), None);
        assert_eq!(value_for(&plan, "SIGNAL_GROUP_ALLOW_FROM"), Some("group-a"));
        assert_eq!(value_for(&plan, "APPLE_MAIL_UNREAD_ONLY"), Some("false"));
        assert_eq!(value_for(&plan, "APPLE_MAIL_MARK_AS_READ"), Some("false"));
        assert_eq!(value_for(&plan, "GATEWAY_ENABLED"), Some("false"));
        assert_eq!(value_for(&plan, "CLI_ENABLED"), Some("false"));
        assert_eq!(value_for(&plan, "WEBCHAT_SHOW_BRANDING"), Some("false"));
        assert_eq!(value_for(&plan, "WEBCHAT_THEME"), None);
    }
}
