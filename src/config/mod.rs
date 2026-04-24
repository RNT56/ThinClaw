//! Configuration for ThinClaw.
//!
//! Settings are loaded with priority: env var > database > default.
//! `DATABASE_URL` lives in `~/.thinclaw/.env` (loaded via dotenvy early
//! in startup). Everything else comes from env vars, the DB settings
//! table, or auto-detection.

mod agent;
mod builder;
mod channels;
mod database;
mod desktop_autonomy;
mod embeddings;
mod experiments;
pub mod formats;
mod heartbeat;
pub(crate) mod helpers;
mod hygiene;
mod llm;
pub mod mdns_discovery;
pub mod model_compat;
pub mod network_modes;
pub mod provider_catalog;
mod routines;
mod safety;
mod sandbox;
mod secrets;
mod skills;
pub mod tunnel;
mod wasm;
pub mod watcher;
mod webchat;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::error::ConfigError;
use crate::secrets::SecretsStore;
use crate::settings::Settings;

// Re-export all public types so `crate::config::FooConfig` continues to work.
pub use self::agent::AgentConfig;
pub(crate) use self::agent::resolve_personality_pack_from_settings;
pub use self::builder::BuilderModeConfig;
#[cfg(feature = "nostr")]
pub use self::channels::NostrConfig;
pub use self::channels::{
    BlueBubblesChannelConfig, ChannelsConfig, CliConfig, DiscordChannelConfig, GatewayConfig,
    HttpConfig, SignalConfig, SlackChannelConfig, TelegramConfig,
};
pub use self::database::{DatabaseBackend, DatabaseConfig, default_libsql_path};
pub use self::desktop_autonomy::DesktopAutonomyConfig;
pub use self::embeddings::EmbeddingsConfig;
pub use self::experiments::ExperimentsConfig;
pub use self::heartbeat::HeartbeatConfig;
pub use self::hygiene::HygieneConfig;
pub use self::llm::{
    AnthropicDirectConfig, BedrockDirectConfig, GeminiDirectConfig, LlamaCppConfig, LlmBackend,
    LlmConfig, OllamaConfig, OpenAiCompatibleConfig, OpenAiDirectConfig, ReliabilityConfig,
    TinfoilConfig,
};
pub use self::routines::RoutineConfig;
pub use self::safety::SafetyConfig;
pub use self::sandbox::{ClaudeCodeConfig, CodexCodeConfig, SandboxModeConfig};
pub use self::secrets::SecretsConfig;
pub use self::skills::SkillsConfig;
pub use self::tunnel::{
    CloudflareTunnelConfig, CustomTunnelConfig, NgrokTunnelConfig, TailscaleTunnelConfig,
    TunnelConfig, TunnelProviderConfig,
};
pub use self::wasm::WasmConfig;
pub use self::webchat::{
    ResolvedWebSkin, WebChatBootstrap, WebChatConfig, WebChatPresentation, WebChatTheme,
    WebSkinCatalogEntry,
};

/// Thread-safe overlay for legacy runtime-injected values.
///
/// Used by `inject_all_secrets_from_store()` and `refresh_secrets()` to make
/// Runtime-only values available to `optional_env()` without unsafe `set_var` calls.
///
/// Stored secrets are intentionally not preloaded here. Runtime paths must
/// request the one credential they need through the encrypted `SecretsStore`.
static INJECTED_VARS: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Thread-safe overlay for explicitly enabled external auth sync sources.
///
/// This intentionally lives outside `optional_env()` so synced Codex/Claude
/// auth cannot silently shadow stored provider API keys. Provider resolution
/// consults this overlay only when the user explicitly selected
/// `external_oauth_sync` for that provider.
static SYNCED_OAUTH_VARS: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// IC-007: Thread-safe overlay for bridge-injected configuration.
///
/// The Tauri bridge (`thinclaw_bridge.rs`) calls [`inject_bridge_vars()`] to pass
/// UI-derived configuration (LLM backend, workspace mode, heartbeat, etc.) into
/// ThinClaw's config resolvers **without** unsafe `std::env::set_var()` calls.
///
/// `optional_env()` checks this overlay FIRST (highest priority), then falls
/// through to `INJECTED_VARS` (legacy runtime overlay), then to real env vars.
///
/// Lifecycle: populated on engine `start()`, cleared on engine `stop()`.
static BRIDGE_VARS: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// IC-007: Inject bridge configuration variables into the overlay.
///
/// Called by the Tauri bridge to pass Scrappy UI configuration to ThinClaw's
/// config resolvers without unsafe `set_var`. Values in the bridge overlay
/// take priority over secrets and real env vars.
///
/// Merges into existing bridge vars (does not replace the entire map).
pub fn inject_bridge_vars(vars: HashMap<String, String>) {
    match BRIDGE_VARS.write() {
        Ok(mut guard) => {
            guard.extend(vars);
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.extend(vars);
        }
    }
}

