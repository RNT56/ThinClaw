//! Setup Review & Apply transaction.
//!
//! All wizard pages are volatile. This module is the sole durable mutation
//! boundary: it seals a secret-free plan, obtains the exclusive runtime
//! operation lease, revalidates the baseline and digest, applies in a fixed
//! order, and compensates every setup-owned write it can make.

use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(feature = "libsql")]
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use thinclaw_app::{SetupAction, SetupPlan, SetupSettingChange};

use crate::db::SettingsStore as _;
use crate::secrets::{CreateSecretParams, SecretAccessContext, SecretsCrypto, SecretsStore};
use crate::setup::prompts::{confirm, print_info, print_success, print_warning};

use super::{PendingSetupAction, SETUP_MASTER_KEY_SLOT, SetupError, SetupWizard};

const MAX_ENV_BYTES: u64 = 1024 * 1024;
const USER_ID: &str = "default";

#[derive(Default)]
struct ApplyRollback {
    env_before: Option<Option<Vec<u8>>>,
    settings_before: Option<HashMap<String, serde_json::Value>>,
    secrets_before: Vec<(String, Option<SecretString>)>,
    master_key_before: Option<Option<Vec<u8>>>,
    os_keys_before: Vec<(String, Option<String>)>,
    created_files: Vec<PathBuf>,
    created_dirs: Vec<PathBuf>,
    images_before: Vec<(String, Option<String>)>,
}

impl SetupWizard {
    pub(super) async fn review_and_apply(&mut self) -> Result<SetupPlan, SetupError> {
        self.persist_followups();
        self.capture_legacy_inline_credentials();
        let plan = self.build_setup_plan().await?;

        print_info("Review & Apply");
        print_info(&format!("Baseline revision: {}", plan.baseline_revision));
        print_info(&format!("Plan digest: {}", plan.digest));
        print_info(&format!(
            "Non-secret settings changed: {}",
            plan.settings_diff.len()
        ));
        for action in &plan.actions {
            print_info(&format!("  - {}", action_label(action)));
        }
        for warning in &plan.warnings {
            print_warning(warning);
        }
        if !plan.blockers.is_empty() {
            return Err(SetupError::Config(format!(
                "Setup plan is blocked: {}",
                plan.blockers.join("; ")
            )));
        }

        if !confirm("Apply this exact plan?", false).map_err(SetupError::Io)? {
            return Err(SetupError::Cancelled);
        }

        self.apply_sealed_plan(&plan).await?;
        print_success("Setup transaction committed");
        Ok(plan)
    }

