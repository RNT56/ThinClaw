//! Main setup wizard orchestration.
//!
//! The wizard guides users through 18 steps, grouped by concern:
//!
//! **Infrastructure**
//! 1. Database connection
//! 2. Security (secrets master key)
//!
//! **LLM Configuration**
//! 3. Inference provider (Anthropic, OpenAI, Ollama, OpenRouter, OpenAI-compatible)
//! 4. Model selection
//! 5. Smart routing (cheap/fast secondary model)
//! 6. Embeddings (semantic search)
//!
//! **Agent Personality**
//! 7. Agent identity (name)
//!
//! **Communication Channels**
//! 8. Channel configuration (Telegram, Discord, Slack, Signal, etc.)
//!
//! **Capabilities & Execution**
//! 9. Extensions (tool installation from registry)
//! 10. Local tools & Docker sandbox
//! 11. Claude Code sandbox
//! 12. Tool approval mode
//!
//! **Automation**
//! 13. Routines (scheduled tasks)
//! 14. Skills (capability plugins)
//! 15. Heartbeat (background tasks)
//!
//! **Presentation & Operations**
//! 16. Notification preferences
//! 17. Web UI (theme, accent, branding)
//! 18. Observability (event/metric recording)

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(feature = "postgres")]
use deadpool_postgres::{Config as PoolConfig, Runtime};
use secrecy::{ExposeSecret, SecretString};
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;

use crate::channels::wasm::{
    ChannelCapabilitiesFile, available_channel_names, install_bundled_channel,
};

