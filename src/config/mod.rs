//! Configuration for IronClaw.
//!
//! Settings are loaded with priority: env var > database > default.
//! `DATABASE_URL` lives in `~/.ironclaw/.env` (loaded via dotenvy early
//! in startup). Everything else comes from env vars, the DB settings
//! table, or auto-detection.

mod agent;
mod builder;
mod channels;
mod database;
mod embeddings;
mod heartbeat;
pub(crate) mod helpers;
mod hygiene;
mod llm;
mod routines;
mod safety;
mod sandbox;
mod secrets;
mod skills;
mod tunnel;
mod wasm;
pub mod watcher;

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use crate::error::ConfigError;
use crate::settings::Settings;

// Re-export all public types so `crate::config::FooConfig` continues to work.
pub use self::agent::AgentConfig;
pub use self::builder::BuilderModeConfig;
pub use self::channels::{
    ChannelsConfig, CliConfig, DiscordChannelConfig, GatewayConfig, HttpConfig, NostrConfig,
    SignalConfig, SlackChannelConfig, TelegramConfig,
};
pub use self::database::{DatabaseBackend, DatabaseConfig, default_libsql_path};
pub use self::embeddings::EmbeddingsConfig;
pub use self::heartbeat::HeartbeatConfig;
pub use self::hygiene::HygieneConfig;
pub use self::llm::{
    AnthropicDirectConfig, LlmBackend, LlmConfig, OllamaConfig, OpenAiCompatibleConfig,
    OpenAiDirectConfig, ReliabilityConfig, TinfoilConfig,
};
pub use self::routines::RoutineConfig;
pub use self::safety::SafetyConfig;
pub use self::sandbox::{ClaudeCodeConfig, SandboxModeConfig};
pub use self::secrets::SecretsConfig;
pub use self::skills::SkillsConfig;
pub use self::tunnel::TunnelConfig;
pub use self::wasm::WasmConfig;

/// Thread-safe overlay for injected env vars (secrets loaded from DB).
///
/// Used by `inject_llm_keys_from_secrets()` and `refresh_secrets()` to make
/// API keys available to `optional_env()` without unsafe `set_var` calls.
/// `optional_env()` checks real env vars first, then falls back to this overlay.
///
/// Uses `RwLock` (not `OnceLock`) so secrets can be updated at runtime via
/// `refresh_secrets()` without requiring a full restart.
static INJECTED_VARS: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Main configuration for the agent.
#[derive(Debug, Clone)]
pub struct Config {
    pub database: DatabaseConfig,
    pub llm: LlmConfig,
    pub embeddings: EmbeddingsConfig,
    pub tunnel: TunnelConfig,
    pub channels: ChannelsConfig,
    pub agent: AgentConfig,
    pub safety: SafetyConfig,
    pub wasm: WasmConfig,
    pub secrets: SecretsConfig,
    pub builder: BuilderModeConfig,
    pub heartbeat: HeartbeatConfig,
    pub hygiene: HygieneConfig,
    pub routines: RoutineConfig,
    pub sandbox: SandboxModeConfig,
    pub claude_code: ClaudeCodeConfig,
    pub skills: SkillsConfig,
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
        crate::bootstrap::load_ironclaw_env();

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

