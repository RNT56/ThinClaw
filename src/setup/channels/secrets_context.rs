//! Secret persistence context used by every channel setup flow.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret, SecretString};

#[cfg(feature = "postgres")]
use crate::secrets::SecretsCrypto;
use crate::secrets::{CreateSecretParams, SecretsStore};

use super::ChannelSetupError;

/// Context for saving secrets during setup.
pub struct SecretsContext {
    backend: SecretsContextBackend,
    user_id: String,
}

enum SecretsContextBackend {
    Persistent(Arc<dyn SecretsStore>),
    Draft(Arc<Mutex<BTreeMap<String, SecretString>>>),
}

/// Top-level setup-owned credential draft.
///
/// This object deliberately has no `Clone` or serialization implementation.
/// Page checkpoints contain only its secret-free set of slot names; values
/// remain in this controller-owned, redacted container until Apply.
pub struct SetupSecretDraft {
    values: Arc<Mutex<BTreeMap<String, SecretString>>>,
}

impl fmt::Debug for SetupSecretDraft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SetupSecretDraft")
            .field("slots", &self.slot_names())
            .field("values", &"[REDACTED]")
            .finish()
    }
}

impl Default for SetupSecretDraft {
    fn default() -> Self {
        Self::new()
    }
}

impl SetupSecretDraft {
    pub fn new() -> Self {
        Self {
            values: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn context(&self, user_id: &str) -> SecretsContext {
        SecretsContext {
            backend: SecretsContextBackend::Draft(Arc::clone(&self.values)),
            user_id: user_id.to_string(),
        }
    }

    pub fn insert(&self, name: impl Into<String>, value: SecretString) {
        self.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name.into(), value);
    }

    pub fn contains(&self, name: &str) -> bool {
        self.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_key(name)
    }

    pub fn slot_names(&self) -> BTreeSet<String> {
        self.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    pub fn retain_slots(&self, slots: &BTreeSet<String>) {
        self.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|name, _| slots.contains(name));
    }

    /// Clone secret values only at the Apply boundary. The returned values
    /// remain secret-typed and are never added to a plan, checkpoint, or log.
    pub fn values_for_apply(&self) -> Vec<(String, SecretString)> {
        self.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    }

    pub fn value_for_apply(&self, name: &str) -> Option<SecretString> {
        self.values
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(name)
            .cloned()
    }
}

impl SecretsContext {
    /// Create a new secrets context from a trait-object store.
    pub fn from_store(store: Arc<dyn SecretsStore>, user_id: &str) -> Self {
        Self {
            backend: SecretsContextBackend::Persistent(store),
            user_id: user_id.to_string(),
        }
    }

    /// Create a new secrets context from a PostgreSQL pool and crypto.
    #[cfg(feature = "postgres")]
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>, user_id: &str) -> Self {
        Self {
            backend: SecretsContextBackend::Persistent(Arc::new(
                crate::secrets::PostgresSecretsStore::new(pool, crypto),
            )),
            user_id: user_id.to_string(),
        }
    }

    /// Save a secret to the database.
    pub async fn save_secret(
        &self,
        name: &str,
        value: &SecretString,
    ) -> Result<(), ChannelSetupError> {
        match &self.backend {
            SecretsContextBackend::Persistent(store) => {
                let params = CreateSecretParams::new(name, value.expose_secret());
                store.create(&self.user_id, params).await.map_err(|e| {
                    ChannelSetupError::Secrets(format!("Failed to save secret: {}", e))
                })?;
            }
            SecretsContextBackend::Draft(values) => {
                values
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(name.to_string(), value.clone());
            }
        }

        Ok(())
    }

    /// Check if a secret exists.
    pub async fn secret_exists(&self, name: &str) -> bool {
        let result = match &self.backend {
            SecretsContextBackend::Persistent(store) => store.exists(&self.user_id, name).await,
            SecretsContextBackend::Draft(values) => Ok(values
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(name)),
        };
        match result {
            Ok(exists) => exists,
            Err(e) => {
                tracing::warn!(secret = name, error = %e, "Failed to check if secret exists, assuming absent");
                false
            }
        }
    }

    /// Read a secret from the database (decrypted).
    pub async fn get_secret(&self, name: &str) -> Result<SecretString, ChannelSetupError> {
        match &self.backend {
            SecretsContextBackend::Persistent(store) => {
                let decrypted = store
                    .get_for_injection(
                        &self.user_id,
                        name,
                        crate::secrets::SecretAccessContext::new(
                            "setup.channels",
                            "setup_validation",
                        ),
                    )
                    .await
                    .map_err(|e| {
                        ChannelSetupError::Secrets(format!("Failed to read secret: {}", e))
                    })?;
                Ok(SecretString::from(decrypted.expose().to_string()))
            }
            SecretsContextBackend::Draft(values) => values
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(name)
                .cloned()
                .ok_or_else(|| {
                    ChannelSetupError::Secrets(format!("Secret slot '{name}' is not configured"))
                }),
        }
    }
}
