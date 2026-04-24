//! Secret storage with PostgreSQL persistence.
//!
//! Provides CRUD operations for encrypted secrets. The store handles:
//! - Encryption/decryption via SecretsCrypto
//! - Expiration checking
//! - Usage tracking
//! - Access control (which secrets a tool can use)

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use secrecy::ExposeSecret;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::secrets::crypto::SecretsCrypto;
use crate::secrets::types::{
    CURRENT_AAD_VERSION, CURRENT_CIPHER, CURRENT_ENCRYPTION_VERSION, CURRENT_KDF,
    CURRENT_KEY_VERSION, CreateSecretParams, DecryptedSecret, MasterKeyRotationReport, Secret,
    SecretAccessContext, SecretError, SecretRef,
};

/// Trait for secret storage operations.
///
/// Allows for different implementations (PostgreSQL, in-memory for testing).
#[async_trait]
pub trait SecretsStore: Send + Sync {
    /// Store a new secret.
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError>;

    /// Get a secret by name (encrypted form).
    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError>;

    /// Get and decrypt a secret.
    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError>;

    /// Get and decrypt a secret for a concrete runtime operation.
    async fn get_for_injection(
        &self,
        user_id: &str,
        name: &str,
        context: SecretAccessContext,
    ) -> Result<DecryptedSecret, SecretError>;

    /// Check if a secret exists.
    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError>;

    /// List all secret references for a user (no values).
    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError>;

    /// Delete a secret.
    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError>;

    /// Re-encrypt all active v2 secrets with a freshly generated master key.
    async fn rotate_master_key(
        &self,
        new_crypto: Arc<SecretsCrypto>,
    ) -> Result<MasterKeyRotationReport, SecretError>;

    /// Persist a host-boundary leak detection event without storing plaintext.
    async fn record_leak_detection_event(
        &self,
        _user_id: &str,
        _source: &str,
        _action_taken: &str,
        _content_hash: &str,
        _redacted_preview: Option<&str>,
    ) -> Result<(), SecretError> {
        Ok(())
    }

    /// Update secret usage tracking.
    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError>;

    /// Check if a secret is accessible by a tool (based on allowed_secrets).
    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError>;
}

fn secret_aad(secret: &Secret) -> Vec<u8> {
    let provider = secret.provider.as_deref().unwrap_or("");
    format!(
        "v{}|user={}|name={}|provider={}|key_version={}|encryption_version={}",
        secret.aad_version,
        secret.user_id,
        secret.name,
        provider,
        secret.key_version,
        secret.encryption_version
    )
    .into_bytes()
}

fn ensure_current_secret(secret: &Secret) -> Result<(), SecretError> {
    if secret.encryption_version != CURRENT_ENCRYPTION_VERSION {
        return Err(SecretError::LegacySecret(secret.name.clone()));
    }
    if secret.cipher != CURRENT_CIPHER || secret.kdf != CURRENT_KDF {
        return Err(SecretError::DecryptionFailed(format!(
            "unsupported secret crypto metadata for {}",
            secret.name
        )));
    }
    Ok(())
}

fn secret_ref_from_secret(secret: &Secret) -> SecretRef {
    SecretRef {
        name: secret.name.clone(),
        provider: secret.provider.clone(),
        encryption_version: secret.encryption_version,
        key_version: secret.key_version,
        created_at: Some(secret.created_at),
        updated_at: Some(secret.updated_at),
        last_used_at: secret.last_used_at,
        usage_count: secret.usage_count,
    }
}

fn rotated_secret_metadata(
    secret: &Secret,
    new_key_version: i32,
    now: chrono::DateTime<Utc>,
) -> Secret {
    let mut rotated = secret.clone();
    rotated.encryption_version = CURRENT_ENCRYPTION_VERSION;
    rotated.key_version = new_key_version;
    rotated.cipher = CURRENT_CIPHER.to_string();
    rotated.kdf = CURRENT_KDF.to_string();
    rotated.aad_version = CURRENT_AAD_VERSION;
    rotated.created_by = Some("master_key_rotation".to_string());
    rotated.rotated_at = Some(now);
    rotated.updated_at = now;
    rotated
}

fn error_for_audit(error: &SecretError) -> String {
    let msg = error.to_string();
    if msg.len() > 240 {
        format!("{}...", &msg[..240])
    } else {
        msg
    }
}

/// PostgreSQL implementation of SecretsStore.
#[cfg(feature = "postgres")]
pub struct PostgresSecretsStore {
    pool: Pool,
    crypto: RwLock<Arc<SecretsCrypto>>,
}

