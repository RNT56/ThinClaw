//! Main setup wizard orchestration.
//!
//! The wizard guides users through 19 steps, grouped by concern:
//!
//! **Infrastructure**
//! 1. Database connection
//! 2. Security (secrets master key)
//!
//! **LLM Configuration**
//! 3. Inference provider (Anthropic, OpenAI, Ollama, OpenRouter, OpenAI-compatible)
//! 4. Model selection
//! 5. Smart routing (cheap/fast secondary model)
//! 6. Fallback providers (secondary providers for failover)
//! 7. Embeddings (semantic search)
//!
//! **Agent Personality**
//! 8. Agent identity (name)
//!
//! **Communication Channels**
//! 9. Channel configuration (Telegram, Discord, Slack, Signal, etc.)
//!
//! **Capabilities & Execution**
//! 10. Extensions (tool installation from registry)
//! 11. Local tools & Docker sandbox
//! 12. Claude Code sandbox
//! 13. Tool approval mode
//!
//! **Automation**
//! 14. Routines (scheduled tasks)
//! 15. Skills (capability plugins)
//! 16. Heartbeat (background tasks)
//!
//! **Presentation & Operations**
//! 17. Notification preferences
//! 18. Web UI (theme, accent, branding)
//! 19. Observability (event/metric recording)

use std::sync::Arc;

use secrecy::SecretString;

use crate::secrets::SecretsCrypto;
use crate::settings::Settings;
use crate::setup::prompts::{print_header, print_info, print_step};

// Channel selection indices — must match the order in step_channels() options vec.
// const CHANNEL_INDEX_CLI: usize = 0;
const CHANNEL_INDEX_HTTP: usize = 1;
const CHANNEL_INDEX_SIGNAL: usize = 2;
const CHANNEL_INDEX_DISCORD: usize = 3;
const CHANNEL_INDEX_SLACK: usize = 4;
const CHANNEL_INDEX_NOSTR: usize = 5;
const CHANNEL_INDEX_GMAIL: usize = 6;
#[cfg(target_os = "macos")]
const CHANNEL_INDEX_IMESSAGE: usize = 7;
#[cfg(target_os = "macos")]
const CHANNEL_INDEX_APPLE_MAIL: usize = 8;
// WASM channels start after the native channels (dynamically computed as `native_count`)

/// Setup wizard error.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Channel setup error: {0}")]
    Channel(String),

    #[error("User cancelled")]
    Cancelled,
}

impl From<crate::setup::channels::ChannelSetupError> for SetupError {
    fn from(e: crate::setup::channels::ChannelSetupError) -> Self {
        SetupError::Channel(e.to_string())
    }
}

/// Setup wizard configuration.
#[derive(Debug, Clone, Default)]
pub struct SetupConfig {
    /// Skip authentication step (use existing session).
    pub skip_auth: bool,
    /// Only reconfigure channels.
    pub channels_only: bool,
}

/// Interactive setup wizard for ThinClaw.
pub struct SetupWizard {
    config: SetupConfig,
    settings: Settings,

    /// Database pool (created during setup, postgres only).
    #[cfg(feature = "postgres")]
    db_pool: Option<deadpool_postgres::Pool>,
    /// libSQL backend (created during setup, libsql only).
    #[cfg(feature = "libsql")]
    db_backend: Option<crate::db::libsql::LibSqlBackend>,
    /// Secrets crypto (created during setup).
    secrets_crypto: Option<Arc<SecretsCrypto>>,
    /// Cached API key from provider setup (used by model fetcher without env mutation).
    llm_api_key: Option<SecretString>,
}

impl SetupWizard {
    /// Create a new setup wizard.
    pub fn new() -> Self {
        Self {
            config: SetupConfig::default(),
            settings: Settings::default(),
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
        }
    }

    /// Create a wizard with custom configuration.
    pub fn with_config(config: SetupConfig) -> Self {
        Self {
            config,
            settings: Settings::default(),
            #[cfg(feature = "postgres")]
            db_pool: None,
            #[cfg(feature = "libsql")]
            db_backend: None,
            secrets_crypto: None,
            llm_api_key: None,
        }
    }

