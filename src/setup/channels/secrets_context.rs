//! Secret persistence context used by every channel setup flow.

use std::sync::Arc;

use secrecy::{ExposeSecret, SecretString};

#[cfg(feature = "postgres")]
use crate::secrets::SecretsCrypto;
use crate::secrets::{CreateSecretParams, SecretsStore};

use super::ChannelSetupError;

/// Context for saving secrets during setup.
pub struct SecretsContext {
    store: Arc<dyn SecretsStore>,
    user_id: String,
}

impl SecretsContext {
    /// Create a new secrets context from a trait-object store.
    pub fn from_store(store: Arc<dyn SecretsStore>, user_id: &str) -> Self {
        Self {
            store,
            user_id: user_id.to_string(),
        }
    }

    /// Create a new secrets context from a PostgreSQL pool and crypto.
    #[cfg(feature = "postgres")]
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>, user_id: &str) -> Self {
        Self {
            store: Arc::new(crate::secrets::PostgresSecretsStore::new(pool, crypto)),
            user_id: user_id.to_string(),
        }
    }

    /// Save a secret to the database.
    pub async fn save_secret(
        &self,
        name: &str,
        value: &SecretString,
    ) -> Result<(), ChannelSetupError> {
        let params = CreateSecretParams::new(name, value.expose_secret());

        self.store
            .create(&self.user_id, params)
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to save secret: {}", e)))?;

        Ok(())
    }

    /// Check if a secret exists.
    pub async fn secret_exists(&self, name: &str) -> bool {
        match self.store.exists(&self.user_id, name).await {
            Ok(exists) => exists,
            Err(e) => {
                tracing::warn!(secret = name, error = %e, "Failed to check if secret exists, assuming absent");
                false
            }
        }
    }

    /// Read a secret from the database (decrypted).
    pub async fn get_secret(&self, name: &str) -> Result<SecretString, ChannelSetupError> {
        let decrypted = self
            .store
            .get_for_injection(
                &self.user_id,
                name,
                crate::secrets::SecretAccessContext::new("setup.channels", "setup_validation"),
            )
            .await
            .map_err(|e| ChannelSetupError::Secrets(format!("Failed to read secret: {}", e)))?;
        Ok(SecretString::from(decrypted.expose().to_string()))
    }
}
