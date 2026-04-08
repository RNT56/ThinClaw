//! OS keychain integration for secrets master key storage.
//!
//! Provides platform-specific keychain support:
//! - macOS: security-framework (Keychain Services)
//! - Linux: secret-service (GNOME Keyring, KWallet)
//!
//! # Example
//!
//! ```ignore
//! use thinclaw::secrets::keychain::{store_master_key, get_master_key, delete_master_key};
//!
//! // Generate and store a new master key
//! let key = generate_master_key();
//! store_master_key(&key)?;
//!
//! // Later, retrieve it
//! let key = get_master_key()?;
//! ```

use crate::secrets::SecretError;

/// Service name for keychain entries.
const SERVICE_NAME: &str = "thinclaw";

/// Account name for the master key.
const MASTER_KEY_ACCOUNT: &str = "master_key";

/// Generate a random 32-byte master key.
pub fn generate_master_key() -> Vec<u8> {
    use rand::RngCore;
    let mut key = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Generate a master key as a hex string.
pub fn generate_master_key_hex() -> String {
    let bytes = generate_master_key();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

// ============================================================================
// macOS implementation using security-framework
// ============================================================================

#[cfg(target_os = "macos")]
mod platform {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn get_cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Store the master key in the macOS Keychain.
    pub async fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
        // Convert to hex for storage (keychain prefers strings)
        let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();

        set_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT, key_hex.as_bytes()).map_err(
            |e| SecretError::KeychainError(format!("Failed to store in keychain: {}", e)),
        )?;

        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(MASTER_KEY_ACCOUNT.to_string(), key_hex.as_bytes().to_vec());
        }
        Ok(())
    }

    /// Retrieve the master key from the macOS Keychain.
    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        if let Ok(cache) = get_cache().lock() {
            if let Some(password) = cache.get(MASTER_KEY_ACCOUNT) {
                let hex_str = String::from_utf8(password.clone()).map_err(|_| {
                    SecretError::KeychainError("Invalid UTF-8 in keychain".to_string())
                })?;
                return hex_to_bytes(&hex_str);
            }
        }

        let password = get_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).map_err(|e| {
            SecretError::KeychainError(format!("Failed to get from keychain: {}", e))
        })?;

        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(MASTER_KEY_ACCOUNT.to_string(), password.clone());
        }

        // Parse hex string back to bytes
        let hex_str = String::from_utf8(password)
            .map_err(|_| SecretError::KeychainError("Invalid UTF-8 in keychain".to_string()))?;

        hex_to_bytes(&hex_str)
    }

    /// Delete the master key from the macOS Keychain.
    pub async fn delete_master_key() -> Result<(), SecretError> {
        delete_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT).map_err(|e| {
            SecretError::KeychainError(format!("Failed to delete from keychain: {}", e))
        })?;
        if let Ok(mut cache) = get_cache().lock() {
            cache.remove(MASTER_KEY_ACCOUNT);
        }
        Ok(())
    }

    /// Check if a master key exists in the keychain.
    pub async fn has_master_key() -> bool {
        if let Ok(cache) = get_cache().lock() {
            if cache.contains_key(MASTER_KEY_ACCOUNT) {
                return true;
            }
        }
        match get_generic_password(SERVICE_NAME, MASTER_KEY_ACCOUNT) {
            Ok(password) => {
                if let Ok(mut cache) = get_cache().lock() {
                    cache.insert(MASTER_KEY_ACCOUNT.to_string(), password);
                }
                true
            }
            Err(_) => false,
        }
    }

    /// Store an arbitrary API key string in the keychain.
    ///
    /// `account` is the keychain account name (e.g., `claude_code_api_key`).
    pub async fn store_api_key(account: &str, value: &str) -> Result<(), SecretError> {
        set_generic_password(SERVICE_NAME, account, value.as_bytes()).map_err(|e| {
            SecretError::KeychainError(format!("Failed to store {} in keychain: {}", account, e))
        })?;
        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(account.to_string(), value.as_bytes().to_vec());
        }
        Ok(())
    }

    /// Retrieve an API key string from the keychain.
    ///
    /// Returns `None` if the key doesn't exist (rather than an error).
    pub async fn get_api_key(account: &str) -> Option<String> {
        if let Ok(cache) = get_cache().lock() {
            if let Some(password) = cache.get(account) {
                return String::from_utf8(password.clone()).ok();
            }
        }
        match get_generic_password(SERVICE_NAME, account) {
            Ok(bytes) => {
                let s = String::from_utf8(bytes.clone()).ok()?;
                if let Ok(mut cache) = get_cache().lock() {
                    cache.insert(account.to_string(), bytes);
                }
                Some(s)
            }
            Err(_) => None,
        }
    }
}

