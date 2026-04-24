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
    if !secure_store_cache_enabled() {
        return None;
    }
    cached_master_key()
        .read()
        .ok()
        .and_then(|guard| guard.as_ref().cloned())
}

#[cfg(target_os = "macos")]
fn write_cached_master_key(key: &[u8]) {
    if !secure_store_cache_enabled() {
        return;
    }
    if let Ok(mut guard) = cached_master_key().write() {
        if let Some(existing) = guard.as_mut() {
            existing.fill(0);
        }
        *guard = Some(key.to_vec());
    }
}

#[cfg(target_os = "macos")]
fn secure_store_cache_enabled() -> bool {
    std::env::var("THINCLAW_KEYCHAIN_CACHE")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureStoreProbe {
    pub available: bool,
    pub env_fallback: bool,
    pub detail: String,
    pub guidance: String,
}

impl SecureStoreProbe {
    fn available(detail: impl Into<String>) -> Self {
        Self {
            available: true,
            env_fallback: false,
            detail: detail.into(),
            guidance: String::new(),
        }
    }

    fn env_fallback(detail: impl Into<String>) -> Self {
        Self {
            available: true,
            env_fallback: true,
            detail: detail.into(),
            guidance: String::new(),
        }
    }

    #[allow(dead_code)]
    fn unavailable(detail: impl Into<String>, guidance: impl Into<String>) -> Self {
        Self {
            available: false,
            env_fallback: false,
            detail: detail.into(),
            guidance: guidance.into(),
        }
    }
}

pub async fn probe_availability() -> SecureStoreProbe {
    if std::env::var_os("SECRETS_MASTER_KEY").is_some() {
        if crate::platform::env_flag_enabled("THINCLAW_ALLOW_ENV_MASTER_KEY") {
            return SecureStoreProbe::env_fallback(
                "SECRETS_MASTER_KEY is configured and explicitly allowed; encrypted secrets can use the environment fallback.",
            );
        }

        return SecureStoreProbe::unavailable(
            "SECRETS_MASTER_KEY is present but ignored by strict defaults.",
            "Set THINCLAW_ALLOW_ENV_MASTER_KEY=1 deliberately for headless Linux/container deployments, or use the OS secure store.",
        );
    }

    probe_os_secure_store().await
}

#[cfg(target_os = "linux")]
async fn probe_os_secure_store() -> SecureStoreProbe {
    use secret_service::{EncryptionType, SecretService};

    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none()
        && std::env::var_os("XDG_RUNTIME_DIR").is_none()
    {
        return SecureStoreProbe::unavailable(
            "No Linux user D-Bus session was detected for Secret Service.",
            "Run ThinClaw from a logged-in desktop user session, start a Secret Service provider such as GNOME Keyring/KWallet, or set SECRETS_MASTER_KEY with THINCLAW_ALLOW_ENV_MASTER_KEY=1 for headless Linux and containers.",
        );
    }

    match SecretService::connect(EncryptionType::Dh).await {
        Ok(service) => match service.get_default_collection().await {
            Ok(_) => SecureStoreProbe::available(
                "Linux Secret Service is reachable through the user D-Bus session.",
            ),
            Err(error) => SecureStoreProbe::unavailable(
                format!(
                    "Linux Secret Service connected but no default collection is usable: {error}"
                ),
                "Unlock or configure GNOME Keyring/KWallet, or set SECRETS_MASTER_KEY with THINCLAW_ALLOW_ENV_MASTER_KEY=1 for headless Linux and containers.",
            ),
        },
        Err(error) => SecureStoreProbe::unavailable(
            format!("Linux Secret Service is not reachable: {error}"),
            "Install/start GNOME Keyring or KWallet in a user D-Bus session, or set SECRETS_MASTER_KEY with THINCLAW_ALLOW_ENV_MASTER_KEY=1 for headless Linux and containers.",
        ),
    }
}

#[cfg(target_os = "macos")]
async fn probe_os_secure_store() -> SecureStoreProbe {
    SecureStoreProbe::available("macOS Keychain is the configured OS secure store.")
}

#[cfg(target_os = "windows")]
async fn probe_os_secure_store() -> SecureStoreProbe {
    SecureStoreProbe::available("Windows Credential Manager is the configured OS secure store.")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
async fn probe_os_secure_store() -> SecureStoreProbe {
    SecureStoreProbe::unavailable(
        "No OS secure store implementation is available on this platform.",
        "Set SECRETS_MASTER_KEY with THINCLAW_ALLOW_ENV_MASTER_KEY=1 to enable the environment fallback.",
    )
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

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::sync::LazyLock;

    use super::*;

    static ENV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    struct ScopedEnvVar {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl ScopedEnvVar {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            // SAFETY: these tests serialize environment mutation with ENV_LOCK.
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            // SAFETY: these tests serialize environment mutation with ENV_LOCK.
            unsafe {
                std::env::remove_var(key);
            }
            Self { key, previous }
        }
    }

    impl Drop for ScopedEnvVar {
        fn drop(&mut self) {
            // SAFETY: these tests serialize environment mutation with ENV_LOCK.
            unsafe {
                if let Some(previous) = self.previous.as_ref() {
                    std::env::set_var(self.key, previous);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn env_master_key_requires_explicit_allow_flag() {
        let _lock = ENV_LOCK.lock().await;
        let _key = ScopedEnvVar::set("SECRETS_MASTER_KEY", "test-master-key");
        let _allow = ScopedEnvVar::remove("THINCLAW_ALLOW_ENV_MASTER_KEY");

        let probe = probe_availability().await;
        assert!(!probe.available);
        assert!(!probe.env_fallback);
        assert!(probe.detail.contains("ignored by strict defaults"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn env_master_key_can_be_allowed_deliberately() {
        let _lock = ENV_LOCK.lock().await;
        let _key = ScopedEnvVar::set("SECRETS_MASTER_KEY", "test-master-key");
        let _allow = ScopedEnvVar::set("THINCLAW_ALLOW_ENV_MASTER_KEY", "1");

        let probe = probe_availability().await;
        assert!(probe.available);
        assert!(probe.env_fallback);
        assert!(probe.detail.contains("explicitly allowed"));
    }
}