    /// Run the setup wizard.
    ///
    /// Settings are persisted incrementally after each successful step so
    /// that progress is not lost if a later step fails. On re-run, existing
    /// settings are loaded from the database after Step 1 establishes a
    /// connection, so users don't have to re-enter everything.
    pub async fn run(&mut self) -> Result<(), SetupError> {
        print_header("ThinClaw Setup Wizard");

        if self.config.channels_only {
            // Channels-only mode: reconnect to existing DB and load settings
            // before running the channel step, so secrets and save work.
            self.reconnect_existing_db().await?;
            print_step(1, 1, "Channel Configuration");
            self.step_channels().await?;
        } else {
            let total_steps = 19;

            // ── Infrastructure ───────────────────────────────────────────

            // Step 1: Database
            print_step(1, total_steps, "Database Connection");
            self.step_database().await?;

            // After establishing a DB connection, load any previously saved
            // settings so we recover progress from prior partial runs.
            // We must load BEFORE persisting, otherwise persist_after_step()
            // would overwrite prior settings with defaults.
            // Save Step 1 choices first so they aren't clobbered by stale
            // DB values (merge_from only applies non-default fields).
            let step1_settings = self.settings.clone();
            self.try_load_existing_settings().await;
            self.settings.merge_from(&step1_settings);

            self.persist_after_step().await;

            // Step 2: Security
            print_step(2, total_steps, "Security");
            self.step_security().await?;
            self.persist_after_step().await;

            // ── LLM Configuration ────────────────────────────────────────

            // Step 3: Inference provider selection (unless skipped)
            if !self.config.skip_auth {
                print_step(3, total_steps, "Inference Provider");
                self.step_inference_provider().await?;
            } else {
                print_info("Skipping inference provider setup (using existing config)");
            }
            self.persist_after_step().await;

            // Step 4: Model selection
            print_step(4, total_steps, "Model Selection");
            self.step_model_selection().await?;
            self.persist_after_step().await;

            // Step 5: Smart Routing (cheap/fast secondary model)
            print_step(5, total_steps, "Smart Routing");
            self.step_smart_routing().await?;
            self.persist_after_step().await;

            // Step 6: Fallback Providers (optional secondary providers)
            print_step(6, total_steps, "Fallback Providers");
            self.step_fallback_providers().await?;
            self.persist_after_step().await;

            // Step 7: Embeddings
            print_step(7, total_steps, "Embeddings (Semantic Search)");
            self.step_embeddings()?;
            self.persist_after_step().await;

            // ── Agent Personality ─────────────────────────────────────────

            // Step 8: Agent Identity
            print_step(8, total_steps, "Agent Identity");
            self.step_agent_identity()?;
            self.persist_after_step().await;

            // ── Communication Channels ───────────────────────────────────

            // Step 9: Channel configuration
            print_step(9, total_steps, "Channel Configuration");
            self.step_channels().await?;
            self.persist_after_step().await;

            // ── Capabilities & Execution ─────────────────────────────────

            // Step 10: Extensions (tools)
            print_step(10, total_steps, "Extensions");
            self.step_extensions().await?;
            self.persist_after_step().await;

            // Step 11: Local Tools & Docker Sandbox
            print_step(11, total_steps, "Local Tools & Docker Sandbox");
            self.step_docker_sandbox().await?;
            self.persist_after_step().await;

            // Step 12: Claude Code Sandbox
            print_step(12, total_steps, "Claude Code Sandbox");
            self.step_claude_code().await?;
            self.persist_after_step().await;

            // Step 13: Tool Approval Mode
            print_step(13, total_steps, "Tool Approval Mode");
            self.step_tool_approval()?;
            self.persist_after_step().await;

            // ── Automation ───────────────────────────────────────────────

            // Step 14: Routines
            print_step(14, total_steps, "Routines (Scheduled Tasks)");
            self.step_routines()?;
            self.persist_after_step().await;

            // Step 15: Skills
            print_step(15, total_steps, "Skills");
            self.step_skills()?;
            self.persist_after_step().await;

            // Step 16: Heartbeat
            print_step(16, total_steps, "Background Tasks");
            self.step_heartbeat()?;
            self.persist_after_step().await;

            // ── Presentation & Operations ────────────────────────────────

            // Step 17: Notification Preferences
            print_step(17, total_steps, "Notification Preferences");
            self.step_notification_preferences()?;
            self.persist_after_step().await;

            // Step 18: Web UI
            print_step(18, total_steps, "Web UI");
            self.step_web_ui()?;
            self.persist_after_step().await;

            // Step 19: Observability
            print_step(19, total_steps, "Observability");
            self.step_observability()?;
            self.persist_after_step().await;
        }

        // Save settings and print summary
        self.save_and_summarize().await?;

        Ok(())
    }

