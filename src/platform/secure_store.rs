//! Shared secure-store wrapper around platform secret backends.

use crate::secrets::{SecretError, keychain};

pub const CLAUDE_CODE_API_KEY_ACCOUNT: &str = keychain::CLAUDE_CODE_API_KEY_ACCOUNT;
pub const CODEX_CODE_API_KEY_ACCOUNT: &str = keychain::CODEX_CODE_API_KEY_ACCOUNT;

pub fn display_name() -> &'static str {
    "OS secure store"
}

pub fn generate_master_key() -> Vec<u8> {
    keychain::generate_master_key()
}

pub fn generate_master_key_hex() -> String {
    keychain::generate_master_key_hex()
}

pub async fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
    keychain::store_master_key(key).await
}

pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
    keychain::get_master_key().await
}

pub async fn delete_master_key() -> Result<(), SecretError> {
    keychain::delete_master_key().await
}

pub async fn has_master_key() -> bool {
    keychain::has_master_key().await
}

pub async fn store_api_key(account: &str, value: &str) -> Result<(), SecretError> {
    keychain::store_api_key(account, value).await
}

pub async fn get_api_key(account: &str) -> Option<String> {
    keychain::get_api_key(account).await
}

pub async fn delete_api_key(account: &str) -> Result<(), SecretError> {
    keychain::delete_api_key(account).await
}
