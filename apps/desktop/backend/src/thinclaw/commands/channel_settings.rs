//! Revisioned, redacted Slack/Telegram settings owned by the Channel Center.
//!
//! The legacy engine JSON remains a migration input for one release, while the
//! current settings database and OS credential store are the authoritative
//! targets. A compensating transaction prevents partial Keychain/DB/file
//! commits, and effective remote mode is checked before any local state opens.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use tauri::State;

use super::ThinClawManager;
use super::rpc_config::local_settings_context;
use crate::secret_store::SecretStore;
use crate::thinclaw::bridge::{BridgeError, RouteMode};
use crate::thinclaw::config::{ThinClawConfig, ThinClawEngineConfig};
use crate::thinclaw::runtime_bridge::ThinClawRuntimeState;

const SETTINGS_USER: &str = "local_user";
const MAX_SECRET_BYTES: usize = 16 * 1024;
const SLACK_ENABLED: &str = "channels.slack_enabled";
const SLACK_DM_POLICY: &str = "channels.slack_dm_policy";
const TELEGRAM_ENABLED: &str = "channels.telegram_enabled";
const TELEGRAM_DM_POLICY: &str = "channels.telegram_dm_policy";
const TELEGRAM_GROUPS_ENABLED: &str = "channels.telegram_groups_enabled";
const SLACK_BOT_SECRET: &str = "slack_bot_token";
const SLACK_APP_SECRET: &str = "slack_app_token";
const SLACK_SIGNING_SECRET: &str = "slack_signing_secret";
const TELEGRAM_BOT_SECRET: &str = "telegram_bot_token";

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ChannelSecretMutation {
    Preserve,
    Replace { value: String },
    Clear,
}

