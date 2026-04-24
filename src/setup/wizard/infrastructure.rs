//! Infrastructure wizard steps: database connection and security.
//!
//! Step 1: Database connection (PostgreSQL / libSQL)
//! Step 2: Security (secrets master key)

use std::sync::Arc;

#[cfg(feature = "postgres")]
use deadpool_postgres::{Config as PoolConfig, Runtime};
#[cfg(feature = "libsql")]
use secrecy::ExposeSecret;
use secrecy::SecretString;
#[cfg(feature = "postgres")]
use tokio_postgres::NoTls;

use crate::secrets::SecretsCrypto;
use crate::settings::KeySource;
use crate::setup::prompts::{
    confirm, input, print_blank_line, print_error, print_info, print_success, select_one,
};
#[cfg(feature = "libsql")]
use crate::setup::prompts::{optional_input, secret_input};

use super::{SetupError, SetupWizard};

#[cfg(feature = "postgres")]
use super::helpers::mask_password_in_url;

impl SetupWizard {
    pub(super) async fn auto_configure_quick_runtime_defaults(&mut self) -> Result<(), SetupError> {
        self.selected_profile = self
            .config
            .profile
            .unwrap_or(super::OnboardingProfile::BuilderAndCoding);
        self.apply_profile_defaults();
        self.settings.user_timezone = Some(crate::timezone::detect_system_timezone().to_string());
        self.settings.webchat_theme = "system".to_string();
        self.settings.webchat_show_branding = false;
        self.settings.observability_backend = "none".to_string();
        self.settings.routines_enabled = true;
        self.settings.skills_enabled = true;
        self.settings.heartbeat.enabled = false;
        self.auto_configure_database().await?;
        self.auto_configure_security().await?;
        Ok(())
    }

    async fn auto_configure_database(&mut self) -> Result<(), SetupError> {
        #[cfg(feature = "postgres")]
        if let Some(url) = self
            .settings
            .database_url
            .clone()
            .or_else(|| std::env::var("DATABASE_URL").ok())
            .filter(|value| !value.trim().is_empty())
        {
            self.settings.database_backend = Some("postgres".to_string());
            self.test_database_connection_postgres(&url).await?;
            self.run_migrations_postgres().await?;
            self.settings.database_url = Some(url);
            return Ok(());
        }

        #[cfg(feature = "libsql")]
        {
            self.settings.database_backend = Some("libsql".to_string());
            let path = self
                .settings
                .libsql_path
                .clone()
                .or_else(|| std::env::var("LIBSQL_PATH").ok())
                .unwrap_or_else(|| {
                    crate::config::default_libsql_path()
                        .to_string_lossy()
                        .into()
                });
            let turso_url = self
                .settings
                .libsql_url
                .clone()
                .or_else(|| std::env::var("LIBSQL_URL").ok());
            let turso_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();
            self.test_database_connection_libsql(
                &path,
                turso_url.as_deref(),
                turso_token.as_deref(),
            )
            .await?;
            self.run_migrations_libsql().await?;
            self.settings.libsql_path = Some(path);
            self.settings.libsql_url = turso_url;
            return Ok(());
        }

        #[allow(unreachable_code)]
        Err(SetupError::Database(
            "No database backend available for quick setup".to_string(),
        ))
    }

    async fn auto_configure_security(&mut self) -> Result<(), SetupError> {
        if let Ok(env_key) = std::env::var("SECRETS_MASTER_KEY")
            && !env_key.trim().is_empty()
        {
            self.secrets_crypto = Some(Arc::new(
                SecretsCrypto::new(SecretString::from(env_key.clone()))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            ));
            self.generated_env_master_key = Some(env_key);
            self.settings.secrets_master_key_source = KeySource::Env;
            self.settings.secrets.master_key_source = crate::settings::SecretsMasterKeySource::Env;
            self.settings.secrets.allow_env_master_key = true;
            return Ok(());
        }

        if let Ok(keychain_key_bytes) = crate::platform::secure_store::get_master_key().await {
            let key_hex: String = keychain_key_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            self.secrets_crypto = Some(Arc::new(
                SecretsCrypto::new(SecretString::from(key_hex))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            ));
            self.settings.secrets_master_key_source = KeySource::Keychain;
            self.settings.secrets.master_key_source =
                crate::settings::SecretsMasterKeySource::OsSecureStore;
            self.settings.secrets.allow_env_master_key = false;
            return Ok(());
        }

        let key = crate::platform::secure_store::generate_master_key();
        if crate::platform::secure_store::store_master_key(&key)
            .await
            .is_ok()
        {
            let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
            self.secrets_crypto = Some(Arc::new(
                SecretsCrypto::new(SecretString::from(key_hex))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            ));
            self.settings.secrets_master_key_source = KeySource::Keychain;
            self.settings.secrets.master_key_source =
                crate::settings::SecretsMasterKeySource::OsSecureStore;
            self.settings.secrets.allow_env_master_key = false;
            return Ok(());
        }

        let key_hex = crate::platform::secure_store::generate_master_key_hex();
        // SAFETY: onboarding performs this env mutation during single-threaded bootstrap
        // before the runtime starts using the generated fallback key elsewhere.
        unsafe {
            std::env::set_var("SECRETS_MASTER_KEY", &key_hex);
        }
        self.secrets_crypto = Some(Arc::new(
            SecretsCrypto::new(SecretString::from(key_hex.clone()))
                .map_err(|e| SetupError::Config(e.to_string()))?,
        ));
        self.generated_env_master_key = Some(key_hex);
        self.settings.secrets_master_key_source = KeySource::Env;
        self.settings.secrets.master_key_source = crate::settings::SecretsMasterKeySource::Env;
        self.settings.secrets.allow_env_master_key = true;
        Ok(())
    }