/// IC-007: Remove specific keys from the bridge overlay.
///
/// Called by the bridge's `stop()` to clear LLM config so the next
/// `start()` re-detects from fresh UI state.
pub fn remove_bridge_vars(keys: &[&str]) {
    match BRIDGE_VARS.write() {
        Ok(mut guard) => {
            for key in keys {
                guard.remove(*key);
            }
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            for key in keys {
                guard.remove(*key);
            }
        }
    }
}

/// IC-007: Clear all bridge overlay vars.
///
/// Called during full engine shutdown to reset all bridge-injected config.
pub fn clear_bridge_vars() {
    match BRIDGE_VARS.write() {
        Ok(mut guard) => guard.clear(),
        Err(poisoned) => poisoned.into_inner().clear(),
    }
}

/// Clear all injected secret-overlay vars (test support).
#[cfg(test)]
pub(crate) fn clear_injected_vars_for_tests() {
    match INJECTED_VARS.write() {
        Ok(mut guard) => guard.clear(),
        Err(poisoned) => poisoned.into_inner().clear(),
    }
    match SYNCED_OAUTH_VARS.write() {
        Ok(mut guard) => guard.clear(),
        Err(poisoned) => poisoned.into_inner().clear(),
    }
}

/// IC-007: Check whether a key exists in the bridge overlay.
///
/// Used by the bridge to replicate the `is_err()` guard logic:
/// "only set defaults if the user hasn't already configured this var."
pub fn bridge_var_exists(key: &str) -> bool {
    if let Ok(guard) = BRIDGE_VARS.read()
        && guard.contains_key(key)
    {
        return true;
    }
    std::env::var(key).is_ok()
}

/// Main configuration for the agent.
#[derive(Debug, Clone)]
pub struct Config {
    pub database: DatabaseConfig,
    pub llm: LlmConfig,
    pub embeddings: EmbeddingsConfig,
    pub tunnel: TunnelConfig,
    pub channels: ChannelsConfig,
    pub agent: AgentConfig,
    pub desktop_autonomy: DesktopAutonomyConfig,
    pub safety: SafetyConfig,
    pub wasm: WasmConfig,
    pub secrets: SecretsConfig,
    pub builder: BuilderModeConfig,
    pub heartbeat: HeartbeatConfig,
    pub hygiene: HygieneConfig,
    pub extensions: crate::settings::ExtensionsSettings,
    pub routines: RoutineConfig,
    pub sandbox: SandboxModeConfig,
    pub claude_code: ClaudeCodeConfig,
    pub codex_code: CodexCodeConfig,
    pub skills: SkillsConfig,
    pub experiments: ExperimentsConfig,
    pub observability: crate::observability::ObservabilityConfig,
}

impl Config {
    /// Load configuration from environment variables and the database.
    ///
    /// Priority: env var > TOML config file > DB settings > default.
    /// This is the primary way to load config after DB is connected.
    pub async fn from_db(
        store: &(dyn crate::db::SettingsStore + Sync),
        user_id: &str,
    ) -> Result<Self, ConfigError> {
        Self::from_db_with_toml(store, user_id, None).await
    }

    /// Load from DB with an optional TOML config file overlay.
    pub async fn from_db_with_toml(
        store: &(dyn crate::db::SettingsStore + Sync),
        user_id: &str,
        toml_path: Option<&std::path::Path>,
    ) -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_thinclaw_env();

        // Load all settings from DB into a Settings struct
        let mut db_settings = match store.get_all_settings(user_id).await {
            Ok(map) => Settings::from_db_map(&map),
            Err(e) => {
                tracing::warn!("Failed to load settings from DB, using defaults: {}", e);
                Settings::default()
            }
        };

        // Overlay TOML config file (values win over DB settings)
        Self::apply_toml_overlay(&mut db_settings, toml_path)?;

