//! Secret types for credential management.
//!
//! WASM tools NEVER see plaintext secrets. This module provides types
//! for secure storage and reference without exposing actual values.

use std::fmt;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const CURRENT_ENCRYPTION_VERSION: i32 = 2;
pub const CURRENT_KEY_VERSION: i32 = 1;
pub const CURRENT_CIPHER: &str = "aes-256-gcm";
pub const CURRENT_KDF: &str = "hkdf-sha256";
pub const CURRENT_AAD_VERSION: i32 = 1;

/// A stored secret with encrypted value.
///
/// The plaintext is never stored; only the encrypted form exists in the database.
#[derive(Clone)]
pub struct Secret {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    /// AES-256-GCM encrypted value (nonce || ciphertext || tag).
    pub encrypted_value: Vec<u8>,
    /// Per-secret salt for key derivation.
    pub key_salt: Vec<u8>,
    /// Optional provider hint (e.g., "openai", "stripe").
    pub provider: Option<String>,
    /// Encryption metadata version. Version 1 rows are legacy and intentionally
    /// rejected by the v2 decrypt path.
    pub encryption_version: i32,
    /// Version of the master key used to encrypt this row.
    pub key_version: i32,
    /// Cipher identifier for operator diagnostics and future migrations.
    pub cipher: String,
    /// KDF identifier for operator diagnostics and future migrations.
    pub kdf: String,
    /// AAD format version used when encrypting this row.
    pub aad_version: i32,
    /// Human/system actor that created the current ciphertext.
    pub created_by: Option<String>,
    /// Last time this row was re-encrypted by master-key rotation.
    pub rotated_at: Option<DateTime<Utc>>,
    /// When this secret expires (None = never).
    pub expires_at: Option<DateTime<Utc>>,
    /// Last time this secret was used for injection.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Total number of times this secret has been used.
    pub usage_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("name", &self.name)
            .field("encrypted_value", &"[REDACTED]")
            .field("key_salt", &"[REDACTED]")
            .field("provider", &self.provider)
            .field("encryption_version", &self.encryption_version)
            .field("key_version", &self.key_version)
            .field("cipher", &self.cipher)
            .field("kdf", &self.kdf)
            .field("aad_version", &self.aad_version)
            .field("created_by", &self.created_by)
            .field("rotated_at", &self.rotated_at)
            .field("expires_at", &self.expires_at)
            .field("last_used_at", &self.last_used_at)
            .field("usage_count", &self.usage_count)
            .finish()
    }
}

/// A reference to a secret by name, without exposing the value.
///
/// WASM tools receive these references and can check if secrets exist,
/// but they cannot read the actual values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub provider: Option<String>,
    #[serde(default)]
    pub encryption_version: i32,
    #[serde(default)]
    pub key_version: i32,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub usage_count: i64,
}

/// Result of a local encrypted master-key rotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterKeyRotationReport {
    pub old_key_version: i32,
    pub new_key_version: i32,
    pub rotated_secrets: usize,
}

impl SecretRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider: None,
            encryption_version: CURRENT_ENCRYPTION_VERSION,
            key_version: CURRENT_KEY_VERSION,
            created_at: None,
            updated_at: None,
            last_used_at: None,
            usage_count: 0,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }
}

/// A decrypted secret value, held in secure memory.
///
/// This type:
/// - Zeros memory on drop
/// - Never appears in Debug output
/// - Only exists briefly during credential injection
pub struct DecryptedSecret {
    value: SecretString,
}

impl DecryptedSecret {
    /// Create a new decrypted secret from raw bytes.
    ///
    /// The bytes are converted to a UTF-8 string. For binary secrets,
    /// consider base64 encoding before storage.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecretError> {
        // Convert to string, then wrap in SecretString
        let s = String::from_utf8(bytes).map_err(|_| SecretError::InvalidUtf8)?;
        Ok(Self {
            value: SecretString::from(s),
        })
    }

    /// Expose the secret value for injection.
    ///
    /// This is the ONLY way to access the plaintext. Use sparingly
    /// and ensure the exposed value isn't logged or persisted.
    pub fn expose(&self) -> &str {
        self.value.expose_secret()
    }

    /// Get the length of the secret without exposing it.
    pub fn len(&self) -> usize {
        self.value.expose_secret().len()
    }

    /// Check if the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for DecryptedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DecryptedSecret([REDACTED, {} bytes])", self.len())
    }
}

impl Clone for DecryptedSecret {
    fn clone(&self) -> Self {
        Self {
            value: SecretString::from(self.value.expose_secret().to_string()),
        }
    }
}