    /// Reconnect to the existing database and load settings.
    ///
    /// Used by channels-only mode (and future single-step modes) so that
    /// `init_secrets_context()` and `save_and_summarize()` have a live
    /// database connection and the wizard's `self.settings` reflects the
    /// previously saved configuration.
    async fn reconnect_existing_db(&mut self) -> Result<(), SetupError> {
        // Determine backend from env (set by bootstrap .env loaded in main).
        let backend = std::env::var("DATABASE_BACKEND").unwrap_or_else(|_| "postgres".to_string());

        // Try libsql first if that's the configured backend.
        #[cfg(feature = "libsql")]
        if backend == "libsql" || backend == "turso" || backend == "sqlite" {
            return self.reconnect_libsql().await;
        }

        // Try postgres (either explicitly configured or as default).
        #[cfg(feature = "postgres")]
        {
            let _ = &backend;
            return self.reconnect_postgres().await;
        }

        #[allow(unreachable_code)]
        Err(SetupError::Database(
            "No database configured. Run full setup first (thinclaw onboard).".to_string(),
        ))
    }

    /// Reconnect to an existing PostgreSQL database and load settings.
    #[cfg(feature = "postgres")]
    async fn reconnect_postgres(&mut self) -> Result<(), SetupError> {
        let url = std::env::var("DATABASE_URL").map_err(|_| {
            SetupError::Database(
                "DATABASE_URL not set. Run full setup first (thinclaw onboard).".to_string(),
            )
        })?;

        self.test_database_connection_postgres(&url).await?;
        self.settings.database_backend = Some("postgres".to_string());
        self.settings.database_url = Some(url.clone());

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref pool) = self.db_pool {
            let store = crate::history::Store::from_pool(pool.clone());
            if let Ok(map) = store.get_all_settings("default").await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("postgres".to_string());
                self.settings.database_url = Some(url);
            }
        }

        Ok(())
    }

    /// Reconnect to an existing libSQL database and load settings.
    #[cfg(feature = "libsql")]
    async fn reconnect_libsql(&mut self) -> Result<(), SetupError> {
        let path = std::env::var("LIBSQL_PATH").unwrap_or_else(|_| {
            crate::config::default_libsql_path()
                .to_string_lossy()
                .to_string()
        });
        let turso_url = std::env::var("LIBSQL_URL").ok();
        let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();

        self.test_database_connection_libsql(&path, turso_url.as_deref(), turso_token.as_deref())
            .await?;

        self.settings.database_backend = Some("libsql".to_string());
        self.settings.libsql_path = Some(path.clone());
        if let Some(ref url) = turso_url {
            self.settings.libsql_url = Some(url.clone());
        }

        // Load existing settings from DB, then restore connection fields that
        // may not be persisted in the settings map.
        if let Some(ref db) = self.db_backend {
            use crate::db::SettingsStore as _;
            if let Ok(map) = db.get_all_settings("default").await {
                self.settings = Settings::from_db_map(&map);
                self.settings.database_backend = Some("libsql".to_string());
                self.settings.libsql_path = Some(path);
                if let Some(url) = turso_url {
                    self.settings.libsql_url = Some(url);
                }
            }
        }

        Ok(())
    }
}

