//! Secrets management for secure credential storage and injection.
//!
//! This crate owns the secrets domain implementation. The root package
//! re-exports this API from its compatibility facade.

mod crypto;
pub mod keychain;
mod store;
mod types;

pub use crypto::SecretsCrypto;
#[cfg(feature = "libsql")]
pub use store::LibSqlSecretsStore;
#[cfg(feature = "postgres")]
pub use store::PostgresSecretsStore;
pub use store::SecretsStore;
pub use store::in_memory::InMemorySecretsStore;
pub use types::{
    CreateSecretParams, CredentialLocation, CredentialMapping, DecryptedSecret,
    MasterKeyRotationReport, Secret, SecretAccessContext, SecretBackend, SecretError, SecretRef,
};