    pub(super) async fn step_database(&mut self) -> Result<(), SetupError> {
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
                        "Unknown DATABASE_BACKEND '{}'. Defaulting to PostgreSQL.",
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

            print_info("Core runtime recommendation:");
            match self.selected_profile {
                super::OnboardingProfile::Balanced
                | super::OnboardingProfile::LocalAndPrivate
                | super::OnboardingProfile::ChannelFirst => {
                    print_success(
                        "Recommended: libSQL with a local file. It is the simplest day-one setup and does not need a separate server.",
                    );
                }
                super::OnboardingProfile::BuilderAndCoding => {
                    print_success(
                        "Recommended: libSQL unless you already need shared PostgreSQL infrastructure.",
                    );
                }
                super::OnboardingProfile::RemoteServer => {
                    print_success(
                        "Recommended: libSQL local file. Remote/headless hosts should avoid requiring a separate database before the service starts.",
                    );
                }
                super::OnboardingProfile::CustomAdvanced => {
                    print_info(
                        "Custom / Advanced leaves this open: libSQL is simpler, while PostgreSQL fits existing shared infrastructure.",
                    );
                }
            }
            print_info("Which database backend would you like to use?");
            crate::setup::prompts::print_blank_line();

            let options = &[
                "PostgreSQL  - requires a running server",
                "libSQL      - embedded SQLite, optional Turso sync",
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
    pub(super) async fn step_database_postgres(&mut self) -> Result<(), SetupError> {
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
                    print_info("Let's set up a new database URL.");
                } else {
                    print_success("Database connection successful");
                    // Run migrations to ensure new tables exist on older schemas
                    self.run_migrations_postgres().await?;
                    self.settings.database_url = Some(url.clone());
                    return Ok(());
                }
            }
        }