    fn capture_legacy_inline_credentials(&mut self) {
        let channels = &mut self.settings.channels;
        for (name, value) in [
            ("discord_bot_token", channels.discord_bot_token.take()),
            ("slack_bot_token", channels.slack_bot_token.take()),
            ("slack_app_token", channels.slack_app_token.take()),
            ("bluebubbles_password", channels.bluebubbles_password.take()),
            ("gateway_auth_token", channels.gateway_auth_token.take()),
        ] {
            if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
                self.secret_draft.insert(name, SecretString::from(value));
            }
        }
        if let Some(value) = self.settings.tunnel.ngrok_token.take() {
            self.secret_draft
                .insert("tunnel_ngrok_token", SecretString::from(value));
        }
        if let Some(value) = self.settings.tunnel.cf_token.take() {
            self.secret_draft
                .insert("tunnel_cloudflare_token", SecretString::from(value));
        }
    }

    pub(super) async fn build_setup_plan(&mut self) -> Result<SetupPlan, SetupError> {
        let baseline_revision = self.durable_baseline_revision().await?;
        let before = safe_settings_map(&self.baseline_settings.to_db_map());
        let after = safe_settings_map(&self.settings.to_db_map());
        let mut changed_keys = HashSet::new();
        changed_keys.extend(before.keys().cloned());
        changed_keys.extend(after.keys().cloned());
        let mut changed_keys: Vec<String> = changed_keys
            .into_iter()
            .filter(|key| before.get(key) != after.get(key))
            .collect();
        changed_keys.sort();

        // Values stay out of the neutral plan. The DB map remains the private
        // controller payload; Review identifies exact non-secret keys.
        let settings_diff = changed_keys
            .iter()
            .map(|key| SetupSettingChange::new(key.clone(), None, None))
            .collect::<Result<Vec<_>, _>>()
            .map_err(SetupError::Config)?;

        let backend = self
            .settings
            .database_backend
            .clone()
            .unwrap_or_else(default_database_backend);
        let mut actions = Vec::new();
        if backend == "libsql"
            && let Some(path) = self.settings.libsql_path.as_ref()
            && !Path::new(path).exists()
        {
            actions.push(SetupAction::CreateDatabase {
                backend: backend.clone(),
                path: path.clone(),
            });
        }
        actions.push(SetupAction::RunMigrations {
            backend: backend.clone(),
            versions: vec!["embedded-current".to_string()],
        });

        let mut secret_slots: Vec<String> = self
            .secret_draft
            .slot_names()
            .into_iter()
            .filter(|name| !name.starts_with("__"))
            .collect();
        secret_slots.push("legacy_plaintext_settings_migration".to_string());
        secret_slots.sort();
        secret_slots.dedup();
        if !secret_slots.is_empty() {
            actions.push(SetupAction::CreateSecretBindings {
                purposes: secret_slots,
            });
        }
        if !changed_keys.is_empty() {
            actions.push(SetupAction::WriteSettings { keys: changed_keys });
        }
        for pending in &self.pending_actions {
            match pending {
                PendingSetupAction::InstallTool { name }
                | PendingSetupAction::InstallChannel { name } => {
                    actions.push(SetupAction::InstallExtension {
                        source_id: name.clone(),
                        digest: blake3::hash(name.as_bytes()).to_hex().to_string(),
                    });
                }
                PendingSetupAction::BuildWorkerImage { image } => {
                    actions.push(SetupAction::ExternalRequest {
                        host: "local-docker-daemon".to_string(),
                        purpose: format!("build reviewed worker image {image}"),
                        billable: false,
                    });
                }
            }
        }
        actions.push(SetupAction::WriteOwnedFile {
            path: crate::bootstrap::thinclaw_env_path()
                .to_string_lossy()
                .into_owned(),
        });
        actions.push(SetupAction::MarkSetupCompleted);

        let mut blockers = Vec::new();
        if self.settings.database_backend.is_none() {
            blockers.push("database backend is not selected".to_string());
        }
        let durable_secret_count = self
            .secret_draft
            .slot_names()
            .iter()
            .filter(|name| !name.starts_with("__os_api_key::") && *name != SETUP_MASTER_KEY_SLOT)
            .count();
        if durable_secret_count > 0
            && self.settings.secrets_master_key_source == crate::settings::KeySource::None
        {
            blockers
                .push("credentials were entered but encrypted secret storage is disabled".into());
        }
        if self.settings.secrets_master_key_source == crate::settings::KeySource::Env
            && std::env::var("SECRETS_MASTER_KEY")
                .ok()
                .is_none_or(|value| value.trim().is_empty())
        {
            blockers.push(
                "environment master-key mode requires an operator-supplied SECRETS_MASTER_KEY"
                    .into(),
            );
        }

        SetupPlan {
            schema_version: 1,
            baseline_revision: baseline_revision.clone(),
            digest: String::new(),
            mode: self.config.mode,
            profile: self.selected_profile.app_profile().title().to_string(),
            settings_diff,
            actions,
            warnings: vec![
                "Apply refuses a running runtime and holds the exclusive operation lease until commit or rollback."
                    .to_string(),
            ],
            blockers,
            continuation: self.config.invocation.continuation.clone(),
        }
        .seal()
        .map_err(|error| SetupError::Config(format!("failed to seal setup plan: {error}")))
    }

    async fn durable_baseline_revision(&mut self) -> Result<String, SetupError> {
        let mut hasher = blake3::Hasher::new();
        let env_path = crate::bootstrap::thinclaw_env_path();
        match thinclaw_platform::read_regular_file_bounded(&env_path, MAX_ENV_BYTES) {
            Ok(bytes) => {
                hasher.update(b"env:present\0");
                hasher.update(&bytes);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"env:absent\0");
            }
            Err(error) => return Err(SetupError::Io(error)),
        }

        #[cfg(feature = "postgres")]
        if self.settings.database_backend.as_deref() == Some("postgres") {
            if self.db_pool.is_none() {
                let url = self.settings.database_url.clone().ok_or_else(|| {
                    SetupError::Database("PostgreSQL URL is not configured".to_string())
                })?;
                self.test_database_connection_postgres(&url).await?;
            }
            if let Some(pool) = &self.db_pool {
                let store = crate::db::postgres::PgBackend::from_pool(pool.clone());
                let map = store.get_all_settings(USER_ID).await.map_err(|error| {
                    SetupError::Database(format!("failed to read setup baseline: {error}"))
                })?;
                hash_settings_map(&mut hasher, &map)?;
            }
        }

        #[cfg(feature = "libsql")]
        if self.settings.database_backend.as_deref() == Some("libsql")
            && let Some(path) = self.settings.libsql_path.as_deref()
        {
            hash_file_if_present(&mut hasher, Path::new(path))?;
        }
        Ok(hasher.finalize().to_hex().to_string())
    }

    async fn apply_sealed_plan(&mut self, plan: &SetupPlan) -> Result<(), SetupError> {
        if !plan.digest_matches(&plan.digest) {
            return Err(SetupError::Config(
                "setup plan digest is invalid".to_string(),
            ));
        }
        let _lease =
            crate::runtime_lease::RuntimeOperationLease::acquire_default().map_err(|error| {
                SetupError::Config(format!(
                    "setup Apply requires a stopped runtime and exclusive state lease: {error}"
                ))
            })?;
        let actual_baseline = self.durable_baseline_revision().await?;
        if actual_baseline != plan.baseline_revision {
            return Err(SetupError::Config(format!(
                "setup baseline changed (expected {}, found {}); review a fresh plan",
                plan.baseline_revision, actual_baseline
            )));
        }

        let mut rollback = ApplyRollback::default();
        let result = self.apply_inner(&mut rollback).await;
        if let Err(error) = result {
            let rollback_errors = self.rollback_setup(&mut rollback).await;
            if rollback_errors.is_empty() {
                return Err(error);
            }
            return Err(SetupError::Config(format!(
                "{error}; rollback was partial: {}",
                rollback_errors.join("; ")
            )));
        }
        Ok(())
    }

    async fn apply_inner(&mut self, rollback: &mut ApplyRollback) -> Result<(), SetupError> {
        let env_path = crate::bootstrap::thinclaw_env_path();
        rollback.env_before = Some(
            match thinclaw_platform::read_regular_file_bounded(&env_path, MAX_ENV_BYTES) {
                Ok(bytes) => Some(bytes),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(SetupError::Io(error)),
            },
        );
        if rollback.env_before.as_ref().is_some_and(Option::is_none)
            && let Some(parent) = env_path.parent()
        {
            record_missing_directories(rollback, parent);
        }

        #[cfg(feature = "libsql")]
        if self.settings.database_backend.as_deref() == Some("libsql")
            && let Some(path) = self.settings.libsql_path.as_deref()
        {
            let path = PathBuf::from(path);
            if !path.exists() {
                rollback.created_files.push(path.clone());
                rollback
                    .created_files
                    .push(PathBuf::from(format!("{}-wal", path.display())));
                rollback
                    .created_files
                    .push(PathBuf::from(format!("{}-shm", path.display())));
                if let Some(parent) = path.parent() {
                    record_missing_directories(rollback, parent);
                }
            }
        }

        self.prepare_apply_database().await?;
        rollback.settings_before = Some(self.read_settings_snapshot().await?);
        if let Some(settings_before) = rollback.settings_before.as_ref() {
            self.capture_plaintext_credentials_from_map(settings_before);
        }

        let crypto = self.prepare_apply_master_key(rollback).await?;
        let persistent_store = if let Some(crypto) = crypto {
            self.persistent_secrets_store(crypto).await?
        } else {
            None
        };
        self.apply_os_api_keys(rollback).await?;
        if let Some(store) = persistent_store.as_ref() {
            self.apply_secret_draft(store, rollback).await?;
        } else if self.secret_draft.slot_names().iter().any(|name| {
            !name.starts_with("__os_api_key::") && name.as_str() != SETUP_MASTER_KEY_SLOT
        }) {
            return Err(SetupError::Config(
                "secret draft cannot be committed without encrypted secret storage".to_string(),
            ));
        }
        self.purge_plaintext_credential_settings().await?;

        self.apply_pending_extensions(rollback).await?;

        self.settings.onboard_completed = false;
        self.persist_settings().await.and_then(|saved| {
            if saved {
                Ok(())
            } else {
                Err(SetupError::Database(
                    "No database connection, cannot save settings".to_string(),
                ))
            }
        })?;
        self.write_bootstrap_env()?;

        // The completion marker is deliberately the final durable write.
        self.settings.onboard_completed = true;
        self.persist_settings().await.and_then(|saved| {
            if saved {
                Ok(())
            } else {
                Err(SetupError::Database(
                    "No database connection, cannot mark setup complete".to_string(),
                ))
            }
        })?;
        self.write_bootstrap_env()?;
        Ok(())
    }

    async fn prepare_apply_database(&mut self) -> Result<(), SetupError> {
        match self.settings.database_backend.as_deref() {
            #[cfg(feature = "postgres")]
            Some("postgres") | Some("postgresql") => {
                if self.db_pool.is_none() {
                    let url = self.settings.database_url.clone().ok_or_else(|| {
                        SetupError::Database("PostgreSQL URL is not configured".to_string())
                    })?;
                    self.test_database_connection_postgres(&url).await?;
                }
                self.run_migrations_postgres().await
            }
            #[cfg(feature = "libsql")]
            Some("libsql") | Some("sqlite") | Some("turso") => {
                if self.db_backend.is_none() {
                    let path = self.settings.libsql_path.clone().ok_or_else(|| {
                        SetupError::Database("libSQL path is not configured".to_string())
                    })?;
                    let url = self.settings.libsql_url.clone();
                    let draft_token = self.secret_draft.value_for_apply("libsql_auth_token");
                    let env_token = std::env::var("LIBSQL_AUTH_TOKEN").ok();
                    let token = draft_token
                        .as_ref()
                        .map(|value| value.expose_secret())
                        .or(env_token.as_deref());
                    self.test_database_connection_libsql(&path, url.as_deref(), token)
                        .await?;
                }
                self.run_migrations_libsql().await
            }
            Some(other) => Err(SetupError::Database(format!(
                "unsupported setup database backend '{other}'"
            ))),
            None => Err(SetupError::Database(
                "database backend is not configured".to_string(),
            )),
        }
    }

    async fn prepare_apply_master_key(
        &mut self,
        rollback: &mut ApplyRollback,
    ) -> Result<Option<Arc<SecretsCrypto>>, SetupError> {
        use crate::settings::KeySource;
        let key = match self.settings.secrets_master_key_source {
            KeySource::Keychain => {
                if let Some(pending) = self.secret_draft.value_for_apply(SETUP_MASTER_KEY_SLOT) {
                    let previous = crate::platform::secure_store::get_master_key().await.ok();
                    rollback.master_key_before = Some(previous);
                    let bytes = decode_hex(pending.expose_secret())?;
                    crate::platform::secure_store::store_master_key(&bytes)
                        .await
                        .map_err(|error| {
                            SetupError::Config(format!("failed to store setup master key: {error}"))
                        })?;
                    let verified = crate::platform::secure_store::get_master_key()
                        .await
                        .map_err(|error| {
                            SetupError::Config(format!(
                                "failed to verify setup master key: {error}"
                            ))
                        })?;
                    if verified != bytes {
                        return Err(SetupError::Config(
                            "OS secure-store master-key verification failed".to_string(),
                        ));
                    }
                    pending
                } else {
                    let bytes = crate::platform::secure_store::get_master_key()
                        .await
                        .map_err(|error| {
                            SetupError::Config(format!("failed to load OS master key: {error}"))
                        })?;
                    SecretString::from(
                        bytes
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect::<String>(),
                    )
                }
            }
            KeySource::Env => {
                SecretString::from(std::env::var("SECRETS_MASTER_KEY").map_err(|_| {
                    SetupError::Config(
                        "SECRETS_MASTER_KEY must be supplied by the operator".to_string(),
                    )
                })?)
            }
            KeySource::None => return Ok(None),
        };
        let crypto =
            Arc::new(SecretsCrypto::new(key).map_err(|error| {
                SetupError::Config(format!("invalid setup master key: {error}"))
            })?);
        self.secrets_crypto = Some(Arc::clone(&crypto));
        Ok(Some(crypto))
    }

    async fn persistent_secrets_store(
        &mut self,
        crypto: Arc<SecretsCrypto>,
    ) -> Result<Option<Arc<dyn SecretsStore>>, SetupError> {
        #[cfg(all(feature = "postgres", feature = "libsql"))]
        {
            if self.settings.database_backend.as_deref() == Some("libsql") {
                return self.create_libsql_secrets_store(&crypto);
            }
            return self.create_postgres_secrets_store(&crypto).await;
        }
        #[cfg(all(feature = "postgres", not(feature = "libsql")))]
        {
            self.create_postgres_secrets_store(&crypto).await
        }
        #[cfg(all(feature = "libsql", not(feature = "postgres")))]
        {
            self.create_libsql_secrets_store(&crypto)
        }
        #[cfg(not(any(feature = "postgres", feature = "libsql")))]
        {
            let _ = crypto;
            Ok(None)
        }
    }

    async fn apply_secret_draft(
        &self,
        store: &Arc<dyn SecretsStore>,
        rollback: &mut ApplyRollback,
    ) -> Result<(), SetupError> {
        for (name, value) in self.secret_draft.values_for_apply() {
            if name == SETUP_MASTER_KEY_SLOT || name.starts_with("__os_api_key::") {
                continue;
            }
            let before = store
                .get_decrypted(USER_ID, &name)
                .await
                .ok()
                .map(|value| SecretString::from(value.expose().to_string()));
            rollback.secrets_before.push((name.clone(), before));
            store
                .create(
                    USER_ID,
                    CreateSecretParams::new(&name, value.expose_secret()),
                )
                .await
                .map_err(|error| {
                    SetupError::Config(format!(
                        "failed to persist credential slot '{name}': {error}"
                    ))
                })?;
            store
                .get_for_injection(
                    USER_ID,
                    &name,
                    SecretAccessContext::new("setup.apply", "verify"),
                )
                .await
                .map_err(|error| {
                    SetupError::Config(format!(
                        "failed to verify credential slot '{name}': {error}"
                    ))
                })?;
        }
        Ok(())
    }

    async fn apply_os_api_keys(&self, rollback: &mut ApplyRollback) -> Result<(), SetupError> {
        for (name, value) in self.secret_draft.values_for_apply() {
            let Some(account) = name.strip_prefix("__os_api_key::") else {
                continue;
            };
            rollback.os_keys_before.push((
                account.to_string(),
                crate::platform::secure_store::get_api_key(account).await,
            ));
            crate::platform::secure_store::store_api_key(account, value.expose_secret())
                .await
                .map_err(|error| {
                    SetupError::Config(format!("failed to store worker credential: {error}"))
                })?;
        }
        Ok(())
    }

    pub(super) fn capture_plaintext_credentials_from_map(
        &self,
        settings: &HashMap<String, serde_json::Value>,
    ) {
        for (key, value) in settings {
            if !credential_shaped_key(key) {
                continue;
            }
            let Some(value) = value.as_str().filter(|value| !value.trim().is_empty()) else {
                continue;
            };
            let purpose = key
                .rsplit(['.', ':'])
                .next()
                .unwrap_or(key)
                .to_ascii_lowercase();
            self.secret_draft
                .insert(purpose, SecretString::from(value.to_string()));
        }
    }

    async fn purge_plaintext_credential_settings(&self) -> Result<(), SetupError> {
        let current = self.read_settings_snapshot().await?;
        let keys: Vec<String> = current
            .keys()
            .filter(|key| credential_shaped_key(key))
            .cloned()
            .collect();
        #[cfg(feature = "postgres")]
        if let Some(pool) = &self.db_pool {
            let store = crate::db::postgres::PgBackend::from_pool(pool.clone());
            for key in &keys {
                store
                    .delete_setting(USER_ID, key)
                    .await
                    .map_err(|error| SetupError::Database(error.to_string()))?;
            }
            return Ok(());
        }
        #[cfg(feature = "libsql")]
        if let Some(store) = &self.db_backend {
            for key in &keys {
                store
                    .delete_setting(USER_ID, key)
                    .await
                    .map_err(|error| SetupError::Database(error.to_string()))?;
            }
            return Ok(());
        }
        Err(SetupError::Database(
            "settings store is unavailable".to_string(),
        ))
    }

    async fn apply_pending_extensions(
        &self,
        rollback: &mut ApplyRollback,
    ) -> Result<(), SetupError> {
        if self.pending_actions.is_empty() {
            return Ok(());
        }
        let tools_dir = crate::platform::state_paths().tools_dir;
        let channels_dir = crate::platform::state_paths().channels_dir;
        for dir in [&tools_dir, &channels_dir] {
            record_missing_directories(rollback, dir);
        }
        for action in &self.pending_actions {
            match action {
                PendingSetupAction::InstallTool { name }
                | PendingSetupAction::InstallChannel { name } => {
                    let target_dir = if matches!(action, PendingSetupAction::InstallTool { .. }) {
                        &tools_dir
                    } else {
                        &channels_dir
                    };
                    let before = directory_files(target_dir)?;
                    let catalog = super::helpers::load_registry_catalog().ok_or_else(|| {
                        SetupError::Config("extension registry is unavailable at Apply".to_string())
                    })?;
                    let manifest = catalog.get(name).ok_or_else(|| {
                        SetupError::Config(format!(
                            "extension '{name}' is no longer in the catalog"
                        ))
                    })?;
                    let repo_root = catalog.root().parent().unwrap_or(catalog.root());
                    let installer = crate::registry::installer::RegistryInstaller::new(
                        repo_root.to_path_buf(),
                        tools_dir.clone(),
                        channels_dir.clone(),
                    );
                    installer
                        .install_with_source_fallback(manifest, false)
                        .await
                        .map_err(|error| {
                            SetupError::Config(format!("failed to install '{name}': {error}"))
                        })?;
                    let after = directory_files(target_dir)?;
                    rollback
                        .created_files
                        .extend(after.difference(&before).cloned());
                }
                PendingSetupAction::BuildWorkerImage { image } => {
                    let mut inspect = thinclaw_platform::tokio_process_command!(
                        "src.setup.wizard.transaction.docker_inspect",
                        "docker"
                    );
                    inspect.args(["image", "inspect", "--format", "{{.Id}}", image]);
                    let previous_image_id = match thinclaw_platform::bounded_command_output(
                        &mut inspect,
                        Duration::from_secs(30),
                        16 * 1024,
                        16 * 1024,
                    )
                    .await
                    {
                        Ok(output) if output.status.success() => {
                            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
                                .filter(|value| !value.is_empty())
                        }
                        Ok(_) => None,
                        Err(error) => {
                            return Err(SetupError::Config(format!(
                                "failed to inspect worker image before build: {error}"
                            )));
                        }
                    };
                    let mut command = thinclaw_platform::tokio_process_command!(
                        "src.setup.wizard.transaction.docker_build",
                        "docker"
                    );
                    command
                        .args(["build", "-f", "Dockerfile.worker", "-t", image, "."])
                        .current_dir(std::env::current_dir().map_err(SetupError::Io)?)
                        .stdin(std::process::Stdio::inherit())
                        .stdout(std::process::Stdio::inherit())
                        .stderr(std::process::Stdio::inherit());
                    let mut child = thinclaw_platform::OwnedChild::spawn(&mut command)
                        .map_err(SetupError::Io)?;
                    let status = tokio::time::timeout(Duration::from_secs(30 * 60), child.wait())
                        .await
                        .map_err(|_| {
                            SetupError::Config("worker image build timed out".to_string())
                        })?
                        .map_err(SetupError::Io)?;
                    if !status.success() {
                        return Err(SetupError::Config(format!(
                            "worker image build failed with status {status}"
                        )));
                    }
                    rollback
                        .images_before
                        .push((image.clone(), previous_image_id));
                }
            }
        }
        Ok(())
    }

    async fn read_settings_snapshot(
        &self,
    ) -> Result<HashMap<String, serde_json::Value>, SetupError> {
        #[cfg(feature = "postgres")]
        if let Some(pool) = &self.db_pool {
            return crate::db::postgres::PgBackend::from_pool(pool.clone())
                .get_all_settings(USER_ID)
                .await
                .map_err(|error| SetupError::Database(error.to_string()));
        }
        #[cfg(feature = "libsql")]
        if let Some(backend) = &self.db_backend {
            return backend
                .get_all_settings(USER_ID)
                .await
                .map_err(|error| SetupError::Database(error.to_string()));
        }
        Err(SetupError::Database(
            "settings store is unavailable".to_string(),
        ))
    }

    async fn restore_settings_snapshot(
        &self,
        before: &HashMap<String, serde_json::Value>,
    ) -> Result<(), SetupError> {
        #[cfg(feature = "postgres")]
        if let Some(pool) = &self.db_pool {
            let store = crate::db::postgres::PgBackend::from_pool(pool.clone());
            let current = store
                .get_all_settings(USER_ID)
                .await
                .map_err(|error| SetupError::Database(error.to_string()))?;
            for key in current.keys().filter(|key| !before.contains_key(*key)) {
                store
                    .delete_setting(USER_ID, key)
                    .await
                    .map_err(|error| SetupError::Database(error.to_string()))?;
            }
            return store
                .set_all_settings(USER_ID, before)
                .await
                .map_err(|error| SetupError::Database(error.to_string()));
        }
        #[cfg(feature = "libsql")]
        if let Some(store) = &self.db_backend {
            let current = store
                .get_all_settings(USER_ID)
                .await
                .map_err(|error| SetupError::Database(error.to_string()))?;
            for key in current.keys().filter(|key| !before.contains_key(*key)) {
                store
                    .delete_setting(USER_ID, key)
                    .await
                    .map_err(|error| SetupError::Database(error.to_string()))?;
            }
            return store
                .set_all_settings(USER_ID, before)
                .await
                .map_err(|error| SetupError::Database(error.to_string()));
        }
        Err(SetupError::Database(
            "settings store is unavailable".to_string(),
        ))
    }

    async fn rollback_setup(&mut self, rollback: &mut ApplyRollback) -> Vec<String> {
        let mut errors = Vec::new();

        if let Some(crypto) = self.secrets_crypto.clone() {
            match self.persistent_secrets_store(crypto).await {
                Ok(Some(store)) => {
                    for (name, before) in rollback.secrets_before.iter().rev() {
                        let result = match before {
                            Some(value) => store
                                .create(
                                    USER_ID,
                                    CreateSecretParams::new(name, value.expose_secret()),
                                )
                                .await
                                .map(|_| ()),
                            None => store.delete(USER_ID, name).await.map(|_| ()),
                        };
                        if let Err(error) = result {
                            errors.push(format!(
                                "could not restore credential slot '{name}': {error}"
                            ));
                        }
                    }
                }
                Ok(None) => errors.push("credential store was unavailable during rollback".into()),
                Err(error) => errors.push(format!(
                    "credential store could not be reopened during rollback: {error}"
                )),
            }
        }

        for (account, before) in rollback.os_keys_before.iter().rev() {
            let result = match before {
                Some(value) => crate::platform::secure_store::store_api_key(account, value).await,
                None => crate::platform::secure_store::delete_api_key(account).await,
            };
            if let Err(error) = result {
                errors.push(format!(
                    "could not restore worker credential account: {error}"
                ));
            }
        }
        if let Some(before) = rollback.master_key_before.take() {
            let result = match before {
                Some(bytes) => crate::platform::secure_store::store_master_key(&bytes).await,
                None => crate::platform::secure_store::delete_master_key().await,
            };
            if let Err(error) = result {
                errors.push(format!("could not restore master key: {error}"));
            }
        }
        if let Some(before) = rollback.settings_before.as_ref()
            && let Err(error) = self.restore_settings_snapshot(before).await
        {
            errors.push(format!("could not restore settings: {error}"));
        }
        if let Some(before) = rollback.env_before.take() {
            let path = crate::bootstrap::thinclaw_env_path();
            let result = match before {
                Some(bytes) => thinclaw_platform::write_private_file_atomic(&path, &bytes, true),
                None => match std::fs::remove_file(&path) {
                    Ok(()) => Ok(()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(error) => Err(error),
                },
            };
            if let Err(error) = result {
                errors.push(format!("could not restore {}: {error}", path.display()));
            }
        }

        // Logical state is restored while its backend still exists. Only then
        // remove artifacts created by this Apply attempt.
        for (image, previous_image_id) in rollback.images_before.iter().rev() {
            let mut command = thinclaw_platform::tokio_process_command!(
                "src.setup.wizard.transaction.docker_rollback",
                "docker"
            );
            command.args(["image", "rm", image]);
            match thinclaw_platform::bounded_command_output(
                &mut command,
                Duration::from_secs(60),
                64 * 1024,
                64 * 1024,
            )
            .await
            {
                Ok(output) if output.status.success() => {}
                Ok(_) => errors.push(format!("could not remove worker image '{image}'")),
                Err(error) => {
                    errors.push(format!("could not remove worker image '{image}': {error}"))
                }
            }
            if let Some(previous_image_id) = previous_image_id {
                let mut command = thinclaw_platform::tokio_process_command!(
                    "src.setup.wizard.transaction.docker_restore_tag",
                    "docker"
                );
                command.args(["image", "tag", previous_image_id, image]);
                match thinclaw_platform::bounded_command_output(
                    &mut command,
                    Duration::from_secs(30),
                    16 * 1024,
                    16 * 1024,
                )
                .await
                {
                    Ok(output) if output.status.success() => {}
                    Ok(_) => errors.push(format!(
                        "could not restore previous worker image tag '{image}'"
                    )),
                    Err(error) => errors.push(format!(
                        "could not restore previous worker image tag '{image}': {error}"
                    )),
                }
            }
        }
        for path in rollback.created_files.iter().rev() {
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                errors.push(format!("could not remove {}: {error}", path.display()));
            }
        }
        for path in rollback.created_dirs.iter().rev() {
            if let Err(error) = std::fs::remove_dir(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                errors.push(format!("could not remove {}: {error}", path.display()));
            }
        }
        errors
    }
}