// ============================================================================
// Linux implementation using secret-service
// ============================================================================

#[cfg(target_os = "linux")]
mod platform {
    use secret_service::{EncryptionType, SecretService};
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn get_cache() -> &'static Mutex<HashMap<String, Vec<u8>>> {
        static CACHE: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
        CACHE.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Store the master key in the Linux secret service (GNOME Keyring, KWallet).
    pub async fn store_master_key(key: &[u8]) -> Result<(), SecretError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to connect to secret service: {}", e))
            })?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to get collection: {}", e)))?;

        // Unlock if needed
        if collection.is_locked().await.unwrap_or(true) {
            collection.unlock().await.map_err(|e| {
                SecretError::KeychainError(format!("Failed to unlock collection: {}", e))
            })?;
        }

        // Convert to hex for storage
        let key_hex: String = key.iter().map(|b| format!("{:02x}", b)).collect();

        collection
            .create_item(
                &format!("{} master key", SERVICE_NAME),
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
                key_hex.as_bytes(),
                true, // Replace if exists
                "text/plain",
            )
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to create secret: {}", e)))?;

        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(MASTER_KEY_ACCOUNT.to_string(), key_hex.as_bytes().to_vec());
        }

        Ok(())
    }

    /// Retrieve the master key from the Linux secret service.
    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        if let Ok(cache) = get_cache().lock() {
            if let Some(password) = cache.get(MASTER_KEY_ACCOUNT) {
                let hex_str = String::from_utf8(password.clone()).map_err(|_| {
                    SecretError::KeychainError("Invalid UTF-8 in secret".to_string())
                })?;
                return hex_to_bytes(&hex_str);
            }
        }

        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to connect to secret service: {}", e))
            })?;

        let items = ss
            .search_items(
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
            )
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to search: {}", e)))?;

        let item = items
            .unlocked
            .first()
            .or(items.locked.first())
            .ok_or_else(|| SecretError::KeychainError("Master key not found".to_string()))?;

        // Unlock if needed
        if item.is_locked().await.unwrap_or(true) {
            item.unlock()
                .await
                .map_err(|e| SecretError::KeychainError(format!("Failed to unlock: {}", e)))?;
        }

        let secret = item
            .get_secret()
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to get secret: {}", e)))?;

        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(MASTER_KEY_ACCOUNT.to_string(), secret.clone());
        }

        let hex_str = String::from_utf8(secret)
            .map_err(|_| SecretError::KeychainError("Invalid UTF-8 in secret".to_string()))?;

        hex_to_bytes(&hex_str)
    }

    /// Delete the master key from the Linux secret service.
    pub async fn delete_master_key() -> Result<(), SecretError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to connect to secret service: {}", e))
            })?;

        let items = ss
            .search_items(
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
            )
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to search: {}", e)))?;

        for item in items.unlocked.iter().chain(items.locked.iter()) {
            item.delete()
                .await
                .map_err(|e| SecretError::KeychainError(format!("Failed to delete: {}", e)))?;
        }

        if let Ok(mut cache) = get_cache().lock() {
            cache.remove(MASTER_KEY_ACCOUNT);
        }

        Ok(())
    }

    /// Check if a master key exists in the secret service.
    pub async fn has_master_key() -> bool {
        if let Ok(cache) = get_cache().lock() {
            if cache.contains_key(MASTER_KEY_ACCOUNT) {
                return true;
            }
        }

        let ss = match SecretService::connect(EncryptionType::Dh).await {
            Ok(ss) => ss,
            Err(_) => return false,
        };

        let items = match ss
            .search_items(
                [("service", SERVICE_NAME), ("account", MASTER_KEY_ACCOUNT)]
                    .into_iter()
                    .collect(),
            )
            .await
        {
            Ok(items) => items,
            Err(_) => return false,
        };

        let exists = !items.unlocked.is_empty() || !items.locked.is_empty();
        if exists {
            // We lazily cache the actual password when it's requested via get_master_key
            // However, we don't have the password right here. We could return true.
        }
        exists
    }

    /// Store an arbitrary API key string in the secret service.
    pub async fn store_api_key(account: &str, value: &str) -> Result<(), SecretError> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to connect to secret service: {}", e))
            })?;

        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| SecretError::KeychainError(format!("Failed to get collection: {}", e)))?;

        if collection.is_locked().await.unwrap_or(true) {
            collection.unlock().await.map_err(|e| {
                SecretError::KeychainError(format!("Failed to unlock collection: {}", e))
            })?;
        }

        collection
            .create_item(
                &format!("{} {}", SERVICE_NAME, account),
                [("service", SERVICE_NAME), ("account", account)]
                    .into_iter()
                    .collect(),
                value.as_bytes(),
                true,
                "text/plain",
            )
            .await
            .map_err(|e| {
                SecretError::KeychainError(format!("Failed to store {}: {}", account, e))
            })?;

        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(account.to_string(), value.as_bytes().to_vec());
        }
        Ok(())
    }

    /// Retrieve an API key string from the secret service.
    pub async fn get_api_key(account: &str) -> Option<String> {
        if let Ok(cache) = get_cache().lock() {
            if let Some(password) = cache.get(account) {
                return String::from_utf8(password.clone()).ok();
            }
        }

        let ss = SecretService::connect(EncryptionType::Dh).await.ok()?;
        let items = ss
            .search_items(
                [("service", SERVICE_NAME), ("account", account)]
                    .into_iter()
                    .collect(),
            )
            .await
            .ok()?;
        let item = items.unlocked.first().or(items.locked.first())?;
        if item.is_locked().await.unwrap_or(true) {
            item.unlock().await.ok()?;
        }
        let secret = item.get_secret().await.ok()?;

        // Update cache
        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(account.to_string(), secret.clone());
        }

        String::from_utf8(secret).ok()
    }
}