/// Errors that can occur during secret operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum SecretError {
    #[error("Secret not found: {0}")]
    NotFound(String),

    #[error("Secret has expired")]
    Expired,

    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),

    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),

    #[error("Invalid master key")]
    InvalidMasterKey,

    #[error("Secret value is not valid UTF-8")]
    InvalidUtf8,

    #[error("Database error: {0}")]
    Database(String),

    #[error("Secret access denied for tool")]
    AccessDenied,

    #[error("Keychain error: {0}")]
    KeychainError(String),

    #[error("Legacy secret requires re-entry: {0}")]
    LegacySecret(String),
}

/// Parameters for creating a new secret.
#[derive(Debug)]
pub struct CreateSecretParams {
    pub name: String,
    pub value: SecretString,
    pub provider: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_by: Option<String>,
}

impl CreateSecretParams {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: SecretString::from(value.into()),
            provider: None,
            expires_at: None,
            created_by: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_expiry(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    pub fn with_created_by(mut self, created_by: impl Into<String>) -> Self {
        self.created_by = Some(created_by.into());
        self
    }
}

/// Context required when plaintext is exposed for a runtime operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretAccessContext {
    /// Component requesting access, e.g. "provider_vault", "llm.discovery", "wasm.http".
    pub caller: String,
    /// Why plaintext is needed.
    pub purpose: String,
    /// Optional target host receiving the credential.
    pub target_host: Option<String>,
    /// Optional target path receiving the credential.
    pub target_path: Option<String>,
    /// Authentication source authorizing this request.
    pub auth_source: Option<String>,
}

impl SecretAccessContext {
    pub fn new(caller: impl Into<String>, purpose: impl Into<String>) -> Self {
        Self {
            caller: caller.into(),
            purpose: purpose.into(),
            target_host: None,
            target_path: None,
            auth_source: None,
        }
    }

    pub fn target(mut self, host: impl Into<String>, path: impl Into<String>) -> Self {
        self.target_host = Some(host.into());
        self.target_path = Some(path.into());
        self
    }

    pub fn auth_source(mut self, auth_source: impl Into<String>) -> Self {
        self.auth_source = Some(auth_source.into());
        self
    }
}

/// Future-facing backend boundary. The v1 implementation is local encrypted
/// storage; external backends should preserve this logical interface.
#[async_trait::async_trait]
pub trait SecretBackend: Send + Sync {
    async fn health(&self) -> Result<String, SecretError>;
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<SecretRef, SecretError>;
    async fn get_for_injection(
        &self,
        user_id: &str,
        name: &str,
        context: SecretAccessContext,
    ) -> Result<DecryptedSecret, SecretError>;
    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError>;
    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError>;
    async fn rotate(&self) -> Result<String, SecretError>;
}

/// Where a credential should be injected in an HTTP request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum CredentialLocation {
    /// Inject as Authorization header (e.g., "Bearer {secret}")
    #[default]
    AuthorizationBearer,
    /// Inject as Authorization header with Basic auth
    AuthorizationBasic { username: String },
    /// Inject as a custom header
    Header {
        name: String,
        prefix: Option<String>,
    },
    /// Inject as a query parameter
    QueryParam { name: String },
    /// Inject by replacing a placeholder in URL or body templates
    UrlPath { placeholder: String },
}

/// Mapping from a secret name to where it should be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMapping {
    /// Name of the secret to use.
    pub secret_name: String,
    /// Where to inject the credential.
    pub location: CredentialLocation,
    /// Host patterns this credential applies to (glob syntax).
    pub host_patterns: Vec<String>,
}

impl CredentialMapping {
    pub fn bearer(secret_name: impl Into<String>, host_pattern: impl Into<String>) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec![host_pattern.into()],
        }
    }

    pub fn header(
        secret_name: impl Into<String>,
        header_name: impl Into<String>,
        host_pattern: impl Into<String>,
    ) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::Header {
                name: header_name.into(),
                prefix: None,
            },
            host_patterns: vec![host_pattern.into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::types::{CreateSecretParams, DecryptedSecret, SecretRef};

    #[test]
    fn test_secret_ref_creation() {
        let r = SecretRef::new("my_api_key").with_provider("openai");
        assert_eq!(r.name, "my_api_key");
        assert_eq!(r.provider, Some("openai".to_string()));
        assert_eq!(r.encryption_version, super::CURRENT_ENCRYPTION_VERSION);
    }

    #[test]
    fn test_decrypted_secret_redaction() {
        let secret = DecryptedSecret::from_bytes(b"super_secret_value".to_vec()).unwrap();
        let debug_str = format!("{:?}", secret);
        assert!(!debug_str.contains("super_secret_value"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_decrypted_secret_expose() {
        let secret = DecryptedSecret::from_bytes(b"test_value".to_vec()).unwrap();
        assert_eq!(secret.expose(), "test_value");
        assert_eq!(secret.len(), 10);
    }

    #[test]
    fn test_create_params() {
        let params = CreateSecretParams::new("key", "value").with_provider("stripe");
        assert_eq!(params.name, "key");
        assert_eq!(params.provider, Some("stripe".to_string()));
    }
}