        Self::build(&db_settings, true).await
    }

    /// Load configuration from environment variables only (no database).
    ///
    /// Used during early startup before the database is connected,
    /// and by CLI commands that don't have DB access.
    /// Falls back to legacy `settings.json` on disk if present.
    ///
    /// Loads both `./.env` (standard, higher priority) and `~/.thinclaw/.env`
    /// (lower priority) via dotenvy, which never overwrites existing vars.
    pub async fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_with_toml(None).await
    }

    /// Load from env with an optional TOML config file overlay.
    pub async fn from_env_with_toml(
        toml_path: Option<&std::path::Path>,
    ) -> Result<Self, ConfigError> {
        Self::from_env_with_toml_options(toml_path, true).await
    }

    /// Load from env with an optional TOML config file overlay and optional DB resolution.
    pub async fn from_env_with_toml_options(
        toml_path: Option<&std::path::Path>,
        resolve_database: bool,
    ) -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_thinclaw_env();
        let mut settings = Settings::load();

        // Overlay TOML config file (values win over JSON settings)
        Self::apply_toml_overlay(&mut settings, toml_path)?;

        Self::build(&settings, resolve_database).await
    }

    /// Load and merge a TOML config file into settings.
    ///
    /// If `explicit_path` is `Some`, loads from that path (errors are fatal).
    /// If `None`, tries the default path `~/.thinclaw/config.toml` (missing
    /// file is silently ignored).
    fn apply_toml_overlay(
        settings: &mut Settings,
        explicit_path: Option<&std::path::Path>,
    ) -> Result<(), ConfigError> {
        let path = explicit_path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(Settings::default_toml_path);

        match Settings::load_toml(&path) {
            Ok(Some(toml_settings)) => {
                settings.merge_from(&toml_settings);
                tracing::debug!("Loaded TOML config from {}", path.display());
            }
            Ok(None) => {
                if explicit_path.is_some() {
                    return Err(ConfigError::ParseError(format!(
                        "Config file not found: {}",
                        path.display()
                    )));
                }
            }
            Err(e) => {
                if explicit_path.is_some() {
                    return Err(ConfigError::ParseError(format!(
                        "Failed to load config file {}: {}",
                        path.display(),
                        e
                    )));
                }
                tracing::warn!("Failed to load default config file: {}", e);
            }
        }
        Ok(())
    }

    /// Build config from settings (shared by from_env and from_db).
    async fn build(settings: &Settings, resolve_database: bool) -> Result<Self, ConfigError> {
        Ok(Self {
            database: if resolve_database {
                DatabaseConfig::resolve()?
            } else {
                DatabaseConfig::disabled()
            },
            llm: LlmConfig::resolve(settings)?,
            embeddings: EmbeddingsConfig::resolve(settings)?,
            tunnel: TunnelConfig::resolve(settings)?,
            channels: ChannelsConfig::resolve(settings)?,
            agent: AgentConfig::resolve(settings)?,
            desktop_autonomy: DesktopAutonomyConfig::resolve(settings)?,
            safety: SafetyConfig::resolve(settings)?,
            wasm: WasmConfig::resolve(settings)?,
            secrets: SecretsConfig::resolve(settings).await?,
            builder: BuilderModeConfig::resolve()?,
            heartbeat: HeartbeatConfig::resolve(settings)?,
            hygiene: HygieneConfig::resolve()?,
            extensions: settings.extensions.clone(),
            routines: RoutineConfig::resolve(settings)?,
            sandbox: SandboxModeConfig::resolve(settings)?,
            claude_code: ClaudeCodeConfig::resolve(settings)?,
            codex_code: CodexCodeConfig::resolve(settings)?,
            skills: SkillsConfig::resolve(settings)?,
            experiments: ExperimentsConfig::resolve(settings)?,
            observability: crate::observability::ObservabilityConfig {
                backend: helpers::optional_env("OBSERVABILITY_BACKEND")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "none".to_string()),
            },
        })
    }
}

async fn collect_injected_secrets(
    _secrets: &dyn crate::secrets::SecretsStore,
    _user_id: &str,
) -> HashMap<String, String> {
    HashMap::new()
}

/// Clear the legacy injected-secret overlay.
///
/// Stored secrets are now resolved by scoped runtime callers through
/// `SecretsStore::get_for_injection`; this function remains for older reload
/// call sites so they drop any pre-hardening overlay values.
pub async fn inject_all_secrets_from_store(
    secrets: &dyn crate::secrets::SecretsStore,
    user_id: &str,
) {
    let injected = collect_injected_secrets(secrets, user_id).await;
    let count = injected.len();
    update_injected_vars(injected);
    tracing::info!(
        "Secret overlay refresh complete: {} legacy key(s) loaded",
        count
    );
}

/// Backwards-compatible wrapper for older call sites.
pub async fn inject_llm_keys_from_secrets(
    secrets: &dyn crate::secrets::SecretsStore,
    user_id: &str,
) {
    inject_all_secrets_from_store(secrets, user_id).await;
}