use crate::secrets::{SecretsCrypto, SecretsStore};
use crate::settings::{KeySource, Settings};
use crate::setup::channels::{
    SecretsContext, setup_http, setup_signal, setup_telegram, setup_tunnel, setup_wasm_channel,
};
use crate::setup::prompts::{
    confirm, input, optional_input, print_error, print_header, print_info, print_step,
    print_success, secret_input, select_many, select_one,
};

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
            let total_steps = 18;

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

            // Step 6: Embeddings
            print_step(6, total_steps, "Embeddings (Semantic Search)");
            self.step_embeddings()?;
            self.persist_after_step().await;

            // ── Agent Personality ─────────────────────────────────────────

            // Step 7: Agent Identity
            print_step(7, total_steps, "Agent Identity");
            self.step_agent_identity()?;
            self.persist_after_step().await;

            // ── Communication Channels ───────────────────────────────────

            // Step 8: Channel configuration
            print_step(8, total_steps, "Channel Configuration");
            self.step_channels().await?;
            self.persist_after_step().await;

            // ── Capabilities & Execution ─────────────────────────────────

            // Step 9: Extensions (tools)
            print_step(9, total_steps, "Extensions");
            self.step_extensions().await?;
            self.persist_after_step().await;

            // Step 10: Local Tools & Docker Sandbox
            print_step(10, total_steps, "Local Tools & Docker Sandbox");
            self.step_docker_sandbox().await?;
            self.persist_after_step().await;

            // Step 11: Claude Code Sandbox
            print_step(11, total_steps, "Claude Code Sandbox");
            self.step_claude_code().await?;
            self.persist_after_step().await;

            // Step 12: Tool Approval Mode
            print_step(12, total_steps, "Tool Approval Mode");
            self.step_tool_approval()?;
            self.persist_after_step().await;

            // ── Automation ───────────────────────────────────────────────

            // Step 13: Routines
            print_step(13, total_steps, "Routines (Scheduled Tasks)");
            self.step_routines()?;
            self.persist_after_step().await;

            // Step 14: Skills
            print_step(14, total_steps, "Skills");
            self.step_skills()?;
            self.persist_after_step().await;

            // Step 15: Heartbeat
            print_step(15, total_steps, "Background Tasks");
            self.step_heartbeat()?;
            self.persist_after_step().await;

            // ── Presentation & Operations ────────────────────────────────

            // Step 16: Notification Preferences
            print_step(16, total_steps, "Notification Preferences");
            self.step_notification_preferences()?;
            self.persist_after_step().await;

            // Step 17: Web UI
            print_step(17, total_steps, "Web UI");
            self.step_web_ui()?;
            self.persist_after_step().await;

            // Step 18: Observability
            print_step(18, total_steps, "Observability");
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

    /// Step 1: Database connection.
    async fn step_database(&mut self) -> Result<(), SetupError> {
        // When both features are compiled, let the user choose.
        // If DATABASE_BACKEND is already set in the environment, respect it.
        #[cfg(all(feature = "postgres", feature = "libsql"))]
        {
            // Check if a backend is already pinned via env var
            let env_backend = std::env::var("DATABASE_BACKEND").ok();

            if let Some(ref backend) = env_backend {
                if backend == "libsql" || backend == "turso" || backend == "sqlite" {
                    return self.step_database_libsql().await;
                }
                if backend != "postgres" && backend != "postgresql" {
                    print_info(&format!(
                        "Unknown DATABASE_BACKEND '{}', defaulting to PostgreSQL",
                        backend
                    ));
                }
                return self.step_database_postgres().await;
            }

            // Interactive selection
            let pre_selected = self.settings.database_backend.as_deref().map(|b| match b {
                "libsql" | "turso" | "sqlite" => 1,
                _ => 0,
            });

            print_info("Which database backend would you like to use?");
            println!();

            let options = &[
                "PostgreSQL  - production-grade, requires a running server",
                "libSQL      - embedded SQLite, zero dependencies, optional Turso cloud sync",
            ];
            let choice =
                select_one("Select a database backend:", options).map_err(SetupError::Io)?;

            // If the user picked something different from what was pre-selected, clear
            // stale connection settings so the next step starts fresh.
            if let Some(prev) = pre_selected
                && prev != choice
            {
                self.settings.database_url = None;
                self.settings.libsql_path = None;
                self.settings.libsql_url = None;
            }

            match choice {
                1 => return self.step_database_libsql().await,
                _ => return self.step_database_postgres().await,
            }
        }

        #[cfg(all(feature = "postgres", not(feature = "libsql")))]
        {
            return self.step_database_postgres().await;
        }

        #[cfg(all(feature = "libsql", not(feature = "postgres")))]
        {
            return self.step_database_libsql().await;
        }
    }

    /// Step 1 (postgres): Database connection via PostgreSQL URL.
    #[cfg(feature = "postgres")]
    async fn step_database_postgres(&mut self) -> Result<(), SetupError> {
        self.settings.database_backend = Some("postgres".to_string());

        let existing_url = std::env::var("DATABASE_URL")
            .ok()
            .or_else(|| self.settings.database_url.clone());

        if let Some(ref url) = existing_url {
            let display_url = mask_password_in_url(url);
            print_info(&format!("Existing database URL: {}", display_url));

            if confirm("Use this database?", true).map_err(SetupError::Io)? {
                if let Err(e) = self.test_database_connection_postgres(url).await {
                    print_error(&format!("Connection failed: {}", e));
                    print_info("Let's configure a new database URL.");
                } else {
                    print_success("Database connection successful");
                    // Run migrations to ensure new tables exist on older schemas
                    self.run_migrations_postgres().await?;
                    self.settings.database_url = Some(url.clone());
                    return Ok(());
                }
            }
        }

        println!();
        print_info("Enter your PostgreSQL connection URL.");
        print_info("Format: postgres://user:password@host:port/database");
        println!();

        loop {
            let url = input("Database URL").map_err(SetupError::Io)?;

            if url.is_empty() {
                print_error("Database URL is required.");
                continue;
            }

            print_info("Testing connection...");
            match self.test_database_connection_postgres(&url).await {
                Ok(()) => {
                    print_success("Database connection successful");

                    if confirm("Run database migrations?", true).map_err(SetupError::Io)? {
                        self.run_migrations_postgres().await?;
                    }

                    self.settings.database_url = Some(url);
                    return Ok(());
                }
                Err(e) => {
                    print_error(&format!("Connection failed: {}", e));
                    if !confirm("Try again?", true).map_err(SetupError::Io)? {
                        return Err(SetupError::Database(
                            "Database connection failed".to_string(),
                        ));
                    }
                }
            }
        }
    }

    /// Step 1 (libsql): Database connection via local file or Turso remote replica.
    #[cfg(feature = "libsql")]
    async fn step_database_libsql(&mut self) -> Result<(), SetupError> {
        self.settings.database_backend = Some("libsql".to_string());

        let default_path = crate::config::default_libsql_path();
        let default_path_str = default_path.to_string_lossy().to_string();

        // Check for existing configuration
        let existing_path = std::env::var("LIBSQL_PATH")
            .ok()
            .or_else(|| self.settings.libsql_path.clone());

        if let Some(ref path) = existing_path {
            print_info(&format!("Existing database path: {}", path));
            if confirm("Use this database?", true).map_err(SetupError::Io)? {
                let turso_url = std::env::var("LIBSQL_URL")
                    .ok()
                    .or_else(|| self.settings.libsql_url.clone());
                let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();

                match self
                    .test_database_connection_libsql(
                        path,
                        turso_url.as_deref(),
                        turso_token.as_deref(),
                    )
                    .await
                {
                    Ok(()) => {
                        print_success("Database connection successful");

                        // Always run migrations — they're idempotent (IF NOT EXISTS)
                        // and ensure new tables (e.g. secrets, routines) exist on
                        // databases created with older schema versions.
                        self.run_migrations_libsql().await?;

                        self.settings.libsql_path = Some(path.clone());
                        if let Some(url) = turso_url {
                            self.settings.libsql_url = Some(url);
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        print_error(&format!("Connection failed: {}", e));
                        print_info("Let's configure a new database path.");
                    }
                }
            }
        }

        println!();
        print_info("ThinClaw uses an embedded SQLite database (libSQL).");
        print_info("No external database server required.");
        println!();

        let path_input = optional_input(
            "Database file path",
            Some(&format!("default: {}", default_path_str)),
        )
        .map_err(SetupError::Io)?;

        let db_path = path_input.unwrap_or(default_path_str.clone());

        // Ask about Turso cloud sync
        println!();
        let use_turso =
            confirm("Enable Turso cloud sync (remote replica)?", false).map_err(SetupError::Io)?;

        let (turso_url, turso_token) = if use_turso {
            print_info("Enter your Turso database URL and auth token.");
            print_info("Format: libsql://your-db.turso.io");
            println!();

            let url = input("Turso URL").map_err(SetupError::Io)?;
            if url.is_empty() {
                print_error("Turso URL is required for cloud sync.");
                (None, None)
            } else {
                let token_secret = secret_input("Auth token").map_err(SetupError::Io)?;
                let token = token_secret.expose_secret().to_string();
                if token.is_empty() {
                    print_error("Auth token is required for cloud sync.");
                    (None, None)
                } else {
                    (Some(url), Some(token))
                }
            }
        } else {
            (None, None)
        };

        print_info("Testing connection...");
        match self
            .test_database_connection_libsql(&db_path, turso_url.as_deref(), turso_token.as_deref())
            .await
        {
            Ok(()) => {
                print_success("Database connection successful");

                // Always run migrations for libsql (they're idempotent)
                self.run_migrations_libsql().await?;

                self.settings.libsql_path = Some(db_path);
                if let Some(url) = turso_url {
                    self.settings.libsql_url = Some(url);
                }
                Ok(())
            }
            Err(e) => Err(SetupError::Database(format!("Connection failed: {}", e))),
        }
    }

    /// Test PostgreSQL connection and store the pool.
    #[cfg(feature = "postgres")]
    async fn test_database_connection_postgres(&mut self, url: &str) -> Result<(), SetupError> {
        let mut cfg = PoolConfig::new();
        cfg.url = Some(url.to_string());
        cfg.pool = Some(deadpool_postgres::PoolConfig {
            max_size: 5,
            ..Default::default()
        });

        let pool = cfg
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .map_err(|e| SetupError::Database(format!("Failed to create pool: {}", e)))?;

        let _ = pool
            .get()
            .await
            .map_err(|e| SetupError::Database(format!("Failed to connect: {}", e)))?;

        self.db_pool = Some(pool);
        Ok(())
    }

    /// Test libSQL connection and store the backend.
    #[cfg(feature = "libsql")]
    async fn test_database_connection_libsql(
        &mut self,
        path: &str,
        turso_url: Option<&str>,
        turso_token: Option<&str>,
    ) -> Result<(), SetupError> {
        use crate::db::libsql::LibSqlBackend;
        use std::path::Path;

        let db_path = Path::new(path);

        let backend = if let (Some(url), Some(token)) = (turso_url, turso_token) {
            LibSqlBackend::new_remote_replica(db_path, url, token)
                .await
                .map_err(|e| SetupError::Database(format!("Failed to connect: {}", e)))?
        } else {
            LibSqlBackend::new_local(db_path)
                .await
                .map_err(|e| SetupError::Database(format!("Failed to open database: {}", e)))?
        };

        self.db_backend = Some(backend);
        Ok(())
    }

    /// Run PostgreSQL migrations.
    #[cfg(feature = "postgres")]
    async fn run_migrations_postgres(&self) -> Result<(), SetupError> {
        if let Some(ref pool) = self.db_pool {
            use refinery::embed_migrations;
            embed_migrations!("migrations");

            print_info("Running migrations...");

            let mut client = pool
                .get()
                .await
                .map_err(|e| SetupError::Database(format!("Pool error: {}", e)))?;

            migrations::runner()
                .run_async(&mut **client)
                .await
                .map_err(|e| SetupError::Database(format!("Migration failed: {}", e)))?;

            print_success("Migrations applied");
        }
        Ok(())
    }

    /// Run libSQL migrations.
    #[cfg(feature = "libsql")]
    async fn run_migrations_libsql(&self) -> Result<(), SetupError> {
        if let Some(ref backend) = self.db_backend {
            use crate::db::Database;

            print_info("Running migrations...");

            backend
                .run_migrations()
                .await
                .map_err(|e| SetupError::Database(format!("Migration failed: {}", e)))?;

            print_success("Migrations applied");
        }
        Ok(())
    }

    /// Step 2: Security (secrets master key).
    async fn step_security(&mut self) -> Result<(), SetupError> {
        // Check current configuration
        let env_key_exists = std::env::var("SECRETS_MASTER_KEY").is_ok();

        if env_key_exists {
            print_info("Secrets master key found in SECRETS_MASTER_KEY environment variable.");
            self.settings.secrets_master_key_source = KeySource::Env;
            print_success("Security configured (env var)");
            return Ok(());
        }

        // Try to retrieve existing key from keychain. We use get_master_key()
        // instead of has_master_key() so we can cache the key bytes and build
        // SecretsCrypto eagerly, avoiding redundant keychain accesses later
        // (each access triggers macOS system dialogs).
        print_info("Checking OS keychain for existing master key...");
        if let Ok(keychain_key_bytes) = crate::secrets::keychain::get_master_key().await {
            let key_hex: String = keychain_key_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            self.secrets_crypto = Some(Arc::new(
                SecretsCrypto::new(SecretString::from(key_hex))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            ));

            print_info("Existing master key found in OS keychain.");
            if confirm("Use existing keychain key?", true).map_err(SetupError::Io)? {
                self.settings.secrets_master_key_source = KeySource::Keychain;
                print_success("Security configured (keychain)");
                return Ok(());
            }
            // User declined the existing key; clear the cached crypto so a fresh
            // key can be generated below.
            self.secrets_crypto = None;
        }

        // Offer options
        println!();
        print_info("The secrets master key encrypts sensitive data like API tokens.");
        print_info("Choose where to store it:");
        println!();

        let options = [
            "OS Keychain (recommended for local installs)",
            "Environment variable (for CI/Docker)",
            "Skip (disable secrets features)",
        ];

        let choice = select_one("Select storage method:", &options).map_err(SetupError::Io)?;

        match choice {
            0 => {
                // Generate and store in keychain
                print_info("Generating master key...");
                let key = crate::secrets::keychain::generate_master_key();

                crate::secrets::keychain::store_master_key(&key)
                    .await
                    .map_err(|e| {
                        SetupError::Config(format!("Failed to store in keychain: {}", e))
                    })?;

                // Also create crypto instance
                let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
                self.secrets_crypto = Some(Arc::new(
                    SecretsCrypto::new(SecretString::from(key_hex))
                        .map_err(|e| SetupError::Config(e.to_string()))?,
                ));

                self.settings.secrets_master_key_source = KeySource::Keychain;
                print_success("Master key generated and stored in OS keychain");
            }
            1 => {
                // Env var mode
                print_info("Generate a key and add it to your environment:");
                let key_hex = crate::secrets::keychain::generate_master_key_hex();
                println!();
                println!("  export SECRETS_MASTER_KEY={}", key_hex);
                println!();
                print_info("Add this to your shell profile or .env file.");

                self.settings.secrets_master_key_source = KeySource::Env;
                print_success("Configured for environment variable");
            }
            _ => {
                self.settings.secrets_master_key_source = KeySource::None;
                print_info("Secrets features disabled. Channel tokens must be set via env vars.");
            }
        }

        Ok(())
    }

    /// Step 3: Inference provider selection.
    ///
    /// Lets the user pick from all supported LLM backends, then runs the
    /// provider-specific auth sub-flow (API key entry, NEAR AI login, etc.).
    async fn step_inference_provider(&mut self) -> Result<(), SetupError> {
        // Show current provider if already configured
        if let Some(ref current) = self.settings.llm_backend {
            let is_openrouter = current == "openai_compatible"
                && self
                    .settings
                    .openai_compatible_base_url
                    .as_deref()
                    .is_some_and(|u| u.contains("openrouter.ai"));

            let display = if is_openrouter {
                "OpenRouter"
            } else {
                match current.as_str() {
                    "anthropic" => "Anthropic (Claude)",
                    "openai" => "OpenAI",
                    "ollama" => "Ollama (local)",
                    "openai_compatible" => "OpenAI-compatible endpoint",
                    other => other,
                }
            };
            print_info(&format!("Current provider: {}", display));
            println!();

            let is_known = matches!(
                current.as_str(),
                "anthropic" | "openai" | "ollama" | "openai_compatible"
            );

            if is_known && confirm("Keep current provider?", true).map_err(SetupError::Io)? {
                // Still run the auth sub-flow in case they need to update keys
                if is_openrouter {
                    return self.setup_openrouter().await;
                }
                match current.as_str() {
                    "anthropic" => return self.setup_anthropic().await,
                    "openai" => return self.setup_openai().await,
                    "ollama" => return self.setup_ollama(),
                    "openai_compatible" => return self.setup_openai_compatible().await,
                    _ => {
                        return Err(SetupError::Config(format!(
                            "Unhandled provider: {}",
                            current
                        )));
                    }
                }
            }

            if !is_known {
                print_info(&format!(
                    "Unknown provider '{}', please select a supported provider.",
                    current
                ));
            }
        }

        print_info("Select your inference provider:");
        println!();

        let options = &[
            "Anthropic        - Claude models (direct API key)",
            "OpenAI           - GPT models (direct API key)",
            "Ollama           - local models, no API key needed",
            "OpenRouter       - 200+ models via single API key",
            "OpenAI-compatible - custom endpoint (vLLM, LiteLLM, etc.)",
        ];

        let choice = select_one("Provider:", options).map_err(SetupError::Io)?;

        match choice {
            0 => self.setup_anthropic().await?,
            1 => self.setup_openai().await?,
            2 => self.setup_ollama()?,
            3 => self.setup_openrouter().await?,
            4 => self.setup_openai_compatible().await?,
            _ => return Err(SetupError::Config("Invalid provider selection".to_string())),
        }

        Ok(())
    }

    /// Anthropic provider setup: collect API key and store in secrets.
    async fn setup_anthropic(&mut self) -> Result<(), SetupError> {
        self.setup_api_key_provider(
            "anthropic",
            "ANTHROPIC_API_KEY",
            "llm_anthropic_api_key",
            "Anthropic API key",
            "https://console.anthropic.com/settings/keys",
            None,
        )
        .await
    }

    /// OpenAI provider setup: collect API key and store in secrets.
    async fn setup_openai(&mut self) -> Result<(), SetupError> {
        self.setup_api_key_provider(
            "openai",
            "OPENAI_API_KEY",
            "llm_openai_api_key",
            "OpenAI API key",
            "https://platform.openai.com/api-keys",
            None,
        )
        .await
    }

    /// Shared setup flow for API-key-based providers (Anthropic, OpenAI, OpenRouter).
    async fn setup_api_key_provider(
        &mut self,
        backend: &str,
        env_var: &str,
        secret_name: &str,
        prompt_label: &str,
        hint_url: &str,
        override_display_name: Option<&str>,
    ) -> Result<(), SetupError> {
        let display_name = override_display_name.unwrap_or(match backend {
            "anthropic" => "Anthropic",
            "openai" => "OpenAI",
            other => other,
        });

        self.settings.llm_backend = Some(backend.to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        // Check env var first
        if let Ok(existing) = std::env::var(env_var) {
            print_info(&format!("{env_var} found: {}", mask_api_key(&existing)));
            if confirm("Use this key?", true).map_err(SetupError::Io)? {
                // Persist env-provided key to secrets store for future runs
                if let Ok(ctx) = self.init_secrets_context().await {
                    let key = SecretString::from(existing.clone());
                    if let Err(e) = ctx.save_secret(secret_name, &key).await {
                        tracing::warn!("Failed to persist env key to secrets: {}", e);
                    }
                }
                self.llm_api_key = Some(SecretString::from(existing));
                print_success(&format!("{display_name} configured (from env)"));
                return Ok(());
            }
        }

        println!();
        print_info(&format!("Get your API key from: {hint_url}"));
        println!();

        let key = secret_input(prompt_label).map_err(SetupError::Io)?;
        let key_str = key.expose_secret();

        if key_str.is_empty() {
            return Err(SetupError::Config("API key cannot be empty".to_string()));
        }

        // Store in secrets if available
        if let Ok(ctx) = self.init_secrets_context().await {
            ctx.save_secret(secret_name, &key)
                .await
                .map_err(|e| SetupError::Config(format!("Failed to save API key: {e}")))?;
            print_success("API key encrypted and saved");
        } else {
            print_info(&format!(
                "Secrets not available. Set {env_var} in your environment."
            ));
        }

        // Cache key in memory for model fetching later in the wizard
        self.llm_api_key = Some(SecretString::from(key_str.to_string()));

        print_success(&format!("{display_name} configured"));
        Ok(())
    }

    /// Ollama provider setup: just needs a base URL, no API key.
    fn setup_ollama(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("ollama".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        let default_url = self
            .settings
            .ollama_base_url
            .as_deref()
            .unwrap_or("http://localhost:11434");

        let url_input = optional_input(
            "Ollama base URL",
            Some(&format!("default: {}", default_url)),
        )
        .map_err(SetupError::Io)?;

        let url = url_input.unwrap_or_else(|| default_url.to_string());
        self.settings.ollama_base_url = Some(url.clone());

        print_success(&format!("Ollama configured ({})", url));
        Ok(())
    }

    /// OpenRouter provider setup: pre-configured OpenAI-compatible endpoint.
    ///
    /// Sets the base URL to `https://openrouter.ai/api/v1` and delegates
    /// API key collection to `setup_api_key_provider` with a display name
    /// override so messages say "OpenRouter" instead of "openai_compatible".
    async fn setup_openrouter(&mut self) -> Result<(), SetupError> {
        self.settings.openai_compatible_base_url = Some("https://openrouter.ai/api/v1".to_string());
        self.setup_api_key_provider(
            "openai_compatible",
            "LLM_API_KEY",
            "llm_compatible_api_key",
            "OpenRouter API key",
            "https://openrouter.ai/settings/keys",
            Some("OpenRouter"),
        )
        .await
    }

    /// OpenAI-compatible provider setup: base URL + optional API key.
    async fn setup_openai_compatible(&mut self) -> Result<(), SetupError> {
        self.settings.llm_backend = Some("openai_compatible".to_string());
        if self.settings.selected_model.is_some() {
            self.settings.selected_model = None;
        }

        let existing_url = self
            .settings
            .openai_compatible_base_url
            .clone()
            .or_else(|| std::env::var("LLM_BASE_URL").ok());

        let url = if let Some(ref u) = existing_url {
            let url_input = optional_input("Base URL", Some(&format!("current: {}", u)))
                .map_err(SetupError::Io)?;
            url_input.unwrap_or_else(|| u.clone())
        } else {
            input("Base URL (e.g., http://localhost:8000/v1)").map_err(SetupError::Io)?
        };

        if url.is_empty() {
            return Err(SetupError::Config(
                "Base URL is required for OpenAI-compatible provider".to_string(),
            ));
        }

        self.settings.openai_compatible_base_url = Some(url.clone());

        // Optional API key
        if confirm("Does this endpoint require an API key?", false).map_err(SetupError::Io)? {
            let key = secret_input("API key").map_err(SetupError::Io)?;
            let key_str = key.expose_secret();

            if !key_str.is_empty() {
                if let Ok(ctx) = self.init_secrets_context().await {
                    ctx.save_secret("llm_compatible_api_key", &key)
                        .await
                        .map_err(|e| {
                            SetupError::Config(format!("Failed to save API key: {}", e))
                        })?;
                    print_success("API key encrypted and saved");
                } else {
                    print_info("Secrets not available. Set LLM_API_KEY in your environment.");
                }
            }
        }

        print_success(&format!("OpenAI-compatible configured ({})", url));
        Ok(())
    }

    /// Step 4: Model selection.
    ///
    /// Branches on the selected LLM backend and fetches models from the
    /// appropriate provider API, with static defaults as fallback.
    async fn step_model_selection(&mut self) -> Result<(), SetupError> {
        // Show current model if already configured
        if let Some(ref current) = self.settings.selected_model {
            print_info(&format!("Current model: {}", current));
            println!();

            let options = ["Keep current model", "Change model"];
            let choice =
                select_one("What would you like to do?", &options).map_err(SetupError::Io)?;

            if choice == 0 {
                print_success(&format!("Keeping {}", current));
                return Ok(());
            }
        }

        let backend = self
            .settings
            .llm_backend
            .as_deref()
            .unwrap_or("openai_compatible");

        match backend {
            "anthropic" => {
                let cached = self
                    .llm_api_key
                    .as_ref()
                    .map(|k| k.expose_secret().to_string());
                let models = fetch_anthropic_models(cached.as_deref()).await;
                self.select_from_model_list(&models)?;
            }
            "openai" => {
                let cached = self
                    .llm_api_key
                    .as_ref()
                    .map(|k| k.expose_secret().to_string());
                let models = fetch_openai_models(cached.as_deref()).await;
                self.select_from_model_list(&models)?;
            }
            "ollama" => {
                let base_url = self
                    .settings
                    .ollama_base_url
                    .as_deref()
                    .unwrap_or("http://localhost:11434");
                let models = fetch_ollama_models(base_url).await;
                if models.is_empty() {
                    print_info("No models found. Pull one first: ollama pull llama3");
                }
                self.select_from_model_list(&models)?;
            }
            "openai_compatible" => {
                // No standard API for listing models on arbitrary endpoints
                let model_id = input("Model name (e.g., meta-llama/Llama-3-8b-chat-hf)")
                    .map_err(SetupError::Io)?;
                if model_id.is_empty() {
                    return Err(SetupError::Config("Model name is required".to_string()));
                }
                self.settings.selected_model = Some(model_id.clone());
                print_success(&format!("Selected {}", model_id));
            }
            _ => {
                // Generic fallback: ask for model name manually
                let model_id = input("Model name (e.g., meta-llama/Llama-3-8b-chat-hf)")
                    .map_err(SetupError::Io)?;
                if model_id.is_empty() {
                    return Err(SetupError::Config("Model name is required".to_string()));
                }
                self.settings.selected_model = Some(model_id.clone());
                print_success(&format!("Selected {}", model_id));
            }
        }

        Ok(())
    }

    /// Present a model list to the user, with a "Custom model ID" escape hatch.
    ///
    /// Each entry is `(model_id, display_label)`.
    fn select_from_model_list(&mut self, models: &[(String, String)]) -> Result<(), SetupError> {
        println!("Available models:");
        println!();

        let mut options: Vec<&str> = models.iter().map(|(_, desc)| desc.as_str()).collect();
        options.push("Custom model ID");

        let choice = select_one("Select a model:", &options).map_err(SetupError::Io)?;

        let selected = if choice == options.len() - 1 {
            loop {
                let raw = input("Enter model ID").map_err(SetupError::Io)?;
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    println!("Model ID cannot be empty.");
                    continue;
                }
                break trimmed;
            }
        } else {
            models[choice].0.clone()
        };

        self.settings.selected_model = Some(selected.clone());
        print_success(&format!("Selected {}", selected));
        Ok(())
    }

    /// Step 6: Embeddings configuration.
    fn step_embeddings(&mut self) -> Result<(), SetupError> {
        print_info("Embeddings enable semantic search in your workspace memory.");
        println!();

        if !confirm("Enable semantic search?", true).map_err(SetupError::Io)? {
            self.settings.embeddings.enabled = false;
            print_info("Embeddings disabled. Workspace will use keyword search only.");
            return Ok(());
        }

        let backend = self
            .settings
            .llm_backend
            .as_deref()
            .unwrap_or("openai_compatible");
        let has_openai_key = std::env::var("OPENAI_API_KEY").is_ok()
            || (backend == "openai" && self.llm_api_key.is_some());

        // If the LLM backend is OpenAI and we already have a key, default to OpenAI embeddings
        if backend == "openai" && has_openai_key {
            self.settings.embeddings.enabled = true;
            self.settings.embeddings.provider = "openai".to_string();
            self.settings.embeddings.model = "text-embedding-3-small".to_string();
            print_success("Embeddings enabled via OpenAI (using existing API key)");
            return Ok(());
        }

        if !has_openai_key {
            print_info("No OPENAI_API_KEY found for embeddings.");
            print_info("Set OPENAI_API_KEY in your environment to enable embeddings.");
            self.settings.embeddings.enabled = false;
            return Ok(());
        }

        let options = &["OpenAI (requires API key)", "Ollama (local, no API key)"];

        let choice = select_one("Select embeddings provider:", options).map_err(SetupError::Io)?;

        match choice {
            1 => {
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "ollama".to_string();
                self.settings.embeddings.model = "nomic-embed-text".to_string();
                print_success("Embeddings enabled via Ollama");
            }
            _ => {
                if !has_openai_key {
                    print_info("OPENAI_API_KEY not set in environment.");
                    print_info("Add it to your .env file or environment to enable embeddings.");
                }
                self.settings.embeddings.enabled = true;
                self.settings.embeddings.provider = "openai".to_string();
                self.settings.embeddings.model = "text-embedding-3-small".to_string();
                print_success("Embeddings configured for OpenAI");
            }
        }

        Ok(())
    }

    /// Initialize secrets context for channel setup.
    async fn init_secrets_context(&mut self) -> Result<SecretsContext, SetupError> {
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
    async fn create_postgres_secrets_store(
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
    fn create_libsql_secrets_store(
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
    async fn step_channels(&mut self) -> Result<(), SetupError> {
        // First, configure tunnel (shared across all channels that need webhooks)
        match setup_tunnel(&self.settings) {
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

        // Determine if we need secrets context
        let needs_secrets = selected.contains(&CHANNEL_INDEX_HTTP)
            || selected.contains(&CHANNEL_INDEX_DISCORD)
            || selected.contains(&CHANNEL_INDEX_SLACK)
            || !selected_wasm_channels.is_empty();
        let secrets = if needs_secrets {
            match self.init_secrets_context().await {
                Ok(ctx) => Some(ctx),
                Err(e) => {
                    print_info(&format!("Secrets not available: {}", e));
                    print_info("Channel tokens must be set via environment variables.");
                    None
                }
            }
        } else {
            None
        };

        // HTTP channel
        if selected.contains(&CHANNEL_INDEX_HTTP) {
            println!();
            if let Some(ref ctx) = secrets {
                let result = setup_http(ctx).await?;
                self.settings.channels.http_enabled = result.enabled;
                self.settings.channels.http_port = Some(result.port);
            } else {
                self.settings.channels.http_enabled = true;
                self.settings.channels.http_port = Some(8080);
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
            self.settings.channels.discord_bot_token = Some(token);

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
            self.settings.channels.slack_bot_token = Some(bot_token);
            self.settings.channels.slack_app_token = Some(app_token);

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
            print_info("⚠️  IMPORTANT: If you leave this blank, ANY email sender can give instructions to your agent.");
            print_info("   For security, specify your email address(es) so only you can control it via email.");
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

            let poll_interval =
                optional_input("Polling interval in seconds", Some("10")).map_err(SetupError::Io)?;
            if let Some(ref pi) = poll_interval {
                if let Ok(n) = pi.parse::<u64>() {
                    self.settings.channels.apple_mail_poll_interval = Some(n);
                }
            }

            let unread_only = confirm("Only process unread messages?", true)
                .map_err(SetupError::Io)?;
            self.settings.channels.apple_mail_unread_only = unread_only;

            let mark_as_read = confirm("Mark messages as read after processing?", true)
                .map_err(SetupError::Io)?;
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

    /// Step 9: Extensions (tools) installation from registry.
    async fn step_extensions(&mut self) -> Result<(), SetupError> {
        let catalog = match load_registry_catalog() {
            Some(c) => c,
            None => {
                print_info("Extension registry not found. Skipping tool installation.");
                print_info("Install tools manually with: thinclaw tool install <path>");
                return Ok(());
            }
        };

        let tools: Vec<_> = catalog
            .list(Some(crate::registry::manifest::ManifestKind::Tool), None)
            .into_iter()
            .cloned()
            .collect();

        if tools.is_empty() {
            print_info("No tools found in registry.");
            return Ok(());
        }

        print_info("Available tools from the extension registry:");
        print_info("Select which tools to install. You can install more later with:");
        print_info("  thinclaw registry install <name>");
        println!();

        // Check which tools are already installed
        let tools_dir = dirs::home_dir()
            .ok_or_else(|| SetupError::Config("Could not determine home directory".into()))?
            .join(".thinclaw/tools");

        let installed_tools = discover_installed_tools(&tools_dir).await;

        // Build options: show display_name + description, pre-check "default" tagged + already installed
        let mut options: Vec<(String, bool)> = Vec::new();
        for tool in &tools {
            let is_installed = installed_tools.contains(&tool.name);
            let is_default = tool.tags.contains(&"default".to_string());
            let status = if is_installed { " (installed)" } else { "" };
            let auth_hint = tool
                .auth_summary
                .as_ref()
                .and_then(|a| a.method.as_deref())
                .map(|m| format!(" [{}]", m))
                .unwrap_or_default();

            let label = format!(
                "{}{}{} - {}",
                tool.display_name, auth_hint, status, tool.description
            );
            options.push((label, is_default || is_installed));
        }

        let options_refs: Vec<(&str, bool)> =
            options.iter().map(|(s, b)| (s.as_str(), *b)).collect();

        let selected = select_many("Which tools do you want to install?", &options_refs)
            .map_err(SetupError::Io)?;

        if selected.is_empty() {
            print_info("No tools selected.");
            return Ok(());
        }

        // Install selected tools that aren't already on disk
        let repo_root = catalog.root().parent().unwrap_or(catalog.root());
        let installer = crate::registry::installer::RegistryInstaller::new(
            repo_root.to_path_buf(),
            tools_dir.clone(),
            dirs::home_dir()
                .unwrap_or_default()
                .join(".thinclaw/channels"),
        );

        let mut installed_count = 0;
        let mut auth_needed: Vec<String> = Vec::new();

        for idx in &selected {
            let tool = &tools[*idx];
            if installed_tools.contains(&tool.name) {
                continue; // Already installed, skip
            }

            // Priority 1: Extract from binary-embedded WASM (--features bundled-wasm)
            if crate::registry::bundled_wasm::is_bundled(&tool.name) {
                match crate::registry::bundled_wasm::extract_bundled(&tool.name, &tools_dir).await {
                    Ok(()) => {
                        print_success(&format!(
                            "Installed {} (from bundled binary)",
                            tool.display_name
                        ));
                        installed_count += 1;

                        // Track auth needs
                        if let Some(auth) = &tool.auth_summary
                            && auth.method.as_deref() != Some("none")
                            && auth.method.is_some()
                        {
                            let provider = auth.provider.as_deref().unwrap_or(&tool.name);
                            let hint = format!("  {} - thinclaw tool auth {}", provider, tool.name);
                            if !auth_needed
                                .iter()
                                .any(|h| h.starts_with(&format!("  {} -", provider)))
                            {
                                auth_needed.push(hint);
                            }
                        }
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!(
                            tool = %tool.name,
                            error = %e,
                            "Bundled WASM extraction failed, trying registry install"
                        );
                    }
                }
            }

            // Priority 2: Registry install (download artifact or build from source)
            match installer.install_with_source_fallback(tool, false).await {
                Ok(outcome) => {
                    print_success(&format!("Installed {}", outcome.name));
                    for warning in &outcome.warnings {
                        print_info(&format!("{}: {}", outcome.name, warning));
                    }
                    installed_count += 1;

                    // Track auth needs
                    if let Some(auth) = &tool.auth_summary
                        && auth.method.as_deref() != Some("none")
                        && auth.method.is_some()
                    {
                        let provider = auth.provider.as_deref().unwrap_or(&tool.name);
                        // Only mention unique providers (Google tools share auth)
                        let hint = format!("  {} - thinclaw tool auth {}", provider, tool.name);
                        if !auth_needed
                            .iter()
                            .any(|h| h.starts_with(&format!("  {} -", provider)))
                        {
                            auth_needed.push(hint);
                        }
                    }
                }
                Err(e) => {
                    print_error(&format!("Failed to install {}: {}", tool.display_name, e));
                }
            }
        }

        if installed_count > 0 {
            println!();
            print_success(&format!("{} tool(s) installed.", installed_count));
        }

        if !auth_needed.is_empty() {
            println!();
            print_info("Some tools need authentication. Run after setup:");
            for hint in &auth_needed {
                print_info(hint);
            }
        }

        Ok(())
    }

    /// Step 10: Docker Sandbox -- check Docker installation and availability.
    async fn step_docker_sandbox(&mut self) -> Result<(), SetupError> {
        // ── Part A: Local tools for the main agent ───────────────────────
        println!();
        print_info("═══ Main Agent: Local Tools ═══");
        println!();
        print_info("ThinClaw's main agent always runs natively on your machine.");
        print_info("Enabling local tools gives the agent full access to:");
        print_info("  • Shell commands (run scripts, install packages, etc.)");
        print_info("  • File read/write anywhere on disk");
        print_info("  • Screen capture (if enabled separately)");
        println!();
        print_info("Without local tools, the agent can only use web search, memory,");
        print_info("and WASM-sandboxed extensions. No direct host access.");
        println!();

        let allow_local = confirm(
            "Allow ThinClaw to use local tools on your machine?",
            false,
        )
        .map_err(SetupError::Io)?;
        self.settings.agent.allow_local_tools = allow_local;

        if allow_local {
            print_success("Local tools enabled. The agent can run commands and access files.");
            print_info("You can disable this later with ALLOW_LOCAL_TOOLS=false.");
        } else {
            print_info("Local tools disabled. The agent will use sandboxed tools only.");
            print_info("Enable later with ALLOW_LOCAL_TOOLS=true.");
        }

        // ── Part B: Docker sandbox for worker processes ──────────────────
        println!();
        print_info("═══ Docker Sandbox (Worker Processes) ═══");
        println!();
        print_info("Docker sandboxing is separate from local tools above.");
        print_info("It isolates *worker processes* like Claude Code — they run inside");
        print_info("Docker containers with no access to your credentials or full filesystem.");
        println!();
        print_info("This does NOT affect ThinClaw's main agent. The main agent always");
        print_info("runs natively, governed by the 'local tools' setting above.");
        println!();
        print_info("Docker is required for: Claude Code sandbox, container-based builds.");
        println!();

        if !confirm("Enable Docker sandbox for worker processes?", false).map_err(SetupError::Io)? {
            self.settings.sandbox.enabled = false;
            print_info("Docker sandbox disabled. Worker processes will not use containers.");
            print_info("You can enable it later with SANDBOX_ENABLED=true.");
            return Ok(());
        }

        // Check Docker availability
        let detection = crate::sandbox::detect::check_docker().await;

        match detection.status {
            crate::sandbox::detect::DockerStatus::Available => {
                self.settings.sandbox.enabled = true;
                print_success("Docker is installed and running. Worker sandbox enabled.");
            }
            crate::sandbox::detect::DockerStatus::NotInstalled
            | crate::sandbox::detect::DockerStatus::NotRunning => {
                println!();
                let not_installed =
                    detection.status == crate::sandbox::detect::DockerStatus::NotInstalled;
                if not_installed {
                    print_error("Docker is not installed.");
                    print_info(detection.platform.install_hint());
                } else {
                    print_error("Docker is installed but not running.");
                    print_info(detection.platform.start_hint());
                }
                println!();

                let retry_prompt = if not_installed {
                    "Retry after installing Docker?"
                } else {
                    "Retry after starting Docker?"
                };
                if confirm(retry_prompt, false).map_err(SetupError::Io)? {
                    let retry = crate::sandbox::detect::check_docker().await;
                    if retry.status.is_ok() {
                        self.settings.sandbox.enabled = true;
                        print_success(if not_installed {
                            "Docker is now available. Worker sandbox enabled."
                        } else {
                            "Docker is now running. Worker sandbox enabled."
                        });
                    } else {
                        self.settings.sandbox.enabled = false;
                        print_info(if not_installed {
                            "Docker still not available. Worker sandbox disabled for now."
                        } else {
                            "Docker still not responding. Worker sandbox disabled for now."
                        });
                    }
                } else {
                    self.settings.sandbox.enabled = false;
                    print_info(if not_installed {
                        "Worker sandbox disabled. Install Docker and set SANDBOX_ENABLED=true later."
                    } else {
                        "Worker sandbox disabled. Start Docker and set SANDBOX_ENABLED=true later."
                    });
                }
            }
            crate::sandbox::detect::DockerStatus::Disabled => {
                self.settings.sandbox.enabled = false;
            }
        }

        // ── Part C: Build worker image if needed ─────────────────────────
        if self.settings.sandbox.enabled {
            println!();
            print_info("═══ Worker Docker Image ═══");
            println!();

            // Check if the image already exists
            let image_name = &self.settings.sandbox.image;
            let image_exists = std::process::Command::new("docker")
                .args(["image", "inspect", image_name])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if image_exists {
                print_success(&format!("Worker image '{}' already exists.", image_name));
            } else {
                print_info(&format!("Worker image '{}' not found locally.", image_name));
                print_info("This image is required for Docker sandbox and Claude Code jobs.");
                print_info("Building it now takes 5-15 minutes (one-time).");
                println!();

                if confirm("Build the worker image now?", true).map_err(SetupError::Io)? {
                    print_info("Building thinclaw-worker image (this may take a while)...");

                    // Find the repo root (where Dockerfile.worker lives)
                    let repo_root = std::env::current_dir().unwrap_or_else(|_| {
                        std::path::PathBuf::from(".")
                    });

                    let status = std::process::Command::new("docker")
                        .args([
                            "build",
                            "-f", "Dockerfile.worker",
                            "-t", image_name,
                            ".",
                        ])
                        .current_dir(&repo_root)
                        .stdin(std::process::Stdio::inherit())
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit())
                        .status();

                    match status {
                        Ok(s) if s.success() => {
                            print_success("Worker image built successfully.");
                        }
                        Ok(s) => {
                            print_error(&format!(
                                "Docker build failed (exit code {:?}).",
                                s.code()
                            ));
                            print_info("You can build it later with:");
                            print_info("  docker build -f Dockerfile.worker -t thinclaw-worker .");
                        }
                        Err(e) => {
                            print_error(&format!("Failed to start docker build: {}", e));
                            print_info("You can build it later with:");
                            print_info("  docker build -f Dockerfile.worker -t thinclaw-worker .");
                        }
                    }
                } else {
                    print_info("Skipping image build. Build it later with:");
                    print_info("  docker build -f Dockerfile.worker -t thinclaw-worker .");
                }
            }
        }

        Ok(())
    }

    /// Step 7: Agent identity (name).
    fn step_agent_identity(&mut self) -> Result<(), SetupError> {
        print_info("Give your ThinClaw agent a name. This is used in greetings,");
        print_info("the boot screen, and session metadata.");
        println!();

        let current = &self.settings.agent.name;
        let default_label = format!("current: {}", current);
        let name = optional_input("Agent name", Some(&default_label)).map_err(SetupError::Io)?;

        if let Some(n) = name {
            if !n.is_empty() {
                self.settings.agent.name = n.clone();
                print_success(&format!("Agent name set to '{}'", n));
            } else {
                print_success(&format!("Keeping '{}'", current));
            }
        } else {
            print_success(&format!("Keeping '{}'", current));
        }

        Ok(())
    }

    /// Step 13: Routines (scheduled tasks).
    fn step_routines(&mut self) -> Result<(), SetupError> {
        print_info("Routines let ThinClaw execute scheduled tasks automatically.");
        print_info("Examples: periodic file backups, daily summaries, cron-style jobs.");
        println!();

        if !confirm("Enable routines?", true).map_err(SetupError::Io)? {
            self.settings.routines_enabled = false;
            print_info("Routines disabled. Enable later with ROUTINES_ENABLED=true.");
            return Ok(());
        }

        self.settings.routines_enabled = true;
        print_success("Routines enabled");
        Ok(())
    }

    /// Step 14: Skills.
    fn step_skills(&mut self) -> Result<(), SetupError> {
        print_info("Skills are composable capability plugins that give ThinClaw");
        print_info("domain-specific knowledge (e.g., coding standards, deployment");
        print_info("procedures). They are loaded from ~/.thinclaw/skills/.");
        println!();

        if !confirm("Enable skills system?", true).map_err(SetupError::Io)? {
            self.settings.skills_enabled = false;
            print_info("Skills disabled. Enable later with SKILLS_ENABLED=true.");
            return Ok(());
        }

        self.settings.skills_enabled = true;
        print_success("Skills system enabled");
        Ok(())
    }

    /// Step 11: Claude Code sandbox.
    async fn step_claude_code(&mut self) -> Result<(), SetupError> {
        // Claude Code requires the Docker sandbox to be enabled
        if !self.settings.sandbox.enabled {
            print_info("Claude Code requires Docker sandbox (not enabled in step 10).");
            print_info("Skipping Claude Code configuration.");
            self.settings.claude_code_enabled = false;
            return Ok(());
        }

        print_info("Claude Code sandbox allows ThinClaw to delegate complex coding");
        print_info("tasks to Anthropic's Claude Code CLI running inside a Docker container.");
        print_info("Requires an Anthropic API key or Claude Code OAuth session.");
        println!();

        if !confirm("Enable Claude Code sandbox?", false).map_err(SetupError::Io)? {
            self.settings.claude_code_enabled = false;
            print_info("Claude Code disabled. Enable later with CLAUDE_CODE_ENABLED=true.");
            return Ok(());
        }

        self.settings.claude_code_enabled = true;

        // ── Auth strategy ────────────────────────────────────────────────
        println!();
        print_info("═══ Claude Code Authentication ═══");
        println!();

        // Check existing auth sources
        let has_env_key = std::env::var("ANTHROPIC_API_KEY").is_ok();
        let has_oauth = crate::config::ClaudeCodeConfig::extract_oauth_token().is_some();

        let has_keychain_key = crate::secrets::keychain::get_api_key(
            crate::secrets::keychain::CLAUDE_CODE_API_KEY_ACCOUNT,
        )
        .await
        .is_some();

        if has_env_key {
            print_success("✓ ANTHROPIC_API_KEY found in environment. This will be used.");
        } else if has_keychain_key {
            print_success("✓ Anthropic API key found in OS keychain. This will be used.");
        } else if has_oauth {
            print_success("✓ Claude Code OAuth session found. This will be used.");
            print_info("  (Token from 'claude login' — typically valid for 8-12 hours)");
        } else {
            print_info("No existing auth found. Claude Code containers need one of:");
            print_info("  1. Anthropic API key (stored securely in OS keychain)");
            print_info("  2. OAuth session from 'claude login' on this machine");
            println!();

            if confirm(
                "Enter an Anthropic API key to store in the OS keychain?",
                true,
            )
            .map_err(SetupError::Io)?
            {
                let api_key =
                    input("Anthropic API key (sk-ant-...)").map_err(SetupError::Io)?;

                if api_key.starts_with("sk-ant-") {
                    match crate::secrets::keychain::store_api_key(
                        crate::secrets::keychain::CLAUDE_CODE_API_KEY_ACCOUNT,
                        &api_key,
                    )
                    .await
                    {
                        Ok(()) => {
                            print_success("API key stored securely in OS keychain.");
                            print_info("It will be injected into Claude Code containers at runtime.");
                        }
                        Err(e) => {
                            print_error(&format!("Failed to store in keychain: {}", e));
                            print_info("You can set ANTHROPIC_API_KEY in your environment instead.");
                        }
                    }
                } else {
                    print_error("Key doesn't look like an Anthropic API key (expected sk-ant-...)");
                    print_info("You can set ANTHROPIC_API_KEY in your environment later.");
                }
            } else {
                print_info("No API key stored. You can:");
                print_info("  • Run 'claude login' to set up OAuth");
                print_info("  • Set ANTHROPIC_API_KEY in your environment");
            }
        }

        // ── Model ────────────────────────────────────────────────────────
        println!();
        let model =
            optional_input("Claude Code model", Some("default: sonnet")).map_err(SetupError::Io)?;
        if let Some(m) = model {
            if !m.is_empty() {
                self.settings.claude_code_model = Some(m);
            }
        }

        // Max turns
        let turns =
            optional_input("Max agentic turns", Some("default: 50")).map_err(SetupError::Io)?;
        if let Some(t) = turns {
            if let Ok(n) = t.parse::<u32>() {
                self.settings.claude_code_max_turns = Some(n);
            }
        }

        let model_display = self
            .settings
            .claude_code_model
            .as_deref()
            .unwrap_or("sonnet");
        let turns_display = self.settings.claude_code_max_turns.unwrap_or(50);
        print_success(&format!(
            "Claude Code enabled (model: {}, max turns: {})",
            model_display, turns_display
        ));

        Ok(())
    }

    /// Step 5: Smart Routing configuration.
    ///
    /// Allows setting a cheap/fast model for lightweight tasks like
    /// routing decisions, heartbeat checks, and prompt evaluation.
    /// If the cheap model uses a different provider than the primary,
    /// prompts for that provider's API key (unless already configured).
    async fn step_smart_routing(&mut self) -> Result<(), SetupError> {
        print_info("Smart Routing can use a cheaper/faster model for lightweight tasks");
        print_info("(e.g., routing decisions, heartbeat checks, prompt evaluation).");
        print_info("The primary model is still used for complex conversations.");
        println!();

        if !confirm("Configure a cheap model for smart routing?", false).map_err(SetupError::Io)? {
            print_info("Smart routing disabled — all tasks use the primary model.");
            return Ok(());
        }

        println!();
        print_info("Format: provider/model (e.g., \"groq/llama-3.1-8b-instant\",");
        print_info("\"openai/gpt-4o-mini\", \"anthropic/claude-3-5-haiku-20241022\")");

        let current = self.settings.providers.cheap_model.as_deref().unwrap_or("");
        let cheap_model = if current.is_empty() {
            input("Cheap model").map_err(SetupError::Io)?
        } else {
            let keep = confirm(&format!("Keep current cheap model ({})?", current), true)
                .map_err(SetupError::Io)?;
            if keep {
                current.to_string()
            } else {
                input("Cheap model").map_err(SetupError::Io)?
            }
        };

        if cheap_model.is_empty() {
            print_info("No cheap model set — smart routing disabled.");
            return Ok(());
        }

        self.settings.providers.cheap_model = Some(cheap_model.clone());
        print_success(&format!(
            "Smart routing enabled — cheap model: {}",
            cheap_model
        ));

        // ── Check if the cheap model's provider needs a separate API key ──
        // Parse provider slug from "provider/model" format.
        if let Some(cheap_provider_slug) = cheap_model.split('/').next() {
            // Determine the primary provider slug for comparison.
            let primary_slug = self
                .settings
                .llm_backend
                .as_deref()
                .unwrap_or("");

            // Only prompt for a key if the cheap provider differs from the primary.
            if !cheap_provider_slug.is_empty()
                && cheap_provider_slug != primary_slug
                && cheap_provider_slug != "ollama"
            {
                // Look up the cheap provider in the catalog.
                if let Some(endpoint) =
                    crate::config::provider_catalog::endpoint_for(cheap_provider_slug)
                {
                    // Check if the API key is already available (env var, keychain, or secrets).
                    let has_env_key = std::env::var(endpoint.env_key_name).is_ok();
                    let has_keychain_key = crate::secrets::keychain::get_api_key(
                        endpoint.secret_name,
                    )
                    .await
                    .is_some();

                    if has_env_key {
                        println!();
                        print_success(&format!(
                            "✓ {} API key found in environment ({}).",
                            endpoint.display_name, endpoint.env_key_name
                        ));
                    } else if has_keychain_key {
                        println!();
                        print_success(&format!(
                            "✓ {} API key found in OS keychain.",
                            endpoint.display_name
                        ));
                    } else {
                        // API key is missing — prompt the user.
                        println!();
                        print_info(&format!(
                            "The cheap model uses {} — a different provider than your primary.",
                            endpoint.display_name
                        ));
                        print_info(&format!(
                            "An API key for {} is required.",
                            endpoint.display_name
                        ));

                        self.setup_api_key_provider(
                            cheap_provider_slug,
                            endpoint.env_key_name,
                            endpoint.secret_name,
                            &format!("{} API key", endpoint.display_name),
                            &format!(
                                "https://console.{}",
                                cheap_provider_slug
                            ),
                            Some(endpoint.display_name),
                        )
                        .await?;
                    }
                } else {
                    // Provider not in catalog — warn but continue.
                    println!();
                    print_info(&format!(
                        "Provider '{}' is not in the built-in catalog.",
                        cheap_provider_slug
                    ));
                    print_info(&format!(
                        "Make sure the API key is set via the appropriate environment variable."
                    ));
                }
            }
        }

        Ok(())
    }

    /// Step 17: Web UI (WebChat) configuration.
    ///
    /// Configures the gateway dashboard appearance: theme, accent color,
    /// and branding badge visibility.
    fn step_web_ui(&mut self) -> Result<(), SetupError> {
        print_info("ThinClaw includes a web dashboard (gateway UI) for chat and monitoring.");
        print_info("You can customize its appearance here.");
        println!();

        if !confirm("Customize web UI appearance?", false).map_err(SetupError::Io)? {
            print_info("Using defaults (system theme, default accent color, branding shown).");
            return Ok(());
        }

        println!();

        // Theme selection
        let theme_options: &[&str] = &["System (follow OS preference)", "Light", "Dark"];
        let theme_idx = select_one("Theme", theme_options).map_err(SetupError::Io)?;
        let theme = match theme_idx {
            1 => "light",
            2 => "dark",
            _ => "system",
        };
        self.settings.webchat_theme = theme.to_string();

        // Accent color
        let accent = optional_input(
            "Accent color (hex, e.g. #22c55e)",
            Some("leave blank for default"),
        )
        .map_err(SetupError::Io)?;
        if let Some(ref color) = accent {
            if !color.is_empty() {
                self.settings.webchat_accent_color = Some(color.clone());
            }
        }

        // Branding badge
        let show_branding =
            confirm("Show \"Powered by ThinClaw\" badge?", true).map_err(SetupError::Io)?;
        self.settings.webchat_show_branding = show_branding;

        let accent_display = self
            .settings
            .webchat_accent_color
            .as_deref()
            .unwrap_or("default");
        print_success(&format!(
            "Web UI configured (theme: {}, accent: {}, branding: {})",
            theme,
            accent_display,
            if show_branding { "shown" } else { "hidden" }
        ));

        Ok(())
    }

    /// Step 18: Observability configuration.
    ///
    /// Selects the event and metric recording backend.
    fn step_observability(&mut self) -> Result<(), SetupError> {
        print_info("Observability records events and metrics for debugging and monitoring.");
        println!();

        let options: &[&str] = &[
            "None (no overhead, default)",
            "Log (structured events via tracing)",
        ];
        let idx = select_one("Observability backend", options).map_err(SetupError::Io)?;
        let backend = match idx {
            1 => "log",
            _ => "none",
        };
        self.settings.observability_backend = backend.to_string();

        if backend == "log" {
            print_success("Observability enabled — events will be emitted via tracing.");
        } else {
            print_info("Observability disabled — zero overhead mode.");
        }

        Ok(())
    }

    /// Step 16: Notification preferences.
    /// Configure which channel receives proactive notifications.
    ///
    /// Auto-selects if only one channel is configured.
    /// For Telegram, auto-populates with the owner's chat ID.
    /// For iMessage/Signal, prompts for phone number.
    fn step_notification_preferences(&mut self) -> Result<(), SetupError> {
        print_info("ThinClaw sends proactive notifications (heartbeat alerts, routine results,");
        print_info("self-repair messages) to a channel of your choice.");
        println!();

        // Collect configured channels
        let mut channels: Vec<String> = Vec::new();
        channels.push("web".to_string()); // Always available
        // Telegram is a WASM channel — detected by owner binding or wasm_channels list
        if self.settings.channels.telegram_owner_id.is_some()
            || self
                .settings
                .channels
                .wasm_channels
                .iter()
                .any(|c| c == "telegram")
        {
            channels.push("telegram".to_string());
        }
        if self.settings.channels.imessage_enabled {
            channels.push("imessage".to_string());
        }
        if self.settings.channels.apple_mail_enabled {
            channels.push("apple_mail".to_string());
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

        if channels.len() == 1 {
            // Only web — no external channels configured
            print_info("Only the web channel is configured.");
            print_info("Notifications will appear in the Web UI.");
            self.settings.notifications.preferred_channel = Some("web".to_string());
            self.settings.notifications.recipient = Some("default".to_string());
            return Ok(());
        }

        if channels.len() == 2 {
            // Exactly one external channel — auto-select it
            let ch = channels[1].clone(); // Skip "web"
            print_info(&format!(
                "Auto-selecting '{}' as your notification channel (only external channel configured).",
                ch
            ));
            self.settings.notifications.preferred_channel = Some(ch.clone());
            self.collect_notification_recipient(&ch)?;
            return Ok(());
        }

        // Multiple channels — ask user to pick
        let options: Vec<String> = channels
            .iter()
            .map(|ch| match ch.as_str() {
                "web" => "web       — Web UI only (always available)".to_string(),
                "telegram" => "telegram  — Telegram bot messages".to_string(),
                "imessage" => "imessage    — iMessage (macOS)".to_string(),
                "apple_mail" => "apple_mail  — Apple Mail (macOS)".to_string(),
                "signal" => "signal    — Signal messenger".to_string(),
                "discord" => "discord   — Discord bot".to_string(),
                "slack" => "slack     — Slack workspace".to_string(),
                "nostr" => "nostr     — Nostr relay".to_string(),
                other => other.to_string(),
            })
            .collect();

        let option_strs: Vec<&str> = options.iter().map(|s| s.as_str()).collect();
        let choice = select_one("Which channel for proactive notifications?", &option_strs)
            .map_err(SetupError::Io)?;

        let selected = channels[choice].clone();
        self.settings.notifications.preferred_channel = Some(selected.clone());

        if selected != "web" {
            self.collect_notification_recipient(&selected)?;
        } else {
            self.settings.notifications.recipient = Some("default".to_string());
        }

        print_success(&format!("Notifications will be sent via '{}'", selected));
        print_info("You can change this later in Settings > Notifications.");

        Ok(())
    }

    /// Collect the recipient identifier for a given notification channel.
    fn collect_notification_recipient(&mut self, channel: &str) -> Result<(), SetupError> {
        match channel {
            "telegram" => {
                // Auto-populate from Telegram owner binding
                if let Some(owner_id) = self.settings.channels.telegram_owner_id {
                    print_info(&format!("Telegram owner detected (ID: {}).", owner_id));
                    if confirm("Use this account for notifications?", true)
                        .map_err(SetupError::Io)?
                    {
                        self.settings.notifications.recipient = Some(owner_id.to_string());
                        return Ok(());
                    }
                }
                let id = input("Telegram chat ID (numeric)").map_err(SetupError::Io)?;
                if !id.is_empty() {
                    self.settings.notifications.recipient = Some(id);
                }
            }
            "imessage" => {
                print_info("Enter your phone number or Apple ID for iMessage notifications.");
                let contact = input("Phone number or Apple ID (e.g., +4917612345678)")
                    .map_err(SetupError::Io)?;
                if !contact.is_empty() {
                    self.settings.notifications.recipient = Some(contact);
                } else {
                    print_info("No recipient set — iMessage notifications disabled.");
                    self.settings.notifications.preferred_channel = Some("web".to_string());
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            "apple_mail" => {
                print_info("Enter your email address for Apple Mail notifications.");
                let email = input("Email address").map_err(SetupError::Io)?;
                if !email.is_empty() {
                    self.settings.notifications.recipient = Some(email);
                } else {
                    print_info("No recipient set — Apple Mail notifications disabled.");
                    self.settings.notifications.preferred_channel = Some("web".to_string());
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            "signal" => {
                print_info("Enter your phone number for Signal notifications.");
                let number = input("Phone number (E.164 format, e.g., +4917612345678)")
                    .map_err(SetupError::Io)?;
                if !number.is_empty() {
                    self.settings.notifications.recipient = Some(number);
                } else {
                    print_info("No recipient set — Signal notifications disabled.");
                    self.settings.notifications.preferred_channel = Some("web".to_string());
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            "discord" => {
                print_info("Enter your Discord user ID for notifications.");
                let id = input("Discord user ID").map_err(SetupError::Io)?;
                if !id.is_empty() {
                    self.settings.notifications.recipient = Some(id);
                } else {
                    self.settings.notifications.recipient = Some("default".to_string());
                }
            }
            _ => {
                self.settings.notifications.recipient = Some("default".to_string());
            }
        }
        Ok(())
    }

    /// Step 12: Tool approval mode.
    fn step_tool_approval(&mut self) -> Result<(), SetupError> {
        print_info("ThinClaw can execute tools (shell commands, file operations, etc.) on your behalf.");
        print_info("Choose how much autonomy to grant the agent:");
        println!();

        let options = [
            "Standard  — Ask before running destructive operations (recommended)",
            "Autonomous — Auto-approve safe operations, still block destructive commands\n               (rm -rf, DROP TABLE, git push --force, etc.)",
            "Full Auto  — Skip ALL approval checks (for benchmarks/CI only)\n               ⚠️  WARNING: The agent can execute ANY command without asking!",
        ];
        let option_refs: Vec<&str> = options.iter().map(|s| *s).collect();
        let choice = select_one("Tool approval mode", &option_refs)
            .map_err(SetupError::Io)?;

        match choice {
            0 => {
                self.settings.agent.auto_approve_tools = false;
                print_success("Standard approval mode — agent will ask before destructive operations.");
            }
            1 => {
                self.settings.agent.auto_approve_tools = true;
                print_success(
                    "Autonomous mode — safe operations auto-approved, destructive commands still blocked.",
                );
                print_info(
                    "Note: Commands matching NEVER_AUTO_APPROVE_PATTERNS (rm -rf, DROP TABLE, etc.)",
                );
                print_info("will still require your approval even in this mode.");
            }
            2 => {
                self.settings.agent.auto_approve_tools = true;
                print_success("Full auto-approve mode — ALL tool executions will run without asking.");
                print_info("⚠️  Use with extreme caution. This is intended for benchmarks/CI environments.");
            }
            _ => {
                self.settings.agent.auto_approve_tools = false;
            }
        }
        Ok(())
    }

    /// Step 15: Heartbeat (background tasks) configuration.
    fn step_heartbeat(&mut self) -> Result<(), SetupError> {
        print_info("Heartbeat runs periodic background tasks (e.g., checking your calendar,");
        print_info("monitoring for notifications, running scheduled workflows).");
        println!();

        if !confirm("Enable heartbeat?", false).map_err(SetupError::Io)? {
            self.settings.heartbeat.enabled = false;
            print_info("Heartbeat disabled.");
            return Ok(());
        }

        self.settings.heartbeat.enabled = true;

        // Interval
        let interval_str = optional_input("Check interval in minutes", Some("default: 30"))
            .map_err(SetupError::Io)?;

        if let Some(s) = interval_str {
            if let Ok(mins) = s.parse::<u64>() {
                self.settings.heartbeat.interval_secs = mins * 60;
            }
        } else {
            self.settings.heartbeat.interval_secs = 1800; // 30 minutes
        }

        // Notification channel is configured in step 16 (Notification Preferences)
        // which handles recipient selection for all proactive messages.

        print_success(&format!(
            "Heartbeat enabled (every {} minutes)",
            self.settings.heartbeat.interval_secs / 60
        ));

        Ok(())
    }

    /// Persist current settings to the database.
    ///
    /// Returns `Ok(true)` if settings were saved, `Ok(false)` if no database
    /// connection is available yet (e.g., before Step 1 completes).
    async fn persist_settings(&self) -> Result<bool, SetupError> {
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
    fn write_bootstrap_env(&self) -> Result<(), SetupError> {
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

        // Discord channel env vars
        if self.settings.channels.discord_enabled {
            env_vars.push(("DISCORD_ENABLED", "true".to_string()));
        }
        if let Some(ref token) = self.settings.channels.discord_bot_token {
            env_vars.push(("DISCORD_BOT_TOKEN", token.clone()));
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
        }
        if let Some(ref token) = self.settings.channels.slack_bot_token {
            env_vars.push(("SLACK_BOT_TOKEN", token.clone()));
        }
        if let Some(ref token) = self.settings.channels.slack_app_token {
            env_vars.push(("SLACK_APP_TOKEN", token.clone()));
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

        // Web Gateway env vars
        if let Some(ref port) = self.settings.channels.gateway_port {
            env_vars.push(("GATEWAY_PORT", port.to_string()));
        }
        if let Some(ref token) = self.settings.channels.gateway_auth_token {
            env_vars.push(("GATEWAY_AUTH_TOKEN", token.clone()));
        }

        // Smart Routing env vars
        if let Some(ref model) = self.settings.providers.cheap_model {
            env_vars.push(("CHEAP_MODEL", model.clone()));
        }

        // Web UI env vars
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
    async fn persist_after_step(&self) {
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
    async fn try_load_existing_settings(&mut self) {
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

    /// Save settings to the database and `~/.thinclaw/.env`, then print summary.
    async fn save_and_summarize(&mut self) -> Result<(), SetupError> {
        self.settings.onboard_completed = true;

        // Final persist (idempotent — earlier incremental saves already wrote
        // most settings, but this ensures onboard_completed is saved).
        let saved = self.persist_settings().await?;

        if !saved {
            return Err(SetupError::Database(
                "No database connection, cannot save settings".to_string(),
            ));
        }

        // Write bootstrap env (also idempotent)
        self.write_bootstrap_env()?;

        println!();
        print_success("Configuration saved to database");
        println!();

        // Print summary
        println!("Configuration Summary:");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        let backend = self
            .settings
            .database_backend
            .as_deref()
            .unwrap_or("postgres");
        match backend {
            "libsql" => {
                if let Some(ref path) = self.settings.libsql_path {
                    println!("  Database: libSQL ({})", path);
                } else {
                    println!("  Database: libSQL (default path)");
                }
                if self.settings.libsql_url.is_some() {
                    println!("  Turso sync: enabled");
                }
            }
            _ => {
                if self.settings.database_url.is_some() {
                    println!("  Database: PostgreSQL (configured)");
                }
            }
        }

        match self.settings.secrets_master_key_source {
            KeySource::Keychain => println!("  Security: OS keychain"),
            KeySource::Env => println!("  Security: environment variable"),
            KeySource::None => println!("  Security: disabled"),
        }

        if let Some(ref provider) = self.settings.llm_backend {
            let display = match provider.as_str() {
                "anthropic" => "Anthropic",
                "openai" => "OpenAI",
                "ollama" => "Ollama",
                "openai_compatible" => "OpenAI-compatible",
                other => other,
            };
            println!("  Provider: {}", display);
        }

        if let Some(ref model) = self.settings.selected_model {
            // Truncate long model names (char-based to avoid UTF-8 panic)
            let display = if model.chars().count() > 40 {
                let truncated: String = model.chars().take(37).collect();
                format!("{}...", truncated)
            } else {
                model.clone()
            };
            println!("  Model: {}", display);
        }

        if self.settings.embeddings.enabled {
            println!(
                "  Embeddings: {} ({})",
                self.settings.embeddings.provider, self.settings.embeddings.model
            );
        } else {
            println!("  Embeddings: disabled");
        }

        if let Some(ref tunnel_url) = self.settings.tunnel.public_url {
            println!("  Tunnel: {} (static)", tunnel_url);
        } else if let Some(ref provider) = self.settings.tunnel.provider {
            println!("  Tunnel: {} (managed, starts at boot)", provider);
        }

        let has_tunnel =
            self.settings.tunnel.public_url.is_some() || self.settings.tunnel.provider.is_some();

        println!("  Channels:");
        println!("    - CLI/TUI: enabled");

        if self.settings.channels.http_enabled {
            let port = self.settings.channels.http_port.unwrap_or(8080);
            println!("    - HTTP: enabled (port {})", port);
        }

        if self.settings.channels.signal_enabled {
            println!("    - Signal: enabled");
        }

        if self.settings.channels.discord_enabled {
            println!("    - Discord: enabled");
        }

        if self.settings.channels.slack_enabled {
            println!("    - Slack: enabled");
        }

        if self.settings.channels.nostr_enabled {
            println!("    - Nostr: enabled");
        }

        if self.settings.channels.gmail_enabled {
            println!("    - Gmail: enabled");
        }

        #[cfg(target_os = "macos")]
        if self.settings.channels.imessage_enabled {
            println!("    - iMessage: enabled");
        }

        #[cfg(target_os = "macos")]
        if self.settings.channels.apple_mail_enabled {
            println!("    - Apple Mail: enabled");
        }

        for channel_name in &self.settings.channels.wasm_channels {
            let mode = if has_tunnel { "webhook" } else { "polling" };
            println!(
                "    - {}: enabled ({})",
                capitalize_first(channel_name),
                mode
            );
        }

        println!("  Agent: {}", self.settings.agent.name);

        if let Some(ref cheap_model) = self.settings.providers.cheap_model {
            println!("  Smart routing: {} (cheap)", cheap_model);
        }

        if self.settings.heartbeat.enabled {
            println!(
                "  Heartbeat: every {} minutes",
                self.settings.heartbeat.interval_secs / 60
            );
        }

        if self.settings.routines_enabled {
            println!("  Routines: enabled");
        }

        if self.settings.skills_enabled {
            println!("  Skills: enabled");
        }

        if self.settings.claude_code_enabled {
            let model = self
                .settings
                .claude_code_model
                .as_deref()
                .unwrap_or("sonnet");
            println!("  Claude Code: enabled (model: {})", model);
        }

        if self.settings.webchat_theme != "system" || self.settings.webchat_accent_color.is_some() {
            let accent = self
                .settings
                .webchat_accent_color
                .as_deref()
                .unwrap_or("default");
            println!(
                "  Web UI: theme={}, accent={}",
                self.settings.webchat_theme, accent
            );
        }

        if self.settings.observability_backend != "none" {
            println!("  Observability: {}", self.settings.observability_backend);
        }

        println!();

        // ── PATH check & symlink offer ──────────────────────────
        // If the current binary isn't on PATH, offer to create a symlink so
        // the user can just type `thinclaw` from any terminal.
        self.offer_path_setup();

        println!("To start the agent, run:");
        println!("  thinclaw");
        println!();
        println!("To change settings later:");
        println!("  thinclaw config set <setting> <value>");
        println!("  thinclaw onboard");
        println!();

        Ok(())
    }

    /// Check if `thinclaw` is accessible on PATH and offer to create a
    /// symlink if it isn't.
    fn offer_path_setup(&self) {
        use std::path::Path;

        // Check if `thinclaw` is already findable on PATH
        if which_thinclaw().is_some() {
            return; // Already on PATH, nothing to do
        }

        let current_exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(_) => return, // Can't determine our own path
        };

        // Choose symlink target based on platform
        let symlink_dir = if cfg!(target_os = "macos") {
            Path::new("/usr/local/bin")
        } else {
            // Linux: ~/.local/bin is in PATH for most distros
            let home = match dirs::home_dir() {
                Some(h) => h,
                None => return,
            };
            // We need a 'static-ish path, so use a leak-safe approach
            let local_bin = home.join(".local").join("bin");
            if !local_bin.exists() {
                let _ = std::fs::create_dir_all(&local_bin);
            }
            // Can't return a reference to a local, so handle inline below
            let target = local_bin.join("thinclaw");
            if try_symlink(&current_exe, &target) {
                print_success(&format!(
                    "Symlinked: {} → {}",
                    target.display(),
                    current_exe.display()
                ));
                println!("  You can now use 'thinclaw' from any terminal.");
                if !path_contains(&local_bin) {
                    println!(
                        "  Note: add {} to your PATH if it isn't already:",
                        local_bin.display()
                    );
                    println!(
                        "    echo 'export PATH=\"{}:$PATH\"' >> ~/.bashrc",
                        local_bin.display()
                    );
                }
            } else {
                println!();
                print_info(&format!(
                    "Tip: add thinclaw to your PATH:\n  \
                     sudo ln -sf {} /usr/local/bin/thinclaw\n  \
                     Or: export PATH=\"{}:$PATH\"",
                    current_exe.display(),
                    current_exe.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                ));
            }
            return;
        };

        let target = symlink_dir.join("thinclaw");

        if !symlink_dir.exists() {
            // /usr/local/bin doesn't exist (rare on macOS), just print a tip
            print_info(&format!(
                "Tip: add thinclaw to your PATH:\n  \
                 export PATH=\"{}:$PATH\"",
                current_exe.parent().map(|p| p.display().to_string()).unwrap_or_default(),
            ));
            return;
        }

        // Try without sudo first (works if user owns /usr/local/bin, e.g. Homebrew)
        if try_symlink(&current_exe, &target) {
            print_success(&format!(
                "Symlinked: {} → {}",
                target.display(),
                current_exe.display()
            ));
            println!("  You can now use 'thinclaw' from any terminal.");
            return;
        }

        // Need elevated permissions — ask
        println!();
        print_info("thinclaw is not on your PATH. Create a symlink so you can run it from anywhere?");
        match confirm("Create /usr/local/bin/thinclaw symlink (requires sudo)?", true) {
            Ok(true) => {
                let status = std::process::Command::new("sudo")
                    .args(["ln", "-sf"])
                    .arg(current_exe.display().to_string())
                    .arg(target.display().to_string())
                    .status();

                match status {
                    Ok(s) if s.success() => {
                        print_success(&format!(
                            "Symlinked: {} → {}",
                            target.display(),
                            current_exe.display()
                        ));
                        println!("  You can now use 'thinclaw' from any terminal.");
                    }
                    _ => {
                        print_info(&format!(
                            "Symlink failed. Add manually:\n  \
                             sudo ln -sf {} {}",
                            current_exe.display(),
                            target.display()
                        ));
                    }
                }
            }
            _ => {
                print_info(&format!(
                    "Skipped. To add later:\n  \
                     sudo ln -sf {} {}",
                    current_exe.display(),
                    target.display()
                ));
            }
        }
    }
}

impl Default for SetupWizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if `thinclaw` is findable on PATH by scanning PATH directories.
fn which_thinclaw() -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("thinclaw");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Try to create a symlink, removing any existing file/link at the target.
/// Returns true on success.
#[cfg(unix)]
fn try_symlink(source: &std::path::Path, target: &std::path::Path) -> bool {
    // Remove existing symlink/file if present (ignore errors)
    let _ = std::fs::remove_file(target);
    std::os::unix::fs::symlink(source, target).is_ok()
}

#[cfg(not(unix))]
fn try_symlink(_source: &std::path::Path, _target: &std::path::Path) -> bool {
    false
}

/// Check if a directory is present in the current PATH.
fn path_contains(dir: &std::path::Path) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|p| p == dir)
}

/// Mask password in a database URL for display.
#[cfg(feature = "postgres")]
fn mask_password_in_url(url: &str) -> String {
    // URL format: scheme://user:password@host/database
    // Find "://" to locate start of credentials
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let credentials_start = scheme_end + 3; // After "://"

    // Find "@" to locate end of credentials
    let Some(at_pos) = url[credentials_start..].find('@') else {
        return url.to_string();
    };
    let at_abs = credentials_start + at_pos;

    // Find ":" in the credentials section (separates user from password)
    let credentials = &url[credentials_start..at_abs];
    let Some(colon_pos) = credentials.find(':') else {
        return url.to_string();
    };

    // Build masked URL: scheme://user:****@host/database
    let scheme = &url[..credentials_start]; // "postgres://"
    let username = &credentials[..colon_pos]; // "user"
    let after_at = &url[at_abs..]; // "@localhost/db"

    format!("{}{}:****{}", scheme, username, after_at)
}

/// Fetch models from the Anthropic API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
async fn fetch_anthropic_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "claude-opus-4-6".into(),
            "Claude Opus 4.6 (latest flagship)".into(),
        ),
        ("claude-sonnet-4-6".into(), "Claude Sonnet 4.6".into()),
        ("claude-opus-4-5".into(), "Claude Opus 4.5".into()),
        ("claude-sonnet-4-5".into(), "Claude Sonnet 4.5".into()),
        ("claude-haiku-4-5".into(), "Claude Haiku 4.5 (fast)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .filter(|k| !k.is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => return static_defaults,
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.anthropic.com/v1/models")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| !m.id.contains("embedding") && !m.id.contains("audio"))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models.sort_by(|a, b| a.0.cmp(&b.0));
            models
        }
        Err(_) => static_defaults,
    }
}

/// Fetch models from the OpenAI API.
///
/// Returns `(model_id, display_label)` pairs. Falls back to static defaults on error.
async fn fetch_openai_models(cached_key: Option<&str>) -> Vec<(String, String)> {
    let static_defaults = vec![
        (
            "gpt-5.3-codex".into(),
            "GPT-5.3 Codex (latest flagship)".into(),
        ),
        ("gpt-5.2-codex".into(), "GPT-5.2 Codex".into()),
        ("gpt-5.2".into(), "GPT-5.2".into()),
        (
            "gpt-5.1-codex-mini".into(),
            "GPT-5.1 Codex Mini (fast)".into(),
        ),
        ("gpt-5".into(), "GPT-5".into()),
        ("gpt-5-mini".into(), "GPT-5 Mini".into()),
        ("gpt-4.1".into(), "GPT-4.1".into()),
        ("gpt-4.1-mini".into(), "GPT-4.1 Mini".into()),
        ("o4-mini".into(), "o4-mini (fast reasoning)".into()),
        ("o3".into(), "o3 (reasoning)".into()),
    ];

    let api_key = cached_key
        .map(String::from)
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .filter(|k| !k.is_empty());

    let api_key = match api_key {
        Some(k) => k,
        None => return static_defaults,
    };

    let client = reqwest::Client::new();
    let resp = match client
        .get("https://api.openai.com/v1/models")
        .bearer_auth(&api_key)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => return static_defaults,
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelEntry>,
    }

    match resp.json::<ModelsResponse>().await {
        Ok(body) => {
            let mut models: Vec<(String, String)> = body
                .data
                .into_iter()
                .filter(|m| is_openai_chat_model(&m.id))
                .map(|m| {
                    let label = m.id.clone();
                    (m.id, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            sort_openai_models(&mut models);
            models
        }
        Err(_) => static_defaults,
    }
}

fn is_openai_chat_model(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();

    let is_chat_family = id.starts_with("gpt-")
        || id.starts_with("chatgpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("o5");

    let is_non_chat_variant = id.contains("realtime")
        || id.contains("audio")
        || id.contains("transcribe")
        || id.contains("tts")
        || id.contains("embedding")
        || id.contains("moderation")
        || id.contains("image");

    is_chat_family && !is_non_chat_variant
}

fn openai_model_priority(model_id: &str) -> usize {
    let id = model_id.to_ascii_lowercase();

    const EXACT_PRIORITY: &[&str] = &[
        "gpt-5.3-codex",
        "gpt-5.2-codex",
        "gpt-5.2",
        "gpt-5.1-codex-mini",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
        "o4-mini",
        "o3",
        "o1",
        "gpt-4.1",
        "gpt-4.1-mini",
        "gpt-4o",
        "gpt-4o-mini",
    ];
    if let Some(pos) = EXACT_PRIORITY.iter().position(|m| id == *m) {
        return pos;
    }

    const PREFIX_PRIORITY: &[&str] = &[
        "gpt-5.", "gpt-5-", "o3-", "o4-", "o1-", "gpt-4.1-", "gpt-4o-", "gpt-3.5-", "chatgpt-",
    ];
    if let Some(pos) = PREFIX_PRIORITY
        .iter()
        .position(|prefix| id.starts_with(prefix))
    {
        return EXACT_PRIORITY.len() + pos;
    }

    EXACT_PRIORITY.len() + PREFIX_PRIORITY.len() + 1
}

fn sort_openai_models(models: &mut [(String, String)]) {
    models.sort_by(|a, b| {
        openai_model_priority(&a.0)
            .cmp(&openai_model_priority(&b.0))
            .then_with(|| a.0.cmp(&b.0))
    });
}

/// Fetch installed models from a local Ollama instance.
///
/// Returns `(model_name, display_label)` pairs. Falls back to static defaults on error.
async fn fetch_ollama_models(base_url: &str) -> Vec<(String, String)> {
    let static_defaults = vec![
        ("llama3".into(), "llama3".into()),
        ("mistral".into(), "mistral".into()),
        ("codellama".into(), "codellama".into()),
    ];

    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();

    let resp = match client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(_) => return static_defaults,
        Err(_) => {
            print_info("Could not connect to Ollama. Is it running?");
            return static_defaults;
        }
    };

    #[derive(serde::Deserialize)]
    struct ModelEntry {
        name: String,
    }
    #[derive(serde::Deserialize)]
    struct TagsResponse {
        models: Vec<ModelEntry>,
    }

    match resp.json::<TagsResponse>().await {
        Ok(body) => {
            let models: Vec<(String, String)> = body
                .models
                .into_iter()
                .map(|m| {
                    let label = m.name.clone();
                    (m.name, label)
                })
                .collect();
            if models.is_empty() {
                return static_defaults;
            }
            models
        }
        Err(_) => static_defaults,
    }
}

/// Discover WASM channels in a directory.
///
/// Returns a list of (channel_name, capabilities_file) pairs.
async fn discover_wasm_channels(dir: &std::path::Path) -> Vec<(String, ChannelCapabilitiesFile)> {
    let mut channels = Vec::new();

    if !dir.is_dir() {
        return channels;
    }

    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(_) => return channels,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();

        // Look for .capabilities.json files
        let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !filename.ends_with(".capabilities.json") {
            continue;
        }

        // Extract channel name
        let name = filename.trim_end_matches(".capabilities.json").to_string();
        if name.is_empty() {
            continue;
        }

        // Check if corresponding .wasm file exists
        let wasm_path = dir.join(format!("{}.wasm", name));
        if !wasm_path.exists() {
            continue;
        }

        // Parse capabilities file
        match tokio::fs::read(&path).await {
            Ok(bytes) => match ChannelCapabilitiesFile::from_bytes(&bytes) {
                Ok(cap_file) => {
                    channels.push((name, cap_file));
                }
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %e,
                        "Failed to parse channel capabilities file"
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Failed to read channel capabilities file"
                );
            }
        }
    }

    // Sort by name for consistent ordering
    channels.sort_by(|a, b| a.0.cmp(&b.0));
    channels
}

/// Mask an API key for display: show first 6 + last 4 chars.
///
/// Uses char-based indexing to avoid panicking on multi-byte UTF-8.
fn mask_api_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() < 12 {
        let prefix: String = chars.iter().take(4).collect();
        return format!("{prefix}...");
    }
    let prefix: String = chars[..6].iter().collect();
    let suffix: String = chars[chars.len() - 4..].iter().collect();
    format!("{prefix}...{suffix}")
}

/// Capitalize the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
async fn install_missing_bundled_channels(
    channels_dir: &std::path::Path,
    already_installed: &HashSet<String>,
) -> Result<Vec<String>, SetupError> {
    let mut installed = Vec::new();

    for name in available_channel_names().iter().copied() {
        if already_installed.contains(name) {
            continue;
        }

        install_bundled_channel(name, channels_dir, false)
            .await
            .map_err(SetupError::Channel)?;
        installed.push(name.to_string());
    }

    Ok(installed)
}

/// Build channel options from discovered channels + bundled + registry catalog.
///
/// Returns a deduplicated, sorted list of channel names available for selection.
fn build_channel_options(discovered: &[(String, ChannelCapabilitiesFile)]) -> Vec<String> {
    let mut names: Vec<String> = discovered.iter().map(|(name, _)| name.clone()).collect();

    // Add channels embedded in the binary (--features bundled-wasm)
    for embedded in crate::registry::bundled_wasm::bundled_channel_names() {
        if !names.iter().any(|n| n == embedded) {
            names.push(embedded.to_string());
        }
    }

    // Add bundled channels (pre-compiled in channels-src/)
    for bundled in available_channel_names().iter().copied() {
        if !names.iter().any(|name| name == bundled) {
            names.push(bundled.to_string());
        }
    }

    // Add registry channels
    if let Some(catalog) = load_registry_catalog() {
        for manifest in catalog.list(Some(crate::registry::manifest::ManifestKind::Channel), None) {
            if !names.iter().any(|n| n == &manifest.name) {
                names.push(manifest.name.clone());
            }
        }
    }

    names.sort();
    names
}

/// Try to load the registry catalog. Falls back to embedded manifests when
/// the `registry/` directory cannot be found (e.g. running from an installed binary).
fn load_registry_catalog() -> Option<crate::registry::catalog::RegistryCatalog> {
    crate::registry::catalog::RegistryCatalog::load_or_embedded().ok()
}

/// Install selected channels from the registry that aren't already on disk
/// and weren't handled by the bundled installer.
async fn install_selected_registry_channels(
    channels_dir: &std::path::Path,
    selected_channels: &[String],
    already_installed: &HashSet<String>,
    bundled_installed: &HashSet<String>,
) -> Vec<String> {
    let catalog = match load_registry_catalog() {
        Some(c) => c,
        None => return Vec::new(),
    };

    let repo_root = catalog
        .root()
        .parent()
        .unwrap_or(catalog.root())
        .to_path_buf();

    let bundled_fs: HashSet<&str> = available_channel_names().iter().copied().collect();
    let installer = crate::registry::installer::RegistryInstaller::new(
        repo_root.clone(),
        dirs::home_dir().unwrap_or_default().join(".thinclaw/tools"),
        channels_dir.to_path_buf(),
    );
    let mut installed = Vec::new();

    for name in selected_channels {
        // Skip if already installed or successfully handled by bundled installer
        if already_installed.contains(name)
            || bundled_installed.contains(name)
            || bundled_fs.contains(name.as_str())
        {
            continue;
        }

        // Check if already on disk (may have been installed between bundled and here)
        let wasm_on_disk = channels_dir.join(format!("{}.wasm", name)).exists()
            || channels_dir.join(format!("{}-channel.wasm", name)).exists();
        if wasm_on_disk {
            continue;
        }

        // Look up in registry
        let manifest = match catalog.get(&format!("channels/{}", name)) {
            Some(m) => m,
            None => continue,
        };

        match installer
            .install_with_source_fallback(manifest, false)
            .await
        {
            Ok(outcome) => {
                for warning in &outcome.warnings {
                    crate::setup::prompts::print_info(&format!("{}: {}", name, warning));
                }
                installed.push(name.clone());
            }
            Err(e) => {
                tracing::warn!(
                    channel = %name,
                    error = %e,
                    "Failed to install channel from registry"
                );
                crate::setup::prompts::print_error(&format!(
                    "Failed to install channel '{}': {}",
                    name, e
                ));
            }
        }
    }

    installed
}

/// Discover which tools are already installed in the tools directory.
///
/// Returns a set of tool names (the stem of .wasm files).
async fn discover_installed_tools(tools_dir: &std::path::Path) -> HashSet<String> {
    let mut names = HashSet::new();

    if !tools_dir.is_dir() {
        return names;
    }

    let mut entries = match tokio::fs::read_dir(tools_dir).await {
        Ok(e) => e,
        Err(_) => return names,
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("wasm")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.insert(stem.to_string());
        }
    }

    names
}

async fn install_selected_bundled_channels(
    channels_dir: &std::path::Path,
    selected_channels: &[String],
    already_installed: &HashSet<String>,
) -> Result<Option<Vec<String>>, SetupError> {
    let mut installed = Vec::new();
    let bundled_on_disk: HashSet<&str> = available_channel_names().iter().copied().collect();

    for name in selected_channels {
        if already_installed.contains(name) {
            continue;
        }

        // Priority 1: Extract from binary-embedded WASM (--features bundled-wasm)
        if crate::registry::bundled_wasm::is_bundled(name) {
            match crate::registry::bundled_wasm::extract_bundled(name, channels_dir).await {
                Ok(()) => {
                    installed.push(name.clone());
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %name,
                        error = %e,
                        "Bundled WASM extraction failed, trying filesystem artifacts"
                    );
                    // Fall through to filesystem path
                }
            }
        }

        // Priority 2: Copy from pre-built filesystem artifacts (channels-src/)
        if bundled_on_disk.contains(name.as_str()) {
            match install_bundled_channel(name, channels_dir, false).await {
                Ok(()) => {
                    installed.push(name.clone());
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        channel = %name,
                        error = %e,
                        "Filesystem bundled install failed, will try registry"
                    );
                    // Fall through to registry installer
                }
            }
        }
        // Channels not found via either bundled path will be tried by
        // install_selected_registry_channels next.
    }

    installed.sort();
    if installed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(installed))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tempfile::tempdir;

    use super::*;

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