// ============================================================================
// Fallback for unsupported platforms
// ============================================================================

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;

    pub async fn store_master_key(_key: &[u8]) -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use SECRETS_MASTER_KEY env var.".to_string(),
        ))
    }

    pub async fn get_master_key() -> Result<Vec<u8>, SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use SECRETS_MASTER_KEY env var.".to_string(),
        ))
    }

    pub async fn delete_master_key() -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform".to_string(),
        ))
    }

    pub async fn has_master_key() -> bool {
        false
    }

    pub async fn store_api_key(_account: &str, _value: &str) -> Result<(), SecretError> {
        Err(SecretError::KeychainError(
            "Keychain not supported on this platform. Use environment variables.".to_string(),
        ))
    }

    pub async fn get_api_key(_account: &str) -> Option<String> {
        None
    }
}

// Re-export platform-specific functions
pub use platform::{
    delete_master_key, get_api_key, get_master_key, has_master_key, store_api_key, store_master_key,
};

/// Keychain account name for the Claude Code API key.
pub const CLAUDE_CODE_API_KEY_ACCOUNT: &str = "claude_code_api_key";

/// Parse a hex string to bytes.
fn hex_to_bytes(hex: &str) -> Result<Vec<u8>, SecretError> {
    if !hex.len().is_multiple_of(2) {
        return Err(SecretError::KeychainError(
            "Invalid hex string length".to_string(),
        ));
    }

    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| SecretError::KeychainError("Invalid hex character".to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_master_key() {
        let key = generate_master_key();
        assert_eq!(key.len(), 32);

        // Should be different each time
        let key2 = generate_master_key();
        assert_ne!(key, key2);
    }

    #[test]
    fn test_generate_master_key_hex() {
        let hex = generate_master_key_hex();
        assert_eq!(hex.len(), 64); // 32 bytes * 2 hex chars
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_hex_to_bytes() {
        let result = hex_to_bytes("deadbeef").unwrap();
        assert_eq!(result, vec![0xde, 0xad, 0xbe, 0xef]);

        let result = hex_to_bytes("00ff").unwrap();
        assert_eq!(result, vec![0x00, 0xff]);
    }

    #[test]
    fn test_hex_to_bytes_invalid() {
        assert!(hex_to_bytes("abc").is_err()); // Odd length
        assert!(hex_to_bytes("gg").is_err()); // Invalid chars
    }
}