/// Replace the injected vars overlay atomically.
///
/// Used by both initial injection and runtime refresh.
fn update_injected_vars(new_vars: HashMap<String, String>) {
    match INJECTED_VARS.write() {
        Ok(mut guard) => {
            *guard = new_vars;
        }
        Err(poisoned) => {
            // Recover from a poisoned lock
            let mut guard = poisoned.into_inner();
            *guard = new_vars;
        }
    }
}

/// Replace the synced external-auth overlay atomically.
pub fn replace_synced_oauth_vars(new_vars: HashMap<String, String>) -> usize {
    let count = new_vars.len();
    match SYNCED_OAUTH_VARS.write() {
        Ok(mut guard) => {
            *guard = new_vars;
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            *guard = new_vars;
        }
    }
    count
}

/// Clear all synced external-auth values.
pub fn clear_synced_oauth_vars() {
    match SYNCED_OAUTH_VARS.write() {
        Ok(mut guard) => guard.clear(),
        Err(poisoned) => poisoned.into_inner().clear(),
    }
}

/// Merge specific values into the legacy injected runtime overlay without
/// replacing the rest of the map.
///
/// This is used by hot-reload paths such as external OAuth credential syncing,
/// where a subset of provider credentials may update independently of the main
/// secrets store.
pub fn merge_injected_vars(vars: HashMap<String, String>) -> usize {
    let count = vars.len();
    match INJECTED_VARS.write() {
        Ok(mut guard) => {
            guard.extend(vars);
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            guard.extend(vars);
        }
    }
    count
}

/// Reload stored secrets and clear the legacy overlay.
///
/// Runtime credential consumers now resolve individual secrets directly through
/// `SecretsStore::get_for_injection`. This compatibility function keeps reload
/// callers clearing stale overlay values from earlier versions.
///
/// Returns the number of secrets that were (re)loaded.
pub async fn refresh_secrets(secrets: &dyn crate::secrets::SecretsStore, user_id: &str) -> usize {
    let injected = collect_injected_secrets(secrets, user_id).await;
    let count = injected.len();
    update_injected_vars(injected);

    tracing::info!("Secrets refreshed: {} key(s) updated in overlay", count);

    count
}

async fn provider_secret_from_store(
    user_id: &str,
    secret_name: &str,
    secrets: Option<&Arc<dyn SecretsStore + Send + Sync>>,
) -> Option<String> {
    if let Some(store) = secrets
        && let Ok(secret) = store
            .get_for_injection(
                user_id,
                secret_name,
                crate::secrets::SecretAccessContext::new(
                    "config.secret_resolver",
                    "provider_credential",
                ),
            )
            .await
    {
        let value = secret.expose().trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }

    None
}

#[cfg(not(target_os = "macos"))]
async fn provider_secret_from_os_secure_store(secret_name: &str) -> Option<String> {
    if let Some(value) = crate::platform::secure_store::get_api_key(secret_name).await
        && !value.trim().is_empty()
    {
        return Some(value);
    }

    None
}

/// Resolve a provider credential using the same precedence as the WebUI and runtime.
///
/// Resolution order:
/// 1. Env/overlay (`optional_env`)
/// 2. On macOS: encrypted secrets store
/// 3. On other platforms: OS secure store, then encrypted secrets store
/// 4. Provider-specific legacy env aliases
pub async fn resolve_provider_secret_value(
    user_id: &str,
    env_key: &str,
    secret_name: &str,
    secrets: Option<&Arc<dyn SecretsStore + Send + Sync>>,
) -> Option<String> {
    if let Ok(Some(value)) = helpers::optional_env(env_key)
        && !value.trim().is_empty()
    {
        return Some(value);
    }

    #[cfg(target_os = "macos")]
    if let Some(value) = provider_secret_from_store(user_id, secret_name, secrets).await {
        return Some(value);
    }

    #[cfg(not(target_os = "macos"))]
    if let Some(value) = provider_secret_from_os_secure_store(secret_name).await {
        return Some(value);
    }

    #[cfg(not(target_os = "macos"))]
    if let Some(value) = provider_secret_from_store(user_id, secret_name, secrets).await {
        return Some(value);
    }

    match env_key {
        "OPENROUTER_API_KEY" => {
            if let Ok(Some(value)) = helpers::optional_env("LLM_API_KEY")
                && !value.trim().is_empty()
            {
                return Some(value);
            }
        }
        "BEDROCK_API_KEY" => {
            if let Ok(Some(value)) = helpers::optional_env("AWS_BEARER_TOKEN_BEDROCK")
                && !value.trim().is_empty()
            {
                return Some(value);
            }
        }
        _ => {}
    }

    None
}