impl Default for ChannelSecretMutation {
    fn default() -> Self {
        Self::Preserve
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct SlackChannelSettingsUpdate {
    pub expected_revision: String,
    pub enabled: Option<bool>,
    pub dm_policy: Option<String>,
    #[serde(default)]
    pub bot_token: ChannelSecretMutation,
    #[serde(default)]
    pub app_token: ChannelSecretMutation,
    #[serde(default)]
    pub signing_secret: ChannelSecretMutation,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct TelegramChannelSettingsUpdate {
    pub expected_revision: String,
    pub enabled: Option<bool>,
    pub dm_policy: Option<String>,
    pub groups_enabled: Option<bool>,
    #[serde(default)]
    pub bot_token: ChannelSecretMutation,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct SlackChannelSettingsSnapshot {
    pub enabled: bool,
    pub dm_policy: String,
    pub bot_token_configured: bool,
    pub bot_token_migration_required: bool,
    pub app_token_configured: bool,
    pub app_token_migration_required: bool,
    pub signing_secret_configured: bool,
    pub active: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct TelegramChannelSettingsSnapshot {
    pub enabled: bool,
    pub dm_policy: String,
    pub groups_enabled: bool,
    pub require_mention: bool,
    pub bot_token_configured: bool,
    pub bot_token_migration_required: bool,
    pub active: bool,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct ChannelSettingsSnapshot {
    pub available: bool,
    pub editable: bool,
    pub source: String,
    pub revision: String,
    pub reason: Option<String>,
    pub slack: SlackChannelSettingsSnapshot,
    pub telegram: TelegramChannelSettingsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct ChannelSettingsMutationResponse {
    pub snapshot: ChannelSettingsSnapshot,
    pub persisted: bool,
    pub applied: bool,
    pub restart_required: bool,
    pub note: String,
}

#[derive(Clone)]
struct LocalChannelState {
    snapshot: ChannelSettingsSnapshot,
    engine: ThinClawEngineConfig,
    engine_existed: bool,
    settings: HashMap<&'static str, Option<serde_json::Value>>,
    secrets: HashMap<&'static str, Option<String>>,
    config: ThinClawConfig,
    store: Arc<dyn thinclaw_core::db::Database>,
    agent: Option<Arc<thinclaw_core::agent::Agent>>,
}

fn configured(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.trim().is_empty())
}

fn setting_bool(value: Option<&serde_json::Value>, fallback: bool) -> bool {
    value
        .and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_str().map(|raw| matches!(raw, "true" | "1")))
        })
        .unwrap_or(fallback)
}

fn setting_string(value: Option<&serde_json::Value>, fallback: &str) -> String {
    value
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn channel_status(enabled: bool, configured: bool, active: bool) -> String {
    if active {
        "running".to_string()
    } else if !enabled {
        "disabled".to_string()
    } else if !configured {
        "missing_credentials".to_string()
    } else {
        "configured_not_running".to_string()
    }
}

fn content_revision(value: &serde_json::Value) -> String {
    let encoded = serde_json::to_vec(value).unwrap_or_default();
    // The revision crosses the renderer boundary. Salt it per process so the
    // digest cannot be used as a stable oracle for credential equality or for
    // offline guesses, while retaining compare-and-swap semantics for the
    // lifetime of this backend process.
    static REVISION_SALT: OnceLock<[u8; 32]> = OnceLock::new();
    let salt = REVISION_SALT.get_or_init(|| {
        let mut value = [0_u8; 32];
        OsRng.fill_bytes(&mut value);
        value
    });
    let mut digest = Sha256::new();
    digest.update(salt);
    digest.update(encoded);
    format!("sha256:{}", hex::encode(digest.finalize()))
}

fn local_content_revision(
    engine_existed: bool,
    engine_channels: &crate::thinclaw::config::ChannelsConfig,
    settings: &HashMap<&'static str, Option<serde_json::Value>>,
    secrets: &HashMap<&'static str, Option<String>>,
) -> String {
    content_revision(&serde_json::json!({
        "engine_existed": engine_existed,
        "engine_channels": engine_channels,
        "settings": {
            "channels.slack_enabled": settings.get(SLACK_ENABLED).and_then(Option::as_ref),
            "channels.slack_dm_policy": settings.get(SLACK_DM_POLICY).and_then(Option::as_ref),
            "channels.telegram_enabled": settings.get(TELEGRAM_ENABLED).and_then(Option::as_ref),
            "channels.telegram_dm_policy": settings.get(TELEGRAM_DM_POLICY).and_then(Option::as_ref),
            "channels.telegram_groups_enabled": settings
                .get(TELEGRAM_GROUPS_ENABLED)
                .and_then(Option::as_ref),
        },
        "credentials": {
            "slack_bot_token": secrets.get(SLACK_BOT_SECRET).and_then(Option::as_ref),
            "slack_app_token": secrets.get(SLACK_APP_SECRET).and_then(Option::as_ref),
            "slack_signing_secret": secrets
                .get(SLACK_SIGNING_SECRET)
                .and_then(Option::as_ref),
            "telegram_bot_token": secrets
                .get(TELEGRAM_BOT_SECRET)
                .and_then(Option::as_ref),
        },
    }))
}

fn invalid_input(message: impl Into<String>, field: &str) -> BridgeError {
    BridgeError::InvalidInput {
        message: message.into(),
        field: Some(field.to_string()),
    }
}

fn validate_policy(value: String, field: &str) -> Result<String, BridgeError> {
    let normalized = value.trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "pairing" | "allowlist" | "open") {
        Ok(normalized)
    } else {
        Err(invalid_input(
            "policy must be pairing, allowlist, or open",
            field,
        ))
    }
}

fn resolve_secret(
    mutation: &ChannelSecretMutation,
    current: Option<String>,
    legacy: Option<String>,
    field: &str,
) -> Result<Option<String>, BridgeError> {
    match mutation {
        ChannelSecretMutation::Preserve => {
            Ok(current.or_else(|| legacy.filter(|value| !value.trim().is_empty())))
        }
        ChannelSecretMutation::Clear => Ok(None),
        ChannelSecretMutation::Replace { value } => {
            let value = value.trim();
            if value.is_empty() {
                return Err(invalid_input(
                    "blank credentials cannot replace or clear a stored credential",
                    field,
                ));
            }
            if value.len() > MAX_SECRET_BYTES || value.chars().any(char::is_control) {
                return Err(invalid_input("credential is malformed or oversized", field));
            }
            Ok(Some(value.to_string()))
        }
    }
}

async fn load_local_state(
    manager: &ThinClawManager,
    runtime: &ThinClawRuntimeState,
    secret_store: &SecretStore,
) -> Result<LocalChannelState, BridgeError> {
    let config = if let Some(config) = manager.get_config().await {
        config
    } else {
        manager.init_config().await?
    };
    let (engine, engine_existed) = match config.load_config() {
        Ok(engine) => (engine, true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (config.generate_config(None, None, None), false)
        }
        Err(error) => {
            return Err(BridgeError::Runtime {
                message: format!("failed to load channel compatibility config: {error}"),
            });
        }
    };

    let context = local_settings_context(runtime).await?;
    let mut settings = HashMap::new();
    for key in [
        SLACK_ENABLED,
        SLACK_DM_POLICY,
        TELEGRAM_ENABLED,
        TELEGRAM_DM_POLICY,
        TELEGRAM_GROUPS_ENABLED,
    ] {
        let value = context
            .store
            .get_setting(SETTINGS_USER, key)
            .await
            .map_err(|error| BridgeError::Runtime {
                message: format!("failed to read channel setting '{key}': {error}"),
            })?;
        settings.insert(key, value);
    }

    let secrets = HashMap::from([
        (SLACK_BOT_SECRET, secret_store.get(SLACK_BOT_SECRET)),
        (SLACK_APP_SECRET, secret_store.get(SLACK_APP_SECRET)),
        (SLACK_SIGNING_SECRET, secret_store.get(SLACK_SIGNING_SECRET)),
        (TELEGRAM_BOT_SECRET, secret_store.get(TELEGRAM_BOT_SECRET)),
    ]);
    let live_statuses = if let Some(agent) = context.agent.as_ref() {
        let registered = agent.channels().channel_names().await;
        agent
            .channels()
            .status_entries()
            .await
            .into_iter()
            .filter(|entry| registered.iter().any(|name| name == &entry.name))
            .map(|entry| (entry.name, entry.state.label().to_string()))
            .collect::<HashMap<_, _>>()
    } else {
        HashMap::new()
    };
    let slack_live_status = live_statuses.get("slack").cloned();
    let telegram_live_status = live_statuses.get("telegram").cloned();
    let slack_active = slack_live_status.as_deref() == Some("running");
    let telegram_active = telegram_live_status.as_deref() == Some("running");

    let slack_enabled = setting_bool(
        settings.get(SLACK_ENABLED).and_then(Option::as_ref),
        engine.channels.slack.enabled,
    );
    let slack_dm_policy = setting_string(
        settings.get(SLACK_DM_POLICY).and_then(Option::as_ref),
        &engine.channels.slack.dm_policy,
    );
    let slack_bot_configured = configured(
        secrets
            .get(SLACK_BOT_SECRET)
            .and_then(Option::as_deref)
            .or(engine.channels.slack.bot_token.as_deref()),
    );
    let slack_app_configured = configured(
        secrets
            .get(SLACK_APP_SECRET)
            .and_then(Option::as_deref)
            .or(engine.channels.slack.app_token.as_deref()),
    );
    let slack_signing_configured =
        configured(secrets.get(SLACK_SIGNING_SECRET).and_then(Option::as_deref));

    let telegram_enabled = setting_bool(
        settings.get(TELEGRAM_ENABLED).and_then(Option::as_ref),
        engine.channels.telegram.enabled,
    );
    let telegram_dm_policy = setting_string(
        settings.get(TELEGRAM_DM_POLICY).and_then(Option::as_ref),
        &engine.channels.telegram.dm_policy,
    );
    let telegram_groups_enabled = setting_bool(
        settings
            .get(TELEGRAM_GROUPS_ENABLED)
            .and_then(Option::as_ref),
        engine.channels.telegram.groups.enabled(),
    );
    let telegram_bot_configured = configured(
        secrets
            .get(TELEGRAM_BOT_SECRET)
            .and_then(Option::as_deref)
            .or(engine.channels.telegram.bot_token.as_deref()),
    );
    let require_mention = engine
        .channels
        .telegram
        .groups
        .wildcard
        .as_ref()
        .map(|group| group.require_mention)
        .or_else(|| {
            engine
                .channels
                .telegram
                .groups
                .extra
                .get("_disabledWildcard")
                .and_then(|value| value.get("requireMention"))
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(true);

    // Hash only the channel-owned compatibility document and spell every
    // database/credential key out explicitly. `generate_config()` contains a
    // fresh metadata timestamp when the legacy file is absent, while HashMap
    // serialization order is intentionally unspecified; either one would make
    // two unchanged reads produce different revisions and reject every first
    // save as a false conflict.
    let revision = local_content_revision(engine_existed, &engine.channels, &settings, &secrets);
    let snapshot = ChannelSettingsSnapshot {
        available: true,
        editable: true,
        source: "local".to_string(),
        revision,
        reason: None,
        slack: SlackChannelSettingsSnapshot {
            enabled: slack_enabled,
            dm_policy: slack_dm_policy,
            bot_token_configured: slack_bot_configured,
            bot_token_migration_required: configured(engine.channels.slack.bot_token.as_deref()),
            app_token_configured: slack_app_configured,
            app_token_migration_required: configured(engine.channels.slack.app_token.as_deref()),
            signing_secret_configured: slack_signing_configured,
            active: slack_active,
            status: slack_live_status.unwrap_or_else(|| {
                channel_status(
                    slack_enabled,
                    slack_bot_configured && slack_signing_configured,
                    slack_active,
                )
            }),
        },
        telegram: TelegramChannelSettingsSnapshot {
            enabled: telegram_enabled,
            dm_policy: telegram_dm_policy,
            groups_enabled: telegram_groups_enabled,
            require_mention,
            bot_token_configured: telegram_bot_configured,
            bot_token_migration_required: configured(engine.channels.telegram.bot_token.as_deref()),
            active: telegram_active,
            status: telegram_live_status.unwrap_or_else(|| {
                channel_status(telegram_enabled, telegram_bot_configured, telegram_active)
            }),
        },
    };

    Ok(LocalChannelState {
        snapshot,
        engine,
        engine_existed,
        settings,
        secrets,
        config,
        store: context.store,
        agent: context.agent,
    })
}

fn remote_setting_map(value: &serde_json::Value) -> HashMap<String, serde_json::Value> {
    value
        .get("settings")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            Some((
                row.get("key")?.as_str()?.to_string(),
                row.get("value")?.clone(),
            ))
        })
        .collect()
}

fn remote_setup_flags(
    status: &serde_json::Value,
    channel: &str,
    required: &[&str],
) -> (bool, HashMap<String, bool>) {
    let setup = status
        .get("channel_setup")
        .and_then(|value| value.get(channel));
    let enabled = setup
        .and_then(|value| value.get("enabled"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let missing = setup
        .and_then(|value| value.get("missing_fields"))
        .and_then(serde_json::Value::as_array);
    let all_configured = setup
        .and_then(|value| value.get("configured"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| {
            missing.is_some_and(|missing| {
                required
                    .iter()
                    .all(|field| !missing.iter().any(|value| value.as_str() == Some(*field)))
            })
        });
    let configured = required
        .iter()
        .map(|field| {
            let present = missing.map_or(all_configured, |missing| {
                !missing
                    .iter()
                    .any(|value| value.as_str() == Some(*field))
            });
            ((*field).to_string(), present)
        })
        .collect();
    (enabled, configured)
}

async fn load_remote_snapshot(
    runtime: &ThinClawRuntimeState,
) -> Result<ChannelSettingsSnapshot, BridgeError> {
    let proxy = runtime
        .remote_proxy()
        .await
        .ok_or_else(|| BridgeError::Runtime {
            message: "remote channel snapshot requested without an effective remote gateway".into(),
        })?;
    let (status, settings_response) = tokio::try_join!(proxy.get_status(), proxy.list_settings())?;
    let settings = remote_setting_map(&settings_response);
    let (slack_enabled_from_status, slack_fields) =
        remote_setup_flags(&status, "slack", &["bot_token", "signing_secret"]);
    let (telegram_enabled_from_status, telegram_fields) =
        remote_setup_flags(&status, "telegram", &["bot_token"]);
    let slack_bot_configured = slack_fields.get("bot_token").copied().unwrap_or(false);
    let slack_signing_configured = slack_fields
        .get("signing_secret")
        .copied()
        .unwrap_or(false);
    let slack_configured = slack_bot_configured && slack_signing_configured;
    let telegram_configured = telegram_fields
        .get("bot_token")
        .copied()
        .unwrap_or(false);
    let slack_enabled = setting_bool(settings.get(SLACK_ENABLED), slack_enabled_from_status);
    let telegram_enabled =
        setting_bool(settings.get(TELEGRAM_ENABLED), telegram_enabled_from_status);
    // The current gateway contract exposes configuration readiness, not the
    // live channel-manager membership. Do not turn readiness into a false
    // "running" claim in a remote renderer.
    let slack_active = false;
    let telegram_active = false;

    Ok(ChannelSettingsSnapshot {
        available: true,
        editable: false,
        source: "remote".to_string(),
        revision: content_revision(&serde_json::json!({
            "status": status,
            "settings": settings_response,
        })),
        reason: Some(
            "This remote gateway does not publish the revisioned channel-secret mutation capability. Configure these channels on the gateway host."
                .to_string(),
        ),
        slack: SlackChannelSettingsSnapshot {
            enabled: slack_enabled,
            dm_policy: setting_string(settings.get(SLACK_DM_POLICY), "pairing"),
            bot_token_configured: slack_bot_configured,
            bot_token_migration_required: false,
            app_token_configured: false,
            app_token_migration_required: false,
            signing_secret_configured: slack_signing_configured,
            active: slack_active,
            status: channel_status(slack_enabled, slack_configured, slack_active),
        },
        telegram: TelegramChannelSettingsSnapshot {
            enabled: telegram_enabled,
            dm_policy: setting_string(settings.get(TELEGRAM_DM_POLICY), "pairing"),
            groups_enabled: setting_bool(settings.get(TELEGRAM_GROUPS_ENABLED), true),
            require_mention: true,
            bot_token_configured: telegram_configured,
            bot_token_migration_required: false,
            active: telegram_active,
            status: channel_status(telegram_enabled, telegram_configured, telegram_active),
        },
    })
}

/// Lightweight compatibility projection for the long-lived global status
/// payload. Enablement is operator intent from the effective settings target,
/// never inferred from unrelated custom-secret grants.
pub(super) async fn effective_channel_enabled(
    manager: &ThinClawManager,
    runtime: &ThinClawRuntimeState,
) -> (bool, bool) {
    if let Some(proxy) = runtime.remote_proxy().await {
        return match proxy.get_status().await {
            Ok(status) => {
                let slack = status
                    .get("channel_setup")
                    .and_then(|value| value.get("slack"))
                    .and_then(|value| value.get("enabled"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let telegram = status
                    .get("channel_setup")
                    .and_then(|value| value.get("telegram"))
                    .and_then(|value| value.get("enabled"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                (slack, telegram)
            }
            Err(error) => {
                tracing::warn!(%error, "Unable to resolve remote channel enablement");
                (false, false)
            }
        };
    }

    let engine = manager
        .get_config()
        .await
        .and_then(|config| config.load_config().ok());
    let fallback = engine
        .as_ref()
        .map(|engine| {
            (
                engine.channels.slack.enabled,
                engine.channels.telegram.enabled,
            )
        })
        .unwrap_or((false, false));
    let Ok(agent) = runtime.agent().await else {
        return fallback;
    };
    let Some(store) = agent.store() else {
        return fallback;
    };
    let (slack, telegram) = tokio::join!(
        store.get_setting(SETTINGS_USER, SLACK_ENABLED),
        store.get_setting(SETTINGS_USER, TELEGRAM_ENABLED),
    );
    (
        slack
            .ok()
            .flatten()
            .as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(fallback.0),
        telegram
            .ok()
            .flatten()
            .as_ref()
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(fallback.1),
    )
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_channel_settings_snapshot(
    manager: State<'_, ThinClawManager>,
    secret_store: State<'_, SecretStore>,
    runtime: State<'_, ThinClawRuntimeState>,
) -> Result<ChannelSettingsSnapshot, BridgeError> {
    if runtime.remote_proxy().await.is_some() {
        return load_remote_snapshot(&runtime).await;
    }
    let _guard = manager.channel_settings_lock.lock().await;
    Ok(load_local_state(&manager, &runtime, &secret_store)
        .await?
        .snapshot)
}

#[derive(Debug, Clone)]
struct SecretPlan {
    name: &'static str,
    before: Option<String>,
    after: Option<String>,
}

#[async_trait]
trait ChannelPersistence {
    fn apply_secrets(&mut self) -> Result<(), String>;
    async fn apply_settings(&mut self) -> Result<(), String>;
    fn verify_secrets(&mut self) -> Result<(), String>;
    fn apply_engine(&mut self) -> Result<(), String>;
    async fn rollback_settings(&mut self) -> Result<(), String>;
    fn rollback_secrets(&mut self) -> Result<(), String>;
}

async fn commit_persistence(transaction: &mut impl ChannelPersistence) -> Result<(), String> {
    transaction.apply_secrets()?;
    if let Err(error) = transaction.apply_settings().await {
        let rollback = transaction.rollback_secrets();
        return Err(match rollback {
            Ok(()) => error,
            Err(rollback) => format!("{error}; credential rollback failed: {rollback}"),
        });
    }
    // Re-prove the secure binding after the database commit. This catches a
    // credential-store failure/race in the opposite ordering direction and
    // compensates the already-committed settings before legacy plaintext is
    // removed.
    if let Err(error) = transaction.verify_secrets() {
        let settings_rollback = transaction.rollback_settings().await;
        let secret_rollback = transaction.rollback_secrets();
        let mut message = error;
        if let Err(rollback) = settings_rollback {
            message.push_str(&format!("; settings rollback failed: {rollback}"));
        }
        if let Err(rollback) = secret_rollback {
            message.push_str(&format!("; credential rollback failed: {rollback}"));
        }
        return Err(message);
    }
    if let Err(error) = transaction.apply_engine() {
        let settings_rollback = transaction.rollback_settings().await;
        let secret_rollback = transaction.rollback_secrets();
        let mut message = error;
        if let Err(rollback) = settings_rollback {
            message.push_str(&format!("; settings rollback failed: {rollback}"));
        }
        if let Err(rollback) = secret_rollback {
            message.push_str(&format!("; credential rollback failed: {rollback}"));
        }
        return Err(message);
    }
    Ok(())
}

struct ProductionPersistence<'a> {
    secret_store: &'a SecretStore,
    store: &'a Arc<dyn thinclaw_core::db::Database>,
    settings_patch: HashMap<String, serde_json::Value>,
    settings_before: HashMap<String, Option<serde_json::Value>>,
    secrets: Vec<SecretPlan>,
    config: &'a ThinClawConfig,
    engine_before: &'a ThinClawEngineConfig,
    engine_after: &'a ThinClawEngineConfig,
    engine_existed: bool,
}

impl ProductionPersistence<'_> {
    fn restore_secret_plans(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        for plan in self.secrets.iter().rev() {
            if let Err(error) = self.secret_store.set(plan.name, plan.before.as_deref()) {
                errors.push(format!("{}: {error}", plan.name));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join(", "))
        }
    }
}

#[async_trait]
impl ChannelPersistence for ProductionPersistence<'_> {
    fn apply_secrets(&mut self) -> Result<(), String> {
        for (index, plan) in self.secrets.iter().enumerate() {
            let result = self
                .secret_store
                .set(plan.name, plan.after.as_deref())
                .and_then(|()| {
                    (self.secret_store.get(plan.name) == plan.after)
                        .then_some(())
                        .ok_or_else(|| {
                            format!("credential '{}' failed post-write verification", plan.name)
                        })
                });
            if let Err(error) = result {
                let mut rollback_errors = Vec::new();
                for applied in self.secrets[..=index].iter().rev() {
                    if let Err(rollback) = self
                        .secret_store
                        .set(applied.name, applied.before.as_deref())
                    {
                        rollback_errors.push(format!("{}: {rollback}", applied.name));
                    }
                }
                if rollback_errors.is_empty() {
                    return Err(error);
                }
                return Err(format!(
                    "{error}; partial credential rollback failed: {}",
                    rollback_errors.join(", ")
                ));
            }
        }
        Ok(())
    }

    async fn apply_settings(&mut self) -> Result<(), String> {
        if self.settings_patch.is_empty() {
            return Ok(());
        }
        self.store
            .set_all_settings(SETTINGS_USER, &self.settings_patch)
            .await
            .map_err(|error| format!("failed to atomically persist channel settings: {error}"))
    }

    fn verify_secrets(&mut self) -> Result<(), String> {
        self.secrets.iter().try_for_each(|plan| {
            (self.secret_store.get(plan.name) == plan.after)
                .then_some(())
                .ok_or_else(|| {
                    format!(
                        "credential '{}' changed before the settings transaction completed",
                        plan.name
                    )
                })
        })
    }

    fn apply_engine(&mut self) -> Result<(), String> {
        if let Err(error) = self.config.write_config(self.engine_after, None) {
            let rollback = if self.engine_existed {
                self.config.write_config(self.engine_before, None)
            } else {
                match std::fs::remove_file(self.config.config_path()) {
                    Ok(()) => Ok(()),
                    Err(remove) if remove.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(remove) => Err(remove),
                }
            };
            return Err(match rollback {
                Ok(()) => format!("failed to persist channel compatibility config: {error}"),
                Err(rollback) => format!(
                    "failed to persist channel compatibility config: {error}; file rollback failed: {rollback}"
                ),
            });
        }
        Ok(())
    }

    async fn rollback_settings(&mut self) -> Result<(), String> {
        let mut restore = HashMap::new();
        let mut absent = Vec::new();
        for (key, value) in &self.settings_before {
            if let Some(value) = value {
                restore.insert(key.clone(), value.clone());
            } else {
                absent.push(key.clone());
            }
        }
        if !restore.is_empty() {
            self.store
                .set_all_settings(SETTINGS_USER, &restore)
                .await
                .map_err(|error| error.to_string())?;
        }
        for key in absent {
            self.store
                .delete_setting(SETTINGS_USER, &key)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn rollback_secrets(&mut self) -> Result<(), String> {
        self.restore_secret_plans()
    }
}

fn secret_plan(
    name: &'static str,
    before: Option<String>,
    after: Option<String>,
    prove_legacy_migration: bool,
) -> Option<SecretPlan> {
    (before != after || prove_legacy_migration).then_some(SecretPlan {
        name,
        before,
        after,
    })
}

async fn apply_runtime(
    agent: Option<&Arc<thinclaw_core::agent::Agent>>,
    channel: &str,
    enabled: bool,
    configured: bool,
    credentials_changed: bool,
    updates: HashMap<String, serde_json::Value>,
) -> (bool, bool, String) {
    let Some(agent) = agent else {
        return (
            false,
            false,
            "Settings are saved and will be evaluated when the local runtime starts.".to_string(),
        );
    };

    let active = agent
        .channels()
        .channel_names()
        .await
        .iter()
        .any(|name| name == channel);

    if !enabled {
        if !active {
            return (
                true,
                false,
                format!("{channel} is disabled and was not running."),
            );
        }
        return match stop_managed_channel(agent, channel).await {
            Ok(()) => (true, false, format!("{channel} was disabled and stopped.")),
            Err(error) => (
                false,
                true,
                format!("Settings were saved, but stopping {channel} failed: {error}"),
            ),
        };
    }
    if !configured {
        if active {
            return match stop_managed_channel(agent, channel).await {
                Ok(()) => (
                    true,
                    false,
                    format!(
                        "{channel} remains enabled but was stopped because required credentials are missing."
                    ),
                ),
                Err(error) => (
                    false,
                    true,
                    format!(
                        "{channel} is missing required credentials, and stopping its active instance failed: {error}"
                    ),
                ),
            };
        }
        return (
            false,
            false,
            format!("{channel} is enabled but required credentials are still missing."),
        );
    }

    if active {
        if credentials_changed {
            return match activate_managed_channel(agent, channel, updates).await {
                Ok(()) => (
                    true,
                    false,
                    format!(
                        "{channel} settings were saved and the active channel reloaded its secure credentials."
                    ),
                ),
                Err(error) => {
                    let stopped = stop_managed_channel(agent, channel).await;
                    (
                        false,
                        true,
                        format!(
                            "Settings were saved, but {channel} could not reload secure credentials ({error}). Its previous instance was stopped to avoid running stale credentials{}.",
                            stopped
                                .err()
                                .map(|stop| format!("; stopping also failed: {stop}"))
                                .unwrap_or_default(),
                        ),
                    )
                }
            };
        }

        let apply_result = agent
            .channels()
            .update_channel_runtime_config(channel, updates)
            .await
            .map_err(|error| error.to_string());
        let restart_result = match apply_result {
            Ok(()) => agent
                .channels()
                .restart_channel(channel)
                .await
                .map_err(|error| error.to_string()),
            Err(error) => Err(error),
        };
        return match restart_result {
            Ok(()) => (
                true,
                false,
                format!("{channel} settings were saved and the active channel restarted."),
            ),
            Err(error) => {
                let stopped = stop_managed_channel(agent, channel).await;
                (
                    false,
                    true,
                    format!(
                        "Settings were saved, but {channel} could not apply its new admission policy ({error}). The stale instance was stopped{}.",
                        stopped
                            .err()
                            .map(|stop| format!("; stopping also failed: {stop}"))
                            .unwrap_or_default(),
                    ),
                )
            }
        };
    }

    match activate_managed_channel(agent, channel, updates).await {
        Ok(()) => (
            true,
            false,
            format!("{channel} settings were saved and the channel was activated."),
        ),
        Err(error) => (
            false,
            true,
            format!(
                "{channel} settings were saved, but the installed channel could not be activated in place ({error}). Restart the local runtime after installing the channel package."
            ),
        ),
    }
}

async fn activate_managed_channel(
    agent: &Arc<thinclaw_core::agent::Agent>,
    channel: &str,
    updates: HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    let manager = agent
        .extension_manager()
        .ok_or_else(|| "extension manager is unavailable".to_string())?;
    manager
        .activate(channel)
        .await
        .map_err(|error| error.to_string())?;
    if let Err(error) = agent
        .channels()
        .update_channel_runtime_config(channel, updates)
        .await
    {
        let _ = stop_managed_channel(agent, channel).await;
        return Err(format!("runtime policy update failed: {error}"));
    }
    if let Err(error) = agent.channels().restart_channel(channel).await {
        let _ = stop_managed_channel(agent, channel).await;
        return Err(format!("runtime policy restart failed: {error}"));
    }
    Ok(())
}

async fn stop_managed_channel(
    agent: &Arc<thinclaw_core::agent::Agent>,
    channel: &str,
) -> Result<(), String> {
    let metadata_result = if let Some(manager) = agent.extension_manager() {
        manager
            .deactivate_wasm_channel(channel)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    } else {
        Ok(())
    };
    // Keep the runtime manager authoritative even when this channel was not
    // installed through ExtensionManager. `hot_remove` is intentionally
    // idempotent, so this also closes any metadata/runtime race.
    let runtime_result = agent
        .channels()
        .hot_remove(channel)
        .await
        .map_err(|error| error.to_string());
    match (metadata_result, runtime_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(metadata), Ok(())) => Err(format!("extension state cleanup failed: {metadata}")),
        (Ok(()), Err(runtime)) => Err(format!("channel shutdown failed: {runtime}")),
        (Err(metadata), Err(runtime)) => Err(format!(
            "extension state cleanup failed: {metadata}; channel shutdown failed: {runtime}"
        )),
    }
}

fn ensure_revision(expected: &str, actual: &str) -> Result<(), BridgeError> {
    if expected == actual {
        Ok(())
    } else {
        Err(BridgeError::Conflict {
            message: "Channel settings changed in another window or process".to_string(),
            remediation: Some("Reload Channel Center and apply the edit again".to_string()),
        })
    }
}

fn remote_mutation_error() -> BridgeError {
    BridgeError::Unavailable {
        capability: "remote channel credential mutation".to_string(),
        reason: "the effective remote gateway does not publish the revisioned secure channel-settings capability"
            .to_string(),
        remediation: Some("configure Slack or Telegram on the gateway host".to_string()),
        satisfied_by: RouteMode::LocalOnly,
    }
}

async fn apply_slack_channel_settings(
    manager: &ThinClawManager,
    secret_store: &SecretStore,
    runtime: &ThinClawRuntimeState,
    update: SlackChannelSettingsUpdate,
) -> Result<ChannelSettingsMutationResponse, BridgeError> {
    if runtime.remote_proxy().await.is_some() {
        return Err(remote_mutation_error());
    }
    let _guard = manager.channel_settings_lock.lock().await;
    let current = load_local_state(manager, runtime, secret_store).await?;
    ensure_revision(&update.expected_revision, &current.snapshot.revision)?;

    let bot_before = current.secrets.get(SLACK_BOT_SECRET).cloned().flatten();
    let app_before = current.secrets.get(SLACK_APP_SECRET).cloned().flatten();
    let signing_before = current.secrets.get(SLACK_SIGNING_SECRET).cloned().flatten();
    let bot_after = resolve_secret(
        &update.bot_token,
        bot_before.clone(),
        current.engine.channels.slack.bot_token.clone(),
        "bot_token",
    )?;
    let app_after = resolve_secret(
        &update.app_token,
        app_before.clone(),
        current.engine.channels.slack.app_token.clone(),
        "app_token",
    )?;
    let signing_after = resolve_secret(
        &update.signing_secret,
        signing_before.clone(),
        None,
        "signing_secret",
    )?;
    let runtime_credentials_changed = bot_before != bot_after || signing_before != signing_after;

    let dm_policy_submitted = update.dm_policy.is_some();
    let mut engine_after = current.engine.clone();
    if let Some(enabled) = update.enabled {
        engine_after.channels.slack.enabled = enabled;
    }
    if let Some(policy) = update.dm_policy {
        engine_after.channels.slack.dm_policy = validate_policy(policy, "dm_policy")?;
    }
    // Legacy plaintext is removed only in the same compensated transaction
    // that wrote and verified the secure replacement.
    engine_after.channels.slack.bot_token = None;
    engine_after.channels.slack.app_token = None;

    let mut patch = HashMap::new();
    let enabled = update.enabled.unwrap_or(current.snapshot.slack.enabled);
    if update.enabled.is_some()
        || current
            .settings
            .get(SLACK_ENABLED)
            .is_none_or(Option::is_none)
    {
        patch.insert(SLACK_ENABLED.to_string(), serde_json::json!(enabled));
    }
    if dm_policy_submitted
        || current
            .settings
            .get(SLACK_DM_POLICY)
            .is_none_or(Option::is_none)
    {
        patch.insert(
            SLACK_DM_POLICY.to_string(),
            serde_json::json!(engine_after.channels.slack.dm_policy),
        );
    }
    let before = patch
        .keys()
        .map(|key| {
            (
                key.clone(),
                current.settings.get(key.as_str()).cloned().flatten(),
            )
        })
        .collect();
    let secrets = [
        secret_plan(
            SLACK_BOT_SECRET,
            bot_before,
            bot_after.clone(),
            configured(current.engine.channels.slack.bot_token.as_deref()),
        ),
        secret_plan(
            SLACK_APP_SECRET,
            app_before,
            app_after.clone(),
            configured(current.engine.channels.slack.app_token.as_deref()),
        ),
        secret_plan(
            SLACK_SIGNING_SECRET,
            signing_before,
            signing_after.clone(),
            false,
        ),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut transaction = ProductionPersistence {
        secret_store,
        store: &current.store,
        settings_patch: patch,
        settings_before: before,
        secrets,
        config: &current.config,
        engine_before: &current.engine,
        engine_after: &engine_after,
        engine_existed: current.engine_existed,
    };
    commit_persistence(&mut transaction)
        .await
        .map_err(|message| BridgeError::Runtime { message })?;

    let configured = configured(bot_after.as_deref()) && configured(signing_after.as_deref());
    let runtime_updates = HashMap::from([(
        "dm_policy".to_string(),
        serde_json::json!(engine_after.channels.slack.dm_policy),
    )]);
    let (applied, restart_required, note) = apply_runtime(
        current.agent.as_ref(),
        "slack",
        enabled,
        configured,
        runtime_credentials_changed,
        runtime_updates,
    )
    .await;
    let snapshot = load_local_state(manager, runtime, secret_store)
        .await?
        .snapshot;
    Ok(ChannelSettingsMutationResponse {
        snapshot,
        persisted: true,
        applied,
        restart_required,
        note,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_update_slack_channel_settings(
    manager: State<'_, ThinClawManager>,
    secret_store: State<'_, SecretStore>,
    runtime: State<'_, ThinClawRuntimeState>,
    update: SlackChannelSettingsUpdate,
) -> Result<ChannelSettingsMutationResponse, BridgeError> {
    apply_slack_channel_settings(&manager, &secret_store, &runtime, update).await
}

async fn apply_telegram_channel_settings(
    manager: &ThinClawManager,
    secret_store: &SecretStore,
    runtime: &ThinClawRuntimeState,
    update: TelegramChannelSettingsUpdate,
) -> Result<ChannelSettingsMutationResponse, BridgeError> {
    if runtime.remote_proxy().await.is_some() {
        return Err(remote_mutation_error());
    }
    let _guard = manager.channel_settings_lock.lock().await;
    let current = load_local_state(manager, runtime, secret_store).await?;
    ensure_revision(&update.expected_revision, &current.snapshot.revision)?;

    let token_before = current.secrets.get(TELEGRAM_BOT_SECRET).cloned().flatten();
    let token_after = resolve_secret(
        &update.bot_token,
        token_before.clone(),
        current.engine.channels.telegram.bot_token.clone(),
        "bot_token",
    )?;
    let runtime_credentials_changed = token_before != token_after;
    let dm_policy_submitted = update.dm_policy.is_some();
    let mut engine_after = current.engine.clone();
    if let Some(enabled) = update.enabled {
        engine_after.channels.telegram.enabled = enabled;
    }
    if let Some(policy) = update.dm_policy {
        engine_after.channels.telegram.dm_policy = validate_policy(policy, "dm_policy")?;
    }
    if let Some(enabled) = update.groups_enabled {
        engine_after.channels.telegram.groups.set_enabled(enabled);
    }
    engine_after.channels.telegram.bot_token = None;

    let mut patch = HashMap::new();
    let enabled = update.enabled.unwrap_or(current.snapshot.telegram.enabled);
    if update.enabled.is_some()
        || current
            .settings
            .get(TELEGRAM_ENABLED)
            .is_none_or(Option::is_none)
    {
        patch.insert(TELEGRAM_ENABLED.to_string(), serde_json::json!(enabled));
    }
    if dm_policy_submitted
        || current
            .settings
            .get(TELEGRAM_DM_POLICY)
            .is_none_or(Option::is_none)
    {
        patch.insert(
            TELEGRAM_DM_POLICY.to_string(),
            serde_json::json!(engine_after.channels.telegram.dm_policy),
        );
    }
    if update.groups_enabled.is_some()
        || current
            .settings
            .get(TELEGRAM_GROUPS_ENABLED)
            .is_none_or(Option::is_none)
    {
        let groups_enabled = update
            .groups_enabled
            .unwrap_or(current.snapshot.telegram.groups_enabled);
        patch.insert(
            TELEGRAM_GROUPS_ENABLED.to_string(),
            serde_json::json!(groups_enabled),
        );
    }
    let before = patch
        .keys()
        .map(|key| {
            (
                key.clone(),
                current.settings.get(key.as_str()).cloned().flatten(),
            )
        })
        .collect();
    let secrets = secret_plan(
        TELEGRAM_BOT_SECRET,
        token_before,
        token_after.clone(),
        configured(current.engine.channels.telegram.bot_token.as_deref()),
    )
    .into_iter()
    .collect();

    let mut transaction = ProductionPersistence {
        secret_store,
        store: &current.store,
        settings_patch: patch,
        settings_before: before,
        secrets,
        config: &current.config,
        engine_before: &current.engine,
        engine_after: &engine_after,
        engine_existed: current.engine_existed,
    };
    commit_persistence(&mut transaction)
        .await
        .map_err(|message| BridgeError::Runtime { message })?;

    let runtime_updates = HashMap::from([
        (
            "dm_policy".to_string(),
            serde_json::json!(engine_after.channels.telegram.dm_policy),
        ),
        (
            "groups_enabled".to_string(),
            serde_json::json!(engine_after.channels.telegram.groups.enabled()),
        ),
    ]);
    let (applied, restart_required, note) = apply_runtime(
        current.agent.as_ref(),
        "telegram",
        enabled,
        configured(token_after.as_deref()),
        runtime_credentials_changed,
        runtime_updates,
    )
    .await;
    let snapshot = load_local_state(manager, runtime, secret_store)
        .await?
        .snapshot;
    Ok(ChannelSettingsMutationResponse {
        snapshot,
        persisted: true,
        applied,
        restart_required,
        note,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_update_telegram_channel_settings(
    manager: State<'_, ThinClawManager>,
    secret_store: State<'_, SecretStore>,
    runtime: State<'_, ThinClawRuntimeState>,
    update: TelegramChannelSettingsUpdate,
) -> Result<ChannelSettingsMutationResponse, BridgeError> {
    apply_telegram_channel_settings(&manager, &secret_store, &runtime, update).await
}

pub(super) async fn legacy_save_slack(
    manager: &ThinClawManager,
    secret_store: &SecretStore,
    runtime: &ThinClawRuntimeState,
    enabled: bool,
    bot_token: Option<String>,
    app_token: Option<String>,
) -> Result<(), BridgeError> {
    if runtime.remote_proxy().await.is_some() {
        return Err(remote_mutation_error());
    }
    let revision = load_local_state(manager, runtime, secret_store)
        .await?
        .snapshot
        .revision;
    let update = SlackChannelSettingsUpdate {
        expected_revision: revision,
        enabled: Some(enabled),
        dm_policy: None,
        bot_token: legacy_secret_mutation(bot_token),
        app_token: legacy_secret_mutation(app_token),
        signing_secret: ChannelSecretMutation::Preserve,
    };
    apply_slack_channel_settings(manager, secret_store, runtime, update)
        .await
        .map(|_| ())
}

pub(super) async fn legacy_save_telegram(
    manager: &ThinClawManager,
    secret_store: &SecretStore,
    runtime: &ThinClawRuntimeState,
    enabled: bool,
    bot_token: Option<String>,
    dm_policy: String,
    groups_enabled: bool,
) -> Result<(), BridgeError> {
    if runtime.remote_proxy().await.is_some() {
        return Err(remote_mutation_error());
    }
    let revision = load_local_state(manager, runtime, secret_store)
        .await?
        .snapshot
        .revision;
    let update = TelegramChannelSettingsUpdate {
        expected_revision: revision,
        enabled: Some(enabled),
        dm_policy: Some(dm_policy),
        groups_enabled: Some(groups_enabled),
        bot_token: legacy_secret_mutation(bot_token),
    };
    apply_telegram_channel_settings(manager, secret_store, runtime, update)
        .await
        .map(|_| ())
}

fn legacy_secret_mutation(value: Option<String>) -> ChannelSecretMutation {
    match value.map(|value| value.trim().to_string()) {
        Some(value) if !value.is_empty() => ChannelSecretMutation::Replace { value },
        _ => ChannelSecretMutation::Preserve,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_replace_is_never_clear() {
        let error = resolve_secret(
            &ChannelSecretMutation::Replace {
                value: "   ".to_string(),
            },
            Some("existing".to_string()),
            None,
            "bot_token",
        )
        .expect_err("blank replace must fail");
        assert!(matches!(error, BridgeError::InvalidInput { .. }));
    }

    #[test]
    fn preserve_migrates_legacy_only_when_secure_value_is_absent() {
        assert_eq!(
            resolve_secret(
                &ChannelSecretMutation::Preserve,
                None,
                Some("legacy".to_string()),
                "bot_token"
            )
            .unwrap(),
            Some("legacy".to_string())
        );
        assert_eq!(
            resolve_secret(
                &ChannelSecretMutation::Preserve,
                Some("secure".to_string()),
                Some("legacy".to_string()),
                "bot_token"
            )
            .unwrap(),
            Some("secure".to_string())
        );
    }

    #[test]
    fn legacy_plaintext_forces_secure_binding_proof_exactly_during_migration() {
        assert!(
            secret_plan(
                SLACK_BOT_SECRET,
                Some("secure".into()),
                Some("secure".into()),
                true,
            )
            .is_some()
        );
        assert!(
            secret_plan(
                SLACK_BOT_SECRET,
                Some("secure".into()),
                Some("secure".into()),
                false,
            )
            .is_none()
        );
    }

    #[test]
    fn explicit_clear_is_distinct_from_preserve() {
        assert_eq!(
            resolve_secret(
                &ChannelSecretMutation::Clear,
                Some("existing".to_string()),
                None,
                "bot_token"
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn snapshots_are_redacted() {
        let snapshot = ChannelSettingsSnapshot {
            available: true,
            editable: true,
            source: "local".into(),
            revision: content_revision(&serde_json::json!({"token": "super-secret"})),
            reason: None,
            slack: SlackChannelSettingsSnapshot {
                enabled: true,
                dm_policy: "pairing".into(),
                bot_token_configured: true,
                bot_token_migration_required: false,
                app_token_configured: true,
                app_token_migration_required: false,
                signing_secret_configured: true,
                active: false,
                status: "configured_not_running".into(),
            },
            telegram: TelegramChannelSettingsSnapshot {
                enabled: false,
                dm_policy: "pairing".into(),
                groups_enabled: true,
                require_mention: true,
                bot_token_configured: false,
                bot_token_migration_required: false,
                active: false,
                status: "disabled".into(),
            },
        };
        let encoded = serde_json::to_string(&snapshot).unwrap();
        assert!(!encoded.contains("super-secret"));
        assert!(!encoded.contains("bot_token\":"));
    }

    #[test]
    fn local_revision_is_stable_across_hashmap_insertion_order() {
        let channels = crate::thinclaw::config::ChannelsConfig {
            slack: Default::default(),
            telegram: Default::default(),
            extra: serde_json::Map::new(),
        };
        let mut settings_a = HashMap::new();
        settings_a.insert(SLACK_ENABLED, Some(serde_json::json!(true)));
        settings_a.insert(TELEGRAM_ENABLED, Some(serde_json::json!(false)));
        let mut settings_b = HashMap::new();
        settings_b.insert(TELEGRAM_ENABLED, Some(serde_json::json!(false)));
        settings_b.insert(SLACK_ENABLED, Some(serde_json::json!(true)));
        let mut secrets_a = HashMap::new();
        secrets_a.insert(SLACK_BOT_SECRET, Some("secret-a".to_string()));
        secrets_a.insert(TELEGRAM_BOT_SECRET, Some("secret-b".to_string()));
        let mut secrets_b = HashMap::new();
        secrets_b.insert(TELEGRAM_BOT_SECRET, Some("secret-b".to_string()));
        secrets_b.insert(SLACK_BOT_SECRET, Some("secret-a".to_string()));

        assert_eq!(
            local_content_revision(false, &channels, &settings_a, &secrets_a),
            local_content_revision(false, &channels, &settings_b, &secrets_b),
        );
    }

    #[test]
    fn remote_setup_presence_remains_per_credential_and_redacted() {
        let status = serde_json::json!({
            "channel_setup": {
                "slack": {
                    "enabled": true,
                    "configured": false,
                    "missing_fields": ["signing_secret"]
                }
            }
        });
        let (enabled, fields) =
            remote_setup_flags(&status, "slack", &["bot_token", "signing_secret"]);
        assert!(enabled);
        assert_eq!(fields.get("bot_token"), Some(&true));
        assert_eq!(fields.get("signing_secret"), Some(&false));
        assert!(!serde_json::to_string(&fields).unwrap().contains("xox"));
    }

    #[derive(Default)]
    struct FakePersistence {
        events: Vec<&'static str>,
        fail_settings: bool,
        fail_secret_verification: bool,
        fail_engine: bool,
    }

    #[async_trait]
    impl ChannelPersistence for FakePersistence {
        fn apply_secrets(&mut self) -> Result<(), String> {
            self.events.push("apply_secrets");
            Ok(())
        }

        async fn apply_settings(&mut self) -> Result<(), String> {
            self.events.push("apply_settings");
            if self.fail_settings {
                Err("settings failed".into())
            } else {
                Ok(())
            }
        }

        fn verify_secrets(&mut self) -> Result<(), String> {
            self.events.push("verify_secrets");
            if self.fail_secret_verification {
                Err("credential verification failed".into())
            } else {
                Ok(())
            }
        }

        fn apply_engine(&mut self) -> Result<(), String> {
            self.events.push("apply_engine");
            if self.fail_engine {
                Err("engine failed".into())
            } else {
                Ok(())
            }
        }

        async fn rollback_settings(&mut self) -> Result<(), String> {
            self.events.push("rollback_settings");
            Ok(())
        }

        fn rollback_secrets(&mut self) -> Result<(), String> {
            self.events.push("rollback_secrets");
            Ok(())
        }
    }

    #[tokio::test]
    async fn settings_failure_rolls_back_secret_first_stage() {
        let mut persistence = FakePersistence {
            fail_settings: true,
            ..Default::default()
        };
        assert!(commit_persistence(&mut persistence).await.is_err());
        assert_eq!(
            persistence.events,
            ["apply_secrets", "apply_settings", "rollback_secrets"]
        );
    }

    #[tokio::test]
    async fn legacy_engine_cleanup_is_last_after_secure_binding_and_settings_proof() {
        let mut persistence = FakePersistence::default();
        commit_persistence(&mut persistence)
            .await
            .expect("transaction should commit");
        assert_eq!(
            persistence.events,
            [
                "apply_secrets",
                "apply_settings",
                "verify_secrets",
                "apply_engine"
            ]
        );
    }

    #[tokio::test]
    async fn engine_failure_rolls_back_settings_and_secrets() {
        let mut persistence = FakePersistence {
            fail_engine: true,
            ..Default::default()
        };
        assert!(commit_persistence(&mut persistence).await.is_err());
        assert_eq!(
            persistence.events,
            [
                "apply_secrets",
                "apply_settings",
                "verify_secrets",
                "apply_engine",
                "rollback_settings",
                "rollback_secrets"
            ]
        );
    }

    #[tokio::test]
    async fn credential_failure_after_settings_rolls_back_both_stores() {
        let mut persistence = FakePersistence {
            fail_secret_verification: true,
            ..Default::default()
        };
        assert!(commit_persistence(&mut persistence).await.is_err());
        assert_eq!(
            persistence.events,
            [
                "apply_secrets",
                "apply_settings",
                "verify_secrets",
                "rollback_settings",
                "rollback_secrets"
            ]
        );
    }

    #[test]
    fn stale_revision_is_a_typed_conflict() {
        assert!(matches!(
            ensure_revision("old", "new"),
            Err(BridgeError::Conflict { .. })
        ));
    }

    #[test]
    fn telegram_group_presence_roundtrips_and_preserves_unknown_fields() {
        let mut config: crate::thinclaw::config::TelegramConfig = serde_json::from_value(
            serde_json::json!({
                "enabled": true,
                "dmPolicy": "pairing",
                "groups": {"*": {"requireMention": false, "futureRule": 7}, "named": {"allow": true}},
                "futureChannelField": {"x": 1}
            }),
        )
        .unwrap();
        config.groups.set_enabled(false);
        let disabled = serde_json::to_value(&config).unwrap();
        assert!(disabled["groups"].get("*").is_none());
        assert_eq!(disabled["groups"]["named"]["allow"], true);
        assert_eq!(
            disabled["groups"]["_disabledWildcard"]["requireMention"],
            false
        );
        assert_eq!(disabled["futureChannelField"]["x"], 1);
        let disabled_roundtrip: crate::thinclaw::config::TelegramConfig =
            serde_json::from_value(disabled).unwrap();
        assert!(!disabled_roundtrip.groups.enabled());
        config.groups.set_enabled(true);
        let enabled = serde_json::to_value(&config).unwrap();
        assert_eq!(enabled["groups"]["*"]["requireMention"], false);
        assert!(enabled["groups"].get("_disabledWildcard").is_none());
    }

    #[test]
    fn compatibility_config_preserves_unknown_top_level_and_channel_definitions() {
        let config: crate::thinclaw::config::ThinClawEngineConfig = serde_json::from_value(
            serde_json::json!({
                "gateway": {
                    "mode": "local",
                    "bind": "loopback",
                    "port": 3030,
                    "auth": {"mode": "token", "token": "redacted-test-token"}
                },
                "discovery": {"mdns": {"mode": "off"}},
                "agents": {"defaults": {"workspace": "/tmp/workspace"}},
                "channels": {
                    "slack": {"enabled": false, "channels": {}, "futureSlackRule": 7},
                    "telegram": {"enabled": false, "dmPolicy": "pairing"},
                    "futureChannel": {"enabled": true, "mode": "new"}
                },
                "futureTopLevel": {"version": 2}
            }),
        )
        .unwrap();
        let encoded = serde_json::to_value(config).unwrap();
        assert_eq!(encoded["futureTopLevel"]["version"], 2);
        assert_eq!(encoded["channels"]["futureChannel"]["mode"], "new");
        assert_eq!(encoded["channels"]["slack"]["futureSlackRule"], 7);
    }
}