// Step implementations are split into sub-modules by concern.
mod infrastructure;
mod llm;
mod channels_step;
mod extensions;
mod agent;
mod sandbox;
mod automation;
mod presentation;
mod persistence;
mod summary;
pub(crate) mod helpers;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;

    use crate::channels::wasm::{ChannelCapabilitiesFile, available_channel_names};

    use super::*;
    use super::helpers::*;

    #[test]
    fn test_wizard_creation() {
        let wizard = SetupWizard::new();
        assert!(!wizard.config.skip_auth);
        assert!(!wizard.config.channels_only);
    }

    #[test]
    fn test_wizard_with_config() {
        let config = SetupConfig {
            skip_auth: true,
            channels_only: false,
        };
        let wizard = SetupWizard::with_config(config);
        assert!(wizard.config.skip_auth);
    }

    #[test]
    #[cfg(feature = "postgres")]
    fn test_mask_password_in_url() {
        assert_eq!(
            mask_password_in_url("postgres://user:secret@localhost/db"),
            "postgres://user:****@localhost/db"
        );

        // URL without password
        assert_eq!(
            mask_password_in_url("postgres://localhost/db"),
            "postgres://localhost/db"
        );
    }

    #[test]
    fn test_capitalize_first() {
        assert_eq!(capitalize_first("telegram"), "Telegram");
        assert_eq!(capitalize_first("CAPS"), "CAPS");
        assert_eq!(capitalize_first(""), "");
    }

    #[test]
    fn test_mask_api_key() {
        assert_eq!(
            mask_api_key("sk-ant-api03-abcdef1234567890"),
            "sk-ant...7890"
        );
        assert_eq!(mask_api_key("short"), "shor...");
        assert_eq!(mask_api_key("exactly12ch"), "exac...");
        assert_eq!(mask_api_key("exactly12chr"), "exactl...2chr");
        assert_eq!(mask_api_key(""), "...");
        // Multi-byte chars should not panic
        assert_eq!(mask_api_key("日本語キー"), "日本語キ...");
    }

    #[tokio::test]
    async fn test_install_missing_bundled_channels_installs_telegram() {
        // WASM artifacts only exist in dev builds (not CI). Skip gracefully
        // rather than fail when the telegram channel hasn't been compiled.
        if !available_channel_names().contains(&"telegram") {
            eprintln!("skipping: telegram WASM artifacts not built");
            return;
        }

        let dir = tempdir().unwrap();
        let installed = HashSet::<String>::new();

        install_missing_bundled_channels(dir.path(), &installed)
            .await
            .unwrap();

        assert!(dir.path().join("telegram.wasm").exists());
        assert!(dir.path().join("telegram.capabilities.json").exists());
    }

    #[test]
    fn test_build_channel_options_includes_available_when_missing() {
        let discovered = Vec::new();
        let options = build_channel_options(&discovered);
        let available = available_channel_names();
        // All available (built) channels should appear
        for name in &available {
            assert!(
                options.contains(&name.to_string()),
                "expected '{}' in options",
                name
            );
        }
    }

    #[test]
    fn test_build_channel_options_dedupes_available() {
        let discovered = vec![(String::from("telegram"), ChannelCapabilitiesFile::default())];
        let options = build_channel_options(&discovered);
        // telegram should appear exactly once despite being both discovered and available
        assert_eq!(
            options.iter().filter(|n| *n == "telegram").count(),
            1,
            "telegram should not be duplicated"
        );
    }

    #[tokio::test]
    async fn test_fetch_anthropic_models_static_fallback() {
        // With no API key, should return static defaults
        let _guard = EnvGuard::clear("ANTHROPIC_API_KEY");
        let models = fetch_anthropic_models(None).await;
        assert!(!models.is_empty());
        assert!(
            models.iter().any(|(id, _)| id.contains("claude")),
            "static defaults should include a Claude model"
        );
    }

    #[tokio::test]
    async fn test_fetch_openai_models_static_fallback() {
        let _guard = EnvGuard::clear("OPENAI_API_KEY");
        let models = fetch_openai_models(None).await;
        assert!(!models.is_empty());
        assert_eq!(models[0].0, "gpt-5.3-codex");
        assert!(
            models.iter().any(|(id, _)| id.contains("gpt")),
            "static defaults should include a GPT model"
        );
    }

    #[test]
    fn test_is_openai_chat_model_includes_gpt5_and_filters_non_chat_variants() {
        assert!(is_openai_chat_model("gpt-5"));
        assert!(is_openai_chat_model("gpt-5-mini-2026-01-01"));
        assert!(is_openai_chat_model("o3-2025-04-16"));
        assert!(!is_openai_chat_model("chatgpt-image-latest"));
        assert!(!is_openai_chat_model("gpt-4o-realtime-preview"));
        assert!(!is_openai_chat_model("gpt-4o-mini-transcribe"));
        assert!(!is_openai_chat_model("text-embedding-3-large"));
    }

    #[test]
    fn test_sort_openai_models_prioritizes_best_models_first() {
        let mut models = vec![
            ("gpt-4o-mini".to_string(), "gpt-4o-mini".to_string()),
            ("gpt-5-mini".to_string(), "gpt-5-mini".to_string()),
            ("o3".to_string(), "o3".to_string()),
            ("gpt-4.1".to_string(), "gpt-4.1".to_string()),
            ("gpt-5".to_string(), "gpt-5".to_string()),
        ];

        sort_openai_models(&mut models);

        let ordered: Vec<String> = models.into_iter().map(|(id, _)| id).collect();
        assert_eq!(
            ordered,
            vec![
                "gpt-5".to_string(),
                "gpt-5-mini".to_string(),
                "o3".to_string(),
                "gpt-4.1".to_string(),
                "gpt-4o-mini".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_fetch_ollama_models_unreachable_fallback() {
        // Point at a port nothing listens on
        let models = fetch_ollama_models("http://127.0.0.1:1").await;
        assert!(!models.is_empty(), "should fall back to static defaults");
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_empty_dir() {
        let dir = tempdir().unwrap();
        let channels = discover_wasm_channels(dir.path()).await;
        assert!(channels.is_empty());
    }

    #[tokio::test]
    async fn test_discover_wasm_channels_nonexistent_dir() {
        let channels =
            discover_wasm_channels(std::path::Path::new("/tmp/thinclaw_nonexistent_dir")).await;
        assert!(channels.is_empty());
    }

    /// RAII guard that sets/clears an env var for the duration of a test.
    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn clear(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                if let Some(ref val) = self.original {
                    std::env::set_var(self.key, val);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}