fn safe_settings_map(
    map: &HashMap<String, serde_json::Value>,
) -> BTreeMap<String, serde_json::Value> {
    map.iter()
        .filter(|(key, _)| !credential_shaped_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

pub(super) fn credential_shaped_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "secret",
        "token",
        "password",
        "api_key",
        "private_key",
        "credential",
    ]
    .iter()
    .any(|marker| key.contains(marker))
}

#[cfg(feature = "postgres")]
fn hash_settings_map(
    hasher: &mut blake3::Hasher,
    map: &HashMap<String, serde_json::Value>,
) -> Result<(), SetupError> {
    let ordered: BTreeMap<_, _> = map.iter().collect();
    let bytes = serde_json::to_vec(&ordered).map_err(|error| {
        SetupError::Config(format!("failed to hash settings baseline: {error}"))
    })?;
    hasher.update(b"settings\0");
    hasher.update(&bytes);
    Ok(())
}

#[cfg(feature = "libsql")]
fn hash_file_if_present(hasher: &mut blake3::Hasher, path: &Path) -> Result<(), SetupError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            hasher.update(b"database:absent\0");
            return Ok(());
        }
        Err(error) => return Err(SetupError::Io(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SetupError::Config(format!(
            "database baseline target is not a regular file: {}",
            path.display()
        )));
    }
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    hasher.update(b"database:present\0");
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(())
}