        crate::setup::prompts::print_blank_line();
        print_info("Enter your PostgreSQL connection URL.");
        print_info("Example: postgres://user:password@host:port/database");
        crate::setup::prompts::print_blank_line();

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
    pub(super) async fn step_database_libsql(&mut self) -> Result<(), SetupError> {
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

        crate::setup::prompts::print_blank_line();
        print_info("ThinClaw uses libSQL, an embedded SQLite database.");
        print_info("No external database server is required.");
        crate::setup::prompts::print_blank_line();

        let path_input = optional_input(
            "Database file path",
            Some(&format!("default: {}", default_path_str)),
        )
        .map_err(SetupError::Io)?;

        let db_path = path_input.unwrap_or(default_path_str.clone());

        // Ask about Turso cloud sync
        crate::setup::prompts::print_blank_line();
        let use_turso =
            confirm("Add Turso cloud sync (remote replica)?", false).map_err(SetupError::Io)?;

        let (turso_url, turso_token) = if use_turso {
            print_info("Enter your Turso database URL and auth token.");
            print_info("Example: libsql://your-db.turso.io");
            crate::setup::prompts::print_blank_line();

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
    pub(super) async fn test_database_connection_postgres(
        &mut self,
        url: &str,
    ) -> Result<(), SetupError> {
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
    pub(super) async fn test_database_connection_libsql(
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
    pub(super) async fn run_migrations_postgres(&self) -> Result<(), SetupError> {
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
    pub(super) async fn run_migrations_libsql(&self) -> Result<(), SetupError> {
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
    pub(super) async fn step_security(&mut self) -> Result<(), SetupError> {
        let secure_store = crate::platform::secure_store::display_name();
        print_info(&format!(
            "Recommended: use the {secure_store} for local installs."
        ));
        print_info("Use environment mode only when your deployment already supplies secrets.");
        crate::setup::prompts::print_blank_line();

        // Check current configuration
        let env_key_exists = std::env::var("SECRETS_MASTER_KEY").is_ok();

        if env_key_exists {
            print_info("Found SECRETS_MASTER_KEY in the environment.");
            self.settings.secrets_master_key_source = KeySource::Env;
            self.settings.secrets.master_key_source = crate::settings::SecretsMasterKeySource::Env;
            self.settings.secrets.allow_env_master_key = true;
            print_success("Security configured (env var)");
            return Ok(());
        }

        // Try to retrieve existing key from keychain. We use get_master_key()
        // instead of has_master_key() so we can cache the key bytes and build
        // SecretsCrypto eagerly, avoiding redundant keychain accesses later
        // (each access triggers macOS system dialogs).
        print_info(&format!(
            "Checking the {secure_store} for an existing master key..."
        ));
        if let Ok(keychain_key_bytes) = crate::platform::secure_store::get_master_key().await {
            let key_hex: String = keychain_key_bytes
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            self.secrets_crypto = Some(Arc::new(
                SecretsCrypto::new(SecretString::from(key_hex))
                    .map_err(|e| SetupError::Config(e.to_string()))?,
            ));

            print_info(&format!(
                "Found an existing master key in the {secure_store}."
            ));
            if confirm(&format!("Use existing {secure_store} key?"), true)
                .map_err(SetupError::Io)?
            {
                self.settings.secrets_master_key_source = KeySource::Keychain;
                self.settings.secrets.master_key_source =
                    crate::settings::SecretsMasterKeySource::OsSecureStore;
                self.settings.secrets.allow_env_master_key = false;
                print_success(&format!("Security configured ({secure_store})"));
                return Ok(());
            }
            // User declined the existing key; clear the cached crypto so a fresh
            // key can be generated below.
            self.secrets_crypto = None;
        }

        // Offer options
        crate::setup::prompts::print_blank_line();
        print_info("The secrets master key encrypts sensitive data like API tokens.");
        print_info("Choose where to store it:");
        crate::setup::prompts::print_blank_line();

        let options = [
            "OS secure store (recommended for local installs)",
            "Environment variable (for CI or Docker)",
            "Skip (disable secrets features)",
        ];

        let choice = select_one("Select storage method:", &options).map_err(SetupError::Io)?;

        match choice {
            0 => {
                // Generate and store in the OS secure store
                print_info("Generating master key...");
                let key = crate::platform::secure_store::generate_master_key();

                crate::platform::secure_store::store_master_key(&key)
                    .await
                    .map_err(|e| {
                        SetupError::Config(format!("Failed to store in {secure_store}: {}", e))
                    })?;

                // Also create crypto instance
                let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();
                self.secrets_crypto = Some(Arc::new(
                    SecretsCrypto::new(SecretString::from(key_hex))
                        .map_err(|e| SetupError::Config(e.to_string()))?,
                ));

                self.settings.secrets_master_key_source = KeySource::Keychain;
                self.settings.secrets.master_key_source =
                    crate::settings::SecretsMasterKeySource::OsSecureStore;
                self.settings.secrets.allow_env_master_key = false;
                print_success(&format!(
                    "Master key generated and saved to the {secure_store}"
                ));
            }
            1 => {
                // Env var mode
                print_info("Generate a key and add it to your environment:");
                let key_hex = crate::platform::secure_store::generate_master_key_hex();
                print_blank_line();
                if cfg!(target_os = "windows") {
                    print_info(&format!("setx SECRETS_MASTER_KEY {}", key_hex));
                    print_info(&format!("$env:SECRETS_MASTER_KEY = \"{}\"", key_hex));
                } else {
                    print_info(&format!("export SECRETS_MASTER_KEY={}", key_hex));
                }
                print_blank_line();
                if cfg!(target_os = "windows") {
                    print_info("Add this to your PowerShell profile or .env file.");
                } else {
                    print_info("Add this to your shell profile or .env file.");
                }

                self.settings.secrets_master_key_source = KeySource::Env;
                self.settings.secrets.master_key_source =
                    crate::settings::SecretsMasterKeySource::Env;
                self.settings.secrets.allow_env_master_key = true;
                print_success("Configured for environment storage");
            }
            _ => {
                self.settings.secrets_master_key_source = KeySource::None;
                self.settings.secrets.master_key_source =
                    crate::settings::SecretsMasterKeySource::None;
                self.settings.secrets.allow_env_master_key = false;
                print_info(
                    "Secrets features are disabled. Channel tokens must be set through environment variables.",
                );
            }
        }

        Ok(())
    }
}