        Self::build(&db_settings).await
    }

    /// Load configuration from environment variables only (no database).
    ///
    /// Used during early startup before the database is connected,
    /// and by CLI commands that don't have DB access.
    /// Falls back to legacy `settings.json` on disk if present.
    ///
    /// Loads both `./.env` (standard, higher priority) and `~/.ironclaw/.env`
    /// (lower priority) via dotenvy, which never overwrites existing vars.
    pub async fn from_env() -> Result<Self, ConfigError> {
        Self::from_env_with_toml(None).await
    }

    /// Load from env with an optional TOML config file overlay.
    pub async fn from_env_with_toml(
        toml_path: Option<&std::path::Path>,
    ) -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        crate::bootstrap::load_ironclaw_env();
        let mut settings = Settings::load();

        // Overlay TOML config file (values win over JSON settings)
        Self::apply_toml_overlay(&mut settings, toml_path)?;

        Self::build(&settings).await
    }

    /// Load and merge a TOML config file into settings.
    ///
    /// If `explicit_path` is `Some`, loads from that path (errors are fatal).
    /// If `None`, tries the default path `~/.ironclaw/config.toml` (missing
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
    async fn build(settings: &Settings) -> Result<Self, ConfigError> {
        Ok(Self {
            database: DatabaseConfig::resolve()?,
            llm: LlmConfig::resolve(settings)?,
            embeddings: EmbeddingsConfig::resolve(settings)?,
            tunnel: TunnelConfig::resolve(settings)?,
            channels: ChannelsConfig::resolve(settings)?,
            agent: AgentConfig::resolve(settings)?,
            safety: SafetyConfig::resolve()?,
            wasm: WasmConfig::resolve()?,
            secrets: SecretsConfig::resolve().await?,
            builder: BuilderModeConfig::resolve()?,
            heartbeat: HeartbeatConfig::resolve(settings)?,
            hygiene: HygieneConfig::resolve()?,
            routines: RoutineConfig::resolve()?,
            sandbox: SandboxModeConfig::resolve()?,
            claude_code: ClaudeCodeConfig::resolve()?,
            skills: SkillsConfig::resolve()?,
            observability: crate::observability::ObservabilityConfig {
                backend: std::env::var("OBSERVABILITY_BACKEND").unwrap_or_else(|_| "none".into()),
            },
        })
    }
}

/// Load API keys from the encrypted secrets store into a thread-safe overlay.
///
/// This bridges the gap between secrets stored during onboarding and the
/// env-var-first resolution in `LlmConfig::resolve()`. Keys in the overlay
/// are read by `optional_env()` before falling back to `std::env::var()`,
/// so explicit env vars always win.
pub async fn inject_llm_keys_from_secrets(
    secrets: &dyn crate::secrets::SecretsStore,
    user_id: &str,
) {
    let mappings = [
        ("llm_openai_api_key", "OPENAI_API_KEY"),
        ("llm_anthropic_api_key", "ANTHROPIC_API_KEY"),
        ("llm_compatible_api_key", "LLM_API_KEY"),
        ("llm_tinfoil_api_key", "TINFOIL_API_KEY"),
    ];

    let mut injected = HashMap::new();

    for (secret_name, env_var) in mappings {
        match std::env::var(env_var) {
            Ok(val) if !val.is_empty() => continue,
            _ => {}
        }
        match secrets.get_decrypted(user_id, secret_name).await {
            Ok(decrypted) => {
                injected.insert(env_var.to_string(), decrypted.expose().to_string());
                tracing::debug!("Loaded secret '{}' for env var '{}'", secret_name, env_var);
            }
            Err(_) => {
                // Secret doesn't exist, that's fine
            }
        }
    }

    update_injected_vars(injected);
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

/// Reload secrets from the store and update the overlay.
///
/// This is the zero-downtime secret refresh API. When a user updates an API
/// key in Scrappy's UI, Scrappy writes it to the SecretsStore and then calls
/// this function. IronClaw re-reads all secrets, updates the injected vars
/// overlay, and the next config resolution picks up the new keys.
///
/// Returns the number of secrets that were (re)loaded.
pub async fn refresh_secrets(secrets: &dyn crate::secrets::SecretsStore, user_id: &str) -> usize {
    let mappings = [
        ("llm_openai_api_key", "OPENAI_API_KEY"),
        ("llm_anthropic_api_key", "ANTHROPIC_API_KEY"),
        ("llm_compatible_api_key", "LLM_API_KEY"),
        ("llm_tinfoil_api_key", "TINFOIL_API_KEY"),
    ];

    let mut injected = HashMap::new();

    for (secret_name, env_var) in mappings {
        // Skip if a real env var is set (env always wins)
        match std::env::var(env_var) {
            Ok(val) if !val.is_empty() => continue,
            _ => {}
        }
        if let Ok(decrypted) = secrets.get_decrypted(user_id, secret_name).await {
            injected.insert(env_var.to_string(), decrypted.expose().to_string());
            tracing::debug!(
                "Refreshed secret '{}' for env var '{}'",
                secret_name,
                env_var
            );
        }
    }

    let count = injected.len();
    update_injected_vars(injected);

    tracing::info!("Secrets refreshed: {} key(s) updated in overlay", count);

    count
}