fn decode_hex(value: &str) -> Result<Vec<u8>, SetupError> {
    if !value.len().is_multiple_of(2) || value.is_empty() {
        return Err(SetupError::Config("invalid staged master key".to_string()));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            std::str::from_utf8(chunk)
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| SetupError::Config("invalid staged master key".to_string()))
        })
        .collect()
}

fn directory_files(path: &Path) -> Result<HashSet<PathBuf>, SetupError> {
    if !path.exists() {
        return Ok(HashSet::new());
    }
    let mut files = HashSet::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_file() {
            files.insert(entry.path());
        }
    }
    Ok(files)
}

fn record_missing_directories(rollback: &mut ApplyRollback, leaf: &Path) {
    let mut missing = Vec::new();
    let mut current = Some(leaf);
    while let Some(path) = current {
        if path.exists() {
            break;
        }
        missing.push(path.to_path_buf());
        current = path.parent();
    }
    missing.reverse();
    for path in missing {
        if !rollback.created_dirs.contains(&path) {
            rollback.created_dirs.push(path);
        }
    }
}

fn default_database_backend() -> String {
    if cfg!(feature = "libsql") {
        "libsql".to_string()
    } else {
        "postgres".to_string()
    }
}

fn action_label(action: &SetupAction) -> String {
    match action {
        SetupAction::CreateDatabase { backend, path } => {
            format!("create {backend} database at {path}")
        }
        SetupAction::RunMigrations { backend, .. } => format!("run {backend} migrations"),
        SetupAction::WriteSettings { keys } => format!("write {} setting keys", keys.len()),
        SetupAction::CreateSecretBindings { purposes } => {
            format!("create {} credential bindings", purposes.len())
        }
        SetupAction::InstallExtension { source_id, .. } => {
            format!("install extension {source_id}")
        }
        SetupAction::WriteOwnedFile { path } => format!("write owned file {path}"),
        SetupAction::BindListener { origin } => format!("bind listener {origin}"),
        SetupAction::ExternalRequest { host, purpose, .. } => {
            format!("external action via {host}: {purpose}")
        }
        SetupAction::MarkSetupCompleted => "mark setup completed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_directory_rollback_tracks_ancestors_parent_first_without_duplicates() {
        let root = tempfile::tempdir().expect("temporary root");
        let first_leaf = root.path().join("setup").join("extensions");
        let second_leaf = root.path().join("setup").join("database");
        let mut rollback = ApplyRollback::default();

        record_missing_directories(&mut rollback, &first_leaf);
        record_missing_directories(&mut rollback, &second_leaf);
        record_missing_directories(&mut rollback, &first_leaf);

        assert_eq!(
            rollback.created_dirs,
            vec![root.path().join("setup"), first_leaf, second_leaf,]
        );
    }

    #[test]
    fn missing_directory_rollback_stops_at_existing_parent() {
        let root = tempfile::tempdir().expect("temporary root");
        let existing = root.path().join("existing");
        std::fs::create_dir(&existing).expect("create existing parent");
        let leaf = existing.join("nested").join("leaf");
        let mut rollback = ApplyRollback::default();

        record_missing_directories(&mut rollback, &leaf);

        assert_eq!(rollback.created_dirs, vec![existing.join("nested"), leaf]);
    }
}