#[cfg(feature = "postgres")]
impl PostgresSecretsStore {
    /// Create a new store with the given database pool and crypto instance.
    pub fn new(pool: Pool, crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            pool,
            crypto: RwLock::new(crypto),
        }
    }

    async fn record_access_audit(
        &self,
        secret: &Secret,
        context: &SecretAccessContext,
        success: bool,
        error_message: Option<&str>,
    ) -> Result<(), SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
        let target_host = context
            .target_host
            .clone()
            .unwrap_or_else(|| context.caller.clone());
        let target_path = context.target_path.clone().or_else(|| {
            Some(format!(
                "{}:{}:{}",
                context.caller,
                context.purpose,
                context.auth_source.as_deref().unwrap_or("unknown")
            ))
        });
        client
            .execute(
                r#"
                INSERT INTO secret_usage_log (id, secret_id, user_id, target_host, target_path, success, error_message)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                "#,
                &[
                    &Uuid::new_v4(),
                    &secret.id,
                    &secret.user_id,
                    &target_host,
                    &target_path,
                    &success,
                    &error_message,
                ],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl SecretsStore for PostgresSecretsStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let key_version = client
            .query_opt(
                "SELECT version FROM secret_key_versions WHERE status = 'active' ORDER BY version DESC LIMIT 1",
                &[],
            )
            .await
            .ok()
            .flatten()
            .map(|row| row.get::<_, i32>(0))
            .unwrap_or(CURRENT_KEY_VERSION);

        let draft = Secret {
            id,
            user_id: user_id.to_string(),
            name: params.name.clone(),
            encrypted_value: Vec::new(),
            key_salt: Vec::new(),
            provider: params.provider.clone(),
            encryption_version: CURRENT_ENCRYPTION_VERSION,
            key_version,
            cipher: CURRENT_CIPHER.to_string(),
            kdf: CURRENT_KDF.to_string(),
            aad_version: CURRENT_AAD_VERSION,
            created_by: params.created_by.clone(),
            rotated_at: None,
            expires_at: params.expires_at,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        };
        let plaintext = params.value.expose_secret().as_bytes();
        let crypto = self.crypto.read().await.clone();
        let (encrypted_value, key_salt) =
            crypto.encrypt_with_aad(plaintext, &secret_aad(&draft))?;

        let row = client
            .query_one(
                r#"
                INSERT INTO secrets (
                    id, user_id, name, encrypted_value, key_salt, provider,
                    encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                    expires_at, created_at, updated_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $15)
                ON CONFLICT (user_id, name) DO UPDATE SET
                    encrypted_value = EXCLUDED.encrypted_value,
                    key_salt = EXCLUDED.key_salt,
                    provider = EXCLUDED.provider,
                    encryption_version = EXCLUDED.encryption_version,
                    key_version = EXCLUDED.key_version,
                    cipher = EXCLUDED.cipher,
                    kdf = EXCLUDED.kdf,
                    aad_version = EXCLUDED.aad_version,
                    created_by = EXCLUDED.created_by,
                    rotated_at = EXCLUDED.rotated_at,
                    expires_at = EXCLUDED.expires_at,
                    updated_at = NOW()
                RETURNING id, user_id, name, encrypted_value, key_salt, provider,
                          encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                          expires_at, last_used_at, usage_count, created_at, updated_at
                "#,
                &[
                    &id,
                    &user_id,
                    &params.name,
                    &encrypted_value,
                    &key_salt,
                    &params.provider,
                    &CURRENT_ENCRYPTION_VERSION,
                    &key_version,
                    &CURRENT_CIPHER,
                    &CURRENT_KDF,
                    &CURRENT_AAD_VERSION,
                    &params.created_by,
                    &Option::<chrono::DateTime<Utc>>::None,
                    &params.expires_at,
                    &now,
                ],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(row_to_secret(&row))
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider,
                       encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                       expires_at, last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE user_id = $1 AND name = $2
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let secret = row_to_secret(&r);

                // Check expiration
                if let Some(expires_at) = secret.expires_at
                    && expires_at < Utc::now()
                {
                    return Err(SecretError::Expired);
                }

                Ok(secret)
            }
            None => Err(SecretError::NotFound(name.to_string())),
        }
    }

    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        ensure_current_secret(&secret)?;
        let crypto = self.crypto.read().await.clone();
        crypto.decrypt_with_aad(
            &secret.encrypted_value,
            &secret.key_salt,
            &secret_aad(&secret),
        )
    }

    async fn get_for_injection(
        &self,
        user_id: &str,
        name: &str,
        context: SecretAccessContext,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        let crypto = self.crypto.read().await.clone();
        let result = (|| {
            ensure_current_secret(&secret)?;
            crypto.decrypt_with_aad(
                &secret.encrypted_value,
                &secret.key_salt,
                &secret_aad(&secret),
            )
        })();
        let success = result.is_ok();
        let error = result.as_ref().err().map(error_for_audit);
        self.record_access_audit(&secret, &context, success, error.as_deref())
            .await?;
        if success {
            self.record_usage(secret.id).await?;
        }
        result
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM secrets WHERE user_id = $1 AND name = $2)",
                &[&user_id, &name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(row.get(0))
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let rows = client
            .query(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider,
                       encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                       expires_at, last_used_at, usage_count, created_at, updated_at
                FROM secrets WHERE user_id = $1 ORDER BY name
                "#,
                &[&user_id],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| secret_ref_from_secret(&row_to_secret(&r)))
            .collect())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let result = client
            .execute(
                "DELETE FROM secrets WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(result > 0)
    }

    async fn rotate_master_key(
        &self,
        new_crypto: Arc<SecretsCrypto>,
    ) -> Result<MasterKeyRotationReport, SecretError> {
        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let old_key_version = match tx
            .query_opt(
                "SELECT version FROM secret_key_versions WHERE status = 'active' ORDER BY version DESC LIMIT 1 FOR UPDATE",
                &[],
            )
            .await
        {
            Ok(Some(row)) => row.get::<_, i32>(0),
            Ok(None) => CURRENT_KEY_VERSION,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "secret_key_versions unavailable; falling back to current key version"
                );
                CURRENT_KEY_VERSION
            }
        };
        let new_key_version = old_key_version + 1;
        let now = Utc::now();
        let old_crypto = self.crypto.read().await.clone();

        let rows = tx
            .query(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider,
                       encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                       expires_at, last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE expires_at IS NULL OR expires_at > NOW()
                ORDER BY user_id, name
                FOR UPDATE
                "#,
                &[],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let mut rotated_count = 0_usize;
        for row in rows {
            let secret = row_to_secret(&row);
            ensure_current_secret(&secret)?;
            if secret.key_version != old_key_version {
                return Err(SecretError::DecryptionFailed(format!(
                    "secret {} is on key_version {}, expected active key_version {}",
                    secret.name, secret.key_version, old_key_version
                )));
            }
            let plaintext = old_crypto.decrypt_with_aad(
                &secret.encrypted_value,
                &secret.key_salt,
                &secret_aad(&secret),
            )?;

            let rotated = rotated_secret_metadata(&secret, new_key_version, now);
            let (encrypted_value, key_salt) = new_crypto
                .encrypt_with_aad(plaintext.expose().as_bytes(), &secret_aad(&rotated))?;
            let verified =
                new_crypto.decrypt_with_aad(&encrypted_value, &key_salt, &secret_aad(&rotated))?;
            if verified.expose() != plaintext.expose() {
                return Err(SecretError::EncryptionFailed(format!(
                    "rotation verification failed for {}",
                    secret.name
                )));
            }

            tx.execute(
                r#"
                UPDATE secrets
                SET encrypted_value = $1,
                    key_salt = $2,
                    encryption_version = $3,
                    key_version = $4,
                    cipher = $5,
                    kdf = $6,
                    aad_version = $7,
                    created_by = $8,
                    rotated_at = $9,
                    updated_at = $9
                WHERE id = $10
                "#,
                &[
                    &encrypted_value,
                    &key_salt,
                    &rotated.encryption_version,
                    &rotated.key_version,
                    &rotated.cipher,
                    &rotated.kdf,
                    &rotated.aad_version,
                    &rotated.created_by,
                    &now,
                    &secret.id,
                ],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
            rotated_count += 1;
        }

        tx.execute(
            r#"
            INSERT INTO secret_key_versions (version, status, created_at)
            VALUES ($1, 'active', $2)
            ON CONFLICT (version) DO UPDATE SET status = 'active', retired_at = NULL
            "#,
            &[&new_key_version, &now],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
        tx.execute(
            "UPDATE secret_key_versions SET status = 'retired', retired_at = $1 WHERE version <> $2 AND status = 'active'",
            &[&now, &new_key_version],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
        *self.crypto.write().await = new_crypto;

        Ok(MasterKeyRotationReport {
            old_key_version,
            new_key_version,
            rotated_secrets: rotated_count,
        })
    }

    async fn record_leak_detection_event(
        &self,
        user_id: &str,
        source: &str,
        action_taken: &str,
        content_hash: &str,
        redacted_preview: Option<&str>,
    ) -> Result<(), SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
        client
            .execute(
                r#"
                INSERT INTO leak_detection_events
                    (id, user_id, source, action_taken, content_hash, redacted_preview)
                VALUES ($1, $2, $3, $4, $5, $6)
                "#,
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &source,
                    &action_taken,
                    &content_hash,
                    &redacted_preview,
                ],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
        Ok(())
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        client
            .execute(
                r#"
                UPDATE secrets
                SET last_used_at = NOW(), usage_count = usage_count + 1
                WHERE id = $1
                "#,
                &[&secret_id],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(())
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        // First check if the secret exists
        if !self.exists(user_id, secret_name).await? {
            return Ok(false);
        }

        // Check if secret is in the allowed list
        // Supports glob patterns: "openai_*" matches "openai_api_key"
        for pattern in allowed_secrets {
            if pattern == secret_name {
                return Ok(true);
            }

            // Simple glob: * matches any suffix
            if let Some(prefix) = pattern.strip_suffix('*')
                && secret_name.starts_with(prefix)
            {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[cfg(feature = "postgres")]
fn row_to_secret(row: &tokio_postgres::Row) -> Secret {
    Secret {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        encrypted_value: row.get("encrypted_value"),
        key_salt: row.get("key_salt"),
        provider: row.get("provider"),
        encryption_version: row.get("encryption_version"),
        key_version: row.get("key_version"),
        cipher: row.get("cipher"),
        kdf: row.get("kdf"),
        aad_version: row.get("aad_version"),
        created_by: row.get("created_by"),
        rotated_at: row.get("rotated_at"),
        expires_at: row.get("expires_at"),
        last_used_at: row.get("last_used_at"),
        usage_count: row.get("usage_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ==================== libSQL implementation ====================

/// libSQL/Turso implementation of SecretsStore.
///
/// Holds an `Arc<Database>` handle and creates a fresh connection per operation,
/// matching the connection-per-request pattern used by the main `LibSqlBackend`.
#[cfg(feature = "libsql")]
pub struct LibSqlSecretsStore {
    db: Arc<libsql::Database>,
    crypto: RwLock<Arc<SecretsCrypto>>,
}

#[cfg(feature = "libsql")]
impl LibSqlSecretsStore {
    /// Create a new store with the given shared libsql database handle and crypto instance.
    pub fn new(db: Arc<libsql::Database>, crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            db,
            crypto: RwLock::new(crypto),
        }
    }

    async fn connect(&self) -> Result<libsql::Connection, SecretError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| SecretError::Database(format!("Connection failed: {}", e)))?;
        let mut rows = conn
            .query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| SecretError::Database(format!("Failed to set busy_timeout: {}", e)))?;
        let _ = rows
            .next()
            .await
            .map_err(|e| SecretError::Database(format!("Failed to confirm busy_timeout: {}", e)))?;
        Ok(conn)
    }

    async fn record_access_audit(
        &self,
        secret: &Secret,
        context: &SecretAccessContext,
        success: bool,
        error_message: Option<&str>,
    ) -> Result<(), SecretError> {
        let conn = self.connect().await?;
        let target_host = context
            .target_host
            .clone()
            .unwrap_or_else(|| context.caller.clone());
        let target_path = context.target_path.clone().or_else(|| {
            Some(format!(
                "{}:{}:{}",
                context.caller,
                context.purpose,
                context.auth_source.as_deref().unwrap_or("unknown")
            ))
        });
        let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        conn.execute(
            r#"
            INSERT INTO secret_usage_log (id, secret_id, user_id, target_host, target_path, success, error_message, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            libsql::params![
                Uuid::new_v4().to_string(),
                secret.id.to_string(),
                secret.user_id.as_str(),
                target_host.as_str(),
                libsql_opt_text(target_path.as_deref()),
                if success { 1_i64 } else { 0_i64 },
                libsql_opt_text(error_message),
                created_at.as_str(),
            ],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
        Ok(())
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl SecretsStore for LibSqlSecretsStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let conn = self.connect().await?;
        let mut version_rows = conn
            .query(
                "SELECT version FROM secret_key_versions WHERE status = 'active' ORDER BY version DESC LIMIT 1",
                (),
            )
            .await
            .ok();
        let key_version = if let Some(rows) = version_rows.as_mut() {
            rows.next()
                .await
                .ok()
                .flatten()
                .and_then(|row| row.get::<i64>(0).ok())
                .map(|version| version as i32)
                .unwrap_or(CURRENT_KEY_VERSION)
        } else {
            CURRENT_KEY_VERSION
        };
        drop(version_rows);
        let draft = Secret {
            id,
            user_id: user_id.to_string(),
            name: params.name.clone(),
            encrypted_value: Vec::new(),
            key_salt: Vec::new(),
            provider: params.provider.clone(),
            encryption_version: CURRENT_ENCRYPTION_VERSION,
            key_version,
            cipher: CURRENT_CIPHER.to_string(),
            kdf: CURRENT_KDF.to_string(),
            aad_version: CURRENT_AAD_VERSION,
            created_by: params.created_by.clone(),
            rotated_at: None,
            expires_at: params.expires_at,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        };
        let plaintext = params.value.expose_secret().as_bytes();
        let crypto = self.crypto.read().await.clone();
        let (encrypted_value, key_salt) =
            crypto.encrypt_with_aad(plaintext, &secret_aad(&draft))?;
        let expires_at_str = params
            .expires_at
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

        // Start transaction for atomic upsert + read-back
        let tx = conn
            .transaction()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        tx.execute(
                r#"
                INSERT INTO secrets (
                    id, user_id, name, encrypted_value, key_salt, provider,
                    encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                    expires_at, created_at, updated_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)
                ON CONFLICT (user_id, name) DO UPDATE SET
                    encrypted_value = excluded.encrypted_value,
                    key_salt = excluded.key_salt,
                    provider = excluded.provider,
                    encryption_version = excluded.encryption_version,
                    key_version = excluded.key_version,
                    cipher = excluded.cipher,
                    kdf = excluded.kdf,
                    aad_version = excluded.aad_version,
                    created_by = excluded.created_by,
                    rotated_at = excluded.rotated_at,
                    expires_at = excluded.expires_at,
                    updated_at = ?15
                "#,
                libsql::params![
                    id.to_string(),
                    user_id,
                    params.name.as_str(),
                    libsql::Value::Blob(encrypted_value.clone()),
                    libsql::Value::Blob(key_salt.clone()),
                    libsql_opt_text(params.provider.as_deref()),
                    CURRENT_ENCRYPTION_VERSION as i64,
                    key_version as i64,
                    CURRENT_CIPHER,
                    CURRENT_KDF,
                    CURRENT_AAD_VERSION as i64,
                    libsql_opt_text(params.created_by.as_deref()),
                    libsql::Value::Null,
                    libsql_opt_text(expires_at_str.as_deref()),
                    now_str.as_str(),
                ],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        // Read back the row (may have been upserted)
        let mut rows = tx
            .query(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider,
                       encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                       expires_at, last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE user_id = ?1 AND name = ?2
                "#,
                libsql::params![user_id, params.name.as_str()],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
            .ok_or_else(|| SecretError::Database("Insert succeeded but row not found".into()))?;

        let secret = libsql_row_to_secret(&row)?;

        tx.commit()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(secret)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider,
                       encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                       expires_at, last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE user_id = ?1 AND name = ?2
                "#,
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
        {
            Some(row) => {
                let secret = libsql_row_to_secret(&row)?;

                if let Some(expires_at) = secret.expires_at
                    && expires_at < Utc::now()
                {
                    return Err(SecretError::Expired);
                }

                Ok(secret)
            }
            None => Err(SecretError::NotFound(name.to_string())),
        }
    }

    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        ensure_current_secret(&secret)?;
        let crypto = self.crypto.read().await.clone();
        crypto.decrypt_with_aad(
            &secret.encrypted_value,
            &secret.key_salt,
            &secret_aad(&secret),
        )
    }

    async fn get_for_injection(
        &self,
        user_id: &str,
        name: &str,
        context: SecretAccessContext,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        let crypto = self.crypto.read().await.clone();
        let result = (|| {
            ensure_current_secret(&secret)?;
            crypto.decrypt_with_aad(
                &secret.encrypted_value,
                &secret.key_salt,
                &secret_aad(&secret),
            )
        })();
        let success = result.is_ok();
        let error = result.as_ref().err().map(error_for_audit);
        self.record_access_audit(&secret, &context, success, error.as_deref())
            .await?;
        if success {
            self.record_usage(secret.id).await?;
        }
        result
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM secrets WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
            .is_some())
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider,
                       encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                       expires_at, last_used_at, usage_count, created_at, updated_at
                FROM secrets WHERE user_id = ?1 ORDER BY name
                "#,
                libsql::params![user_id],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let mut refs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
        {
            refs.push(secret_ref_from_secret(&libsql_row_to_secret(&row)?));
        }
        Ok(refs)
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                "DELETE FROM secrets WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(affected > 0)
    }

    async fn rotate_master_key(
        &self,
        new_crypto: Arc<SecretsCrypto>,
    ) -> Result<MasterKeyRotationReport, SecretError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE TRANSACTION", ())
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let result = async {
            let mut version_rows = conn
                .query(
                    "SELECT version FROM secret_key_versions WHERE status = 'active' ORDER BY version DESC LIMIT 1",
                    (),
                )
                .await
                .map_err(|e| SecretError::Database(e.to_string()))?;
            let old_key_version = match version_rows
                .next()
                .await
                .map_err(|e| SecretError::Database(e.to_string()))?
            {
                Some(row) => row.get::<i64>(0).unwrap_or(CURRENT_KEY_VERSION as i64) as i32,
                None => CURRENT_KEY_VERSION,
            };
            drop(version_rows);

            let new_key_version = old_key_version + 1;
            let now = Utc::now();
            let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let old_crypto = self.crypto.read().await.clone();

            let mut rows = conn
                .query(
                    r#"
                    SELECT id, user_id, name, encrypted_value, key_salt, provider,
                           encryption_version, key_version, cipher, kdf, aad_version, created_by, rotated_at,
                           expires_at, last_used_at, usage_count, created_at, updated_at
                    FROM secrets
                    WHERE expires_at IS NULL OR expires_at > datetime('now')
                    ORDER BY user_id, name
                    "#,
                    (),
                )
                .await
                .map_err(|e| SecretError::Database(e.to_string()))?;

            let mut updates: Vec<(Secret, Vec<u8>, Vec<u8>)> = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| SecretError::Database(e.to_string()))?
            {
                let secret = libsql_row_to_secret(&row)?;
                ensure_current_secret(&secret)?;
                if secret.key_version != old_key_version {
                    return Err(SecretError::DecryptionFailed(format!(
                        "secret {} is on key_version {}, expected active key_version {}",
                        secret.name, secret.key_version, old_key_version
                    )));
                }
                let plaintext = old_crypto.decrypt_with_aad(
                    &secret.encrypted_value,
                    &secret.key_salt,
                    &secret_aad(&secret),
                )?;
                let rotated = rotated_secret_metadata(&secret, new_key_version, now);
                let (encrypted_value, key_salt) = new_crypto.encrypt_with_aad(
                    plaintext.expose().as_bytes(),
                    &secret_aad(&rotated),
                )?;
                let verified = new_crypto.decrypt_with_aad(
                    &encrypted_value,
                    &key_salt,
                    &secret_aad(&rotated),
                )?;
                if verified.expose() != plaintext.expose() {
                    return Err(SecretError::EncryptionFailed(format!(
                        "rotation verification failed for {}",
                        secret.name
                    )));
                }
                updates.push((rotated, encrypted_value, key_salt));
            }
            drop(rows);

            for (rotated, encrypted_value, key_salt) in &updates {
                conn.execute(
                    r#"
                    UPDATE secrets
                    SET encrypted_value = ?1,
                        key_salt = ?2,
                        encryption_version = ?3,
                        key_version = ?4,
                        cipher = ?5,
                        kdf = ?6,
                        aad_version = ?7,
                        created_by = ?8,
                        rotated_at = ?9,
                        updated_at = ?9
                    WHERE id = ?10
                    "#,
                    libsql::params![
                        libsql::Value::Blob(encrypted_value.clone()),
                        libsql::Value::Blob(key_salt.clone()),
                        rotated.encryption_version as i64,
                        rotated.key_version as i64,
                        rotated.cipher.as_str(),
                        rotated.kdf.as_str(),
                        rotated.aad_version as i64,
                        libsql_opt_text(rotated.created_by.as_deref()),
                        now_str.as_str(),
                        rotated.id.to_string(),
                    ],
                )
                .await
                .map_err(|e| SecretError::Database(e.to_string()))?;
            }

            conn.execute(
                r#"
                INSERT INTO secret_key_versions (version, status, created_at)
                VALUES (?1, 'active', ?2)
                ON CONFLICT(version) DO UPDATE SET status = 'active', retired_at = NULL
                "#,
                libsql::params![new_key_version as i64, now_str.as_str()],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
            conn.execute(
                "UPDATE secret_key_versions SET status = 'retired', retired_at = ?1 WHERE version <> ?2 AND status = 'active'",
                libsql::params![now_str.as_str(), new_key_version as i64],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

            Ok::<_, SecretError>(MasterKeyRotationReport {
                old_key_version,
                new_key_version,
                rotated_secrets: updates.len(),
            })
        }
        .await;

        match result {
            Ok(report) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| SecretError::Database(e.to_string()))?;
                *self.crypto.write().await = new_crypto;
                Ok(report)
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(error)
            }
        }
    }

    async fn record_leak_detection_event(
        &self,
        user_id: &str,
        source: &str,
        action_taken: &str,
        content_hash: &str,
        redacted_preview: Option<&str>,
    ) -> Result<(), SecretError> {
        let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO leak_detection_events
                (id, user_id, source, action_taken, content_hash, redacted_preview, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            libsql::params![
                Uuid::new_v4().to_string(),
                user_id,
                source,
                action_taken,
                content_hash,
                libsql_opt_text(redacted_preview),
                created_at,
            ],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
        Ok(())
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let conn = self.connect().await?;

        conn.execute(
            r#"
                UPDATE secrets
                SET last_used_at = ?1, usage_count = usage_count + 1
                WHERE id = ?2
                "#,
            libsql::params![now.as_str(), secret_id.to_string()],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(())
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        if !self.exists(user_id, secret_name).await? {
            return Ok(false);
        }

        for pattern in allowed_secrets {
            if pattern == secret_name {
                return Ok(true);
            }

            if let Some(prefix) = pattern.strip_suffix('*')
                && secret_name.starts_with(prefix)
            {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[cfg(feature = "libsql")]
fn libsql_opt_text(s: Option<&str>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s.to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn libsql_parse_timestamp(s: &str) -> Result<chrono::DateTime<Utc>, SecretError> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(SecretError::Database(format!(
        "unparseable timestamp: {:?}",
        s
    )))
}

#[cfg(feature = "libsql")]
fn libsql_row_to_secret(row: &libsql::Row) -> Result<Secret, SecretError> {
    let id_str: String = row
        .get(0)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let user_id: String = row
        .get(1)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let name: String = row
        .get(2)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let encrypted_value: Vec<u8> = row
        .get(3)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let key_salt: Vec<u8> = row
        .get(4)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let provider: Option<String> = row.get::<String>(5).ok().filter(|s| !s.is_empty());
    let encryption_version: i64 = row.get::<i64>(6).unwrap_or(1);
    let key_version: i64 = row.get::<i64>(7).unwrap_or(1);
    let cipher: String = row
        .get::<String>(8)
        .unwrap_or_else(|_| CURRENT_CIPHER.to_string());
    let kdf: String = row
        .get::<String>(9)
        .unwrap_or_else(|_| CURRENT_KDF.to_string());
    let aad_version: i64 = row.get::<i64>(10).unwrap_or(0);
    let created_by: Option<String> = row.get::<String>(11).ok().filter(|s| !s.is_empty());
    let rotated_at = row
        .get::<String>(12)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| libsql_parse_timestamp(&s).ok());
    let expires_at = row
        .get::<String>(13)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| libsql_parse_timestamp(&s).ok());
    let last_used_at = row
        .get::<String>(14)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| libsql_parse_timestamp(&s).ok());
    let usage_count: i64 = row.get::<i64>(15).unwrap_or(0);
    let created_at_str: String = row
        .get(16)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let updated_at_str: String = row
        .get(17)
        .map_err(|e| SecretError::Database(e.to_string()))?;

    Ok(Secret {
        id: id_str
            .parse()
            .map_err(|e: uuid::Error| SecretError::Database(e.to_string()))?,
        user_id,
        name,
        encrypted_value,
        key_salt,
        provider,
        encryption_version: encryption_version as i32,
        key_version: key_version as i32,
        cipher,
        kdf,
        aad_version: aad_version as i32,
        created_by,
        rotated_at,
        expires_at,
        last_used_at,
        usage_count,
        created_at: libsql_parse_timestamp(&created_at_str)?,
        updated_at: libsql_parse_timestamp(&updated_at_str)?,
    })
}

/// In-memory secrets store. Used for testing and as a fallback when no
/// persistent secrets backend is configured (extension listing/install still
/// works, but stored secrets won't survive a restart).
pub mod in_memory {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;
    use secrecy::ExposeSecret;
    use tokio::sync::RwLock;
    use uuid::Uuid;

    use crate::secrets::crypto::SecretsCrypto;
    use crate::secrets::store::SecretsStore;
    use crate::secrets::types::{
        CURRENT_AAD_VERSION, CURRENT_CIPHER, CURRENT_ENCRYPTION_VERSION, CURRENT_KDF,
        CURRENT_KEY_VERSION, CreateSecretParams, DecryptedSecret, MasterKeyRotationReport, Secret,
        SecretAccessContext, SecretError, SecretRef,
    };

    pub struct InMemorySecretsStore {
        secrets: RwLock<HashMap<(String, String), Secret>>,
        crypto: RwLock<Arc<SecretsCrypto>>,
        key_version: RwLock<i32>,
    }

    impl InMemorySecretsStore {
        pub fn new(crypto: Arc<SecretsCrypto>) -> Self {
            Self {
                secrets: RwLock::new(HashMap::new()),
                crypto: RwLock::new(crypto),
                key_version: RwLock::new(CURRENT_KEY_VERSION),
            }
        }
    }

    #[async_trait]
    impl SecretsStore for InMemorySecretsStore {
        async fn create(
            &self,
            user_id: &str,
            params: CreateSecretParams,
        ) -> Result<Secret, SecretError> {
            let now = Utc::now();
            let mut secret = Secret {
                id: Uuid::new_v4(),
                user_id: user_id.to_string(),
                name: params.name.clone(),
                encrypted_value: Vec::new(),
                key_salt: Vec::new(),
                provider: params.provider.clone(),
                encryption_version: CURRENT_ENCRYPTION_VERSION,
                key_version: *self.key_version.read().await,
                cipher: CURRENT_CIPHER.to_string(),
                kdf: CURRENT_KDF.to_string(),
                aad_version: CURRENT_AAD_VERSION,
                created_by: params.created_by,
                rotated_at: None,
                expires_at: params.expires_at,
                last_used_at: None,
                usage_count: 0,
                created_at: now,
                updated_at: now,
            };
            let plaintext = params.value.expose_secret().as_bytes();
            let crypto = self.crypto.read().await.clone();
            let (encrypted_value, key_salt) =
                crypto.encrypt_with_aad(plaintext, &super::secret_aad(&secret))?;
            secret.encrypted_value = encrypted_value;
            secret.key_salt = key_salt;

            self.secrets
                .write()
                .await
                .insert((user_id.to_string(), params.name), secret.clone());
            Ok(secret)
        }

        async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
            let secret = self
                .secrets
                .read()
                .await
                .get(&(user_id.to_string(), name.to_string()))
                .cloned()
                .ok_or_else(|| SecretError::NotFound(name.to_string()))?;

            if let Some(expires_at) = secret.expires_at
                && expires_at < Utc::now()
            {
                return Err(SecretError::Expired);
            }

            Ok(secret)
        }

        async fn get_decrypted(
            &self,
            user_id: &str,
            name: &str,
        ) -> Result<DecryptedSecret, SecretError> {
            let secret = self.get(user_id, name).await?;
            super::ensure_current_secret(&secret)?;
            let crypto = self.crypto.read().await.clone();
            crypto.decrypt_with_aad(
                &secret.encrypted_value,
                &secret.key_salt,
                &super::secret_aad(&secret),
            )
        }

        async fn get_for_injection(
            &self,
            user_id: &str,
            name: &str,
            _context: SecretAccessContext,
        ) -> Result<DecryptedSecret, SecretError> {
            let secret = self.get(user_id, name).await?;
            super::ensure_current_secret(&secret)?;
            let crypto = self.crypto.read().await.clone();
            let decrypted = crypto.decrypt_with_aad(
                &secret.encrypted_value,
                &secret.key_salt,
                &super::secret_aad(&secret),
            )?;
            self.record_usage(secret.id).await?;
            Ok(decrypted)
        }

        async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
            Ok(self
                .secrets
                .read()
                .await
                .contains_key(&(user_id.to_string(), name.to_string())))
        }

        async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
            Ok(self
                .secrets
                .read()
                .await
                .iter()
                .filter(|((uid, _), _)| uid == user_id)
                .map(|((_, _), s)| super::secret_ref_from_secret(s))
                .collect())
        }

        async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
            Ok(self
                .secrets
                .write()
                .await
                .remove(&(user_id.to_string(), name.to_string()))
                .is_some())
        }

        async fn rotate_master_key(
            &self,
            new_crypto: Arc<SecretsCrypto>,
        ) -> Result<MasterKeyRotationReport, SecretError> {
            let mut key_version = self.key_version.write().await;
            let old_key_version = *key_version;
            let new_key_version = old_key_version + 1;
            let now = Utc::now();
            let mut guard = self.secrets.write().await;
            let old_crypto = self.crypto.read().await.clone();

            let mut updates = Vec::new();
            for ((user_id, name), secret) in guard.iter() {
                if let Some(expires_at) = secret.expires_at
                    && expires_at < now
                {
                    continue;
                }
                super::ensure_current_secret(secret)?;
                if secret.key_version != old_key_version {
                    return Err(SecretError::DecryptionFailed(format!(
                        "secret {} is on key_version {}, expected active key_version {}",
                        secret.name, secret.key_version, old_key_version
                    )));
                }
                let plaintext = old_crypto.decrypt_with_aad(
                    &secret.encrypted_value,
                    &secret.key_salt,
                    &super::secret_aad(secret),
                )?;
                let mut rotated = super::rotated_secret_metadata(secret, new_key_version, now);
                let (encrypted_value, key_salt) = new_crypto.encrypt_with_aad(
                    plaintext.expose().as_bytes(),
                    &super::secret_aad(&rotated),
                )?;
                let verified = new_crypto.decrypt_with_aad(
                    &encrypted_value,
                    &key_salt,
                    &super::secret_aad(&rotated),
                )?;
                if verified.expose() != plaintext.expose() {
                    return Err(SecretError::EncryptionFailed(format!(
                        "rotation verification failed for {}",
                        secret.name
                    )));
                }
                rotated.encrypted_value = encrypted_value;
                rotated.key_salt = key_salt;
                updates.push(((user_id.clone(), name.clone()), rotated));
            }

            let rotated_secrets = updates.len();
            for (key, secret) in updates {
                guard.insert(key, secret);
            }
            *key_version = new_key_version;
            *self.crypto.write().await = new_crypto;

            Ok(MasterKeyRotationReport {
                old_key_version,
                new_key_version,
                rotated_secrets,
            })
        }

        async fn record_usage(&self, _secret_id: Uuid) -> Result<(), SecretError> {
            Ok(())
        }

        async fn is_accessible(
            &self,
            user_id: &str,
            secret_name: &str,
            allowed_secrets: &[String],
        ) -> Result<bool, SecretError> {
            if !self.exists(user_id, secret_name).await? {
                return Ok(false);
            }
            for pattern in allowed_secrets {
                if pattern == secret_name {
                    return Ok(true);
                }
                if let Some(prefix) = pattern.strip_suffix('*')
                    && secret_name.starts_with(prefix)
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use secrecy::SecretString;

    use crate::secrets::crypto::SecretsCrypto;
    use crate::secrets::store::SecretsStore;
    use crate::secrets::store::in_memory::InMemorySecretsStore;
    use crate::secrets::types::CreateSecretParams;

    fn test_store() -> InMemorySecretsStore {
        let key = "0123456789abcdef0123456789abcdef";
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key.to_string())).unwrap());
        InMemorySecretsStore::new(crypto)
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let store = test_store();
        let params = CreateSecretParams::new("api_key", "sk-test-12345");

        store.create("user1", params).await.unwrap();

        let decrypted = store.get_decrypted("user1", "api_key").await.unwrap();
        assert_eq!(decrypted.expose(), "sk-test-12345");
    }

    #[tokio::test]
    async fn test_exists() {
        let store = test_store();
        let params = CreateSecretParams::new("my_secret", "value");

        assert!(!store.exists("user1", "my_secret").await.unwrap());
        store.create("user1", params).await.unwrap();
        assert!(store.exists("user1", "my_secret").await.unwrap());
    }

    #[tokio::test]
    async fn test_delete() {
        let store = test_store();
        let params = CreateSecretParams::new("to_delete", "value");

        store.create("user1", params).await.unwrap();
        assert!(store.exists("user1", "to_delete").await.unwrap());

        store.delete("user1", "to_delete").await.unwrap();
        assert!(!store.exists("user1", "to_delete").await.unwrap());
    }

    #[tokio::test]
    async fn test_list() {
        let store = test_store();

        store
            .create("user1", CreateSecretParams::new("key1", "v1"))
            .await
            .unwrap();
        store
            .create(
                "user1",
                CreateSecretParams::new("key2", "v2").with_provider("openai"),
            )
            .await
            .unwrap();
        store
            .create("user2", CreateSecretParams::new("key3", "v3"))
            .await
            .unwrap();

        let list = store.list("user1").await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_is_accessible() {
        let store = test_store();
        store
            .create("user1", CreateSecretParams::new("openai_key", "sk-test"))
            .await
            .unwrap();
        store
            .create("user1", CreateSecretParams::new("stripe_key", "sk-live"))
            .await
            .unwrap();

        // Exact match
        let allowed = vec!["openai_key".to_string()];
        assert!(
            store
                .is_accessible("user1", "openai_key", &allowed)
                .await
                .unwrap()
        );
        assert!(
            !store
                .is_accessible("user1", "stripe_key", &allowed)
                .await
                .unwrap()
        );

        // Glob pattern
        let allowed = vec!["openai_*".to_string()];
        assert!(
            store
                .is_accessible("user1", "openai_key", &allowed)
                .await
                .unwrap()
        );
        assert!(
            !store
                .is_accessible("user1", "stripe_key", &allowed)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_expired_secret_returns_error() {
        let store = test_store();
        let expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        let params = CreateSecretParams::new("expired_key", "value").with_expiry(expires_at);

        store.create("user1", params).await.unwrap();

        let result = store.get("user1", "expired_key").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::secrets::SecretError::Expired
        ));
    }

    #[tokio::test]
    async fn test_non_expired_secret_succeeds() {
        let store = test_store();
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        let params = CreateSecretParams::new("fresh_key", "value").with_expiry(expires_at);

        store.create("user1", params).await.unwrap();

        let result = store.get("user1", "fresh_key").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_user_isolation() {
        let store = test_store();

        store
            .create(
                "user1",
                CreateSecretParams::new("shared_name", "user1_value"),
            )
            .await
            .unwrap();
        store
            .create(
                "user2",
                CreateSecretParams::new("shared_name", "user2_value"),
            )
            .await
            .unwrap();

        let v1 = store.get_decrypted("user1", "shared_name").await.unwrap();
        let v2 = store.get_decrypted("user2", "shared_name").await.unwrap();

        assert_eq!(v1.expose(), "user1_value");
        assert_eq!(v2.expose(), "user2_value");
    }

    #[tokio::test]
    async fn test_master_rotation_preserves_values_and_advances_key_version() {
        let store = test_store();
        store
            .create(
                "user1",
                CreateSecretParams::new("api_key", "value-before-rotation"),
            )
            .await
            .unwrap();

        let new_crypto = Arc::new(
            SecretsCrypto::new(SecretString::from(
                "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".to_string(),
            ))
            .unwrap(),
        );
        let report = store.rotate_master_key(new_crypto).await.unwrap();

        assert_eq!(report.old_key_version, 1);
        assert_eq!(report.new_key_version, 2);
        assert_eq!(report.rotated_secrets, 1);

        let metadata = store.get("user1", "api_key").await.unwrap();
        assert_eq!(metadata.key_version, 2);
        assert!(metadata.rotated_at.is_some());

        let decrypted = store.get_decrypted("user1", "api_key").await.unwrap();
        assert_eq!(decrypted.expose(), "value-before-rotation");
    }
}
