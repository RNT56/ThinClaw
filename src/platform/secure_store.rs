//! Shared secure-store wrapper around platform secret backends.

use crate::secrets::{SecretError, keychain};

pub const CLAUDE_CODE_API_KEY_ACCOUNT: &str = keychain::CLAUDE_CODE_API_KEY_ACCOUNT;
pub const CODEX_CODE_API_KEY_ACCOUNT: &str = keychain::CODEX_CODE_API_KEY_ACCOUNT;

#[cfg(target_os = "macos")]
fn cached_master_key() -> &'static std::sync::RwLock<Option<Vec<u8>>> {
    use std::sync::{OnceLock, RwLock};

    static CACHE: OnceLock<RwLock<Option<Vec<u8>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(None))
}

#[cfg(target_os = "macos")]
fn read_cached_master_key() -> Option<Vec<u8>> {
    cached_master_key()
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
}

#[cfg(target_os = "macos")]
fn write_cached_master_key(key: &[u8]) {
    if let Ok(mut guard) = cached_master_key().write() {
        if let Some(existing) = guard.as_mut() {
            existing.fill(0);
        }
        *guard = Some(key.to_vec());
    }
}

#[cfg(target_os = "macos")]
fn clear_cached_master_key() {
    if let Ok(mut guard) = cached_master_key().write()
        && let Some(mut existing) = guard.take()
    {
        existing.fill(0);
    }
}

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
    keychain::store_master_key(key).await?;
    #[cfg(target_os = "macos")]
    write_cached_master_key(key);
    Ok(())
}

pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
    #[cfg(target_os = "macos")]
    if let Some(key) = read_cached_master_key() {
        return Ok(key);
    }

    let key = keychain::get_master_key().await?;
    #[cfg(target_os = "macos")]
    write_cached_master_key(&key);
    Ok(key)
}

pub async fn delete_master_key() -> Result<(), SecretError> {
    keychain::delete_master_key().await?;
    #[cfg(target_os = "macos")]
    clear_cached_master_key();
    Ok(())
}

pub async fn has_master_key() -> bool {
    #[cfg(target_os = "macos")]
    if read_cached_master_key().is_some() {
        return true;
    }

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
