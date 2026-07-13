//! Desktop secret-envelope recovery and master-key rotation.

use std::sync::{Arc, OnceLock};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use thinclaw_core::secrets::SecretsCrypto;
use tokio::sync::Mutex;
use zeroize::Zeroize;

use super::types::{SecretMasterKeyRotation, SecretRecoveryStatus};
use crate::thinclaw::config::keychain;

const RECOVERY_KEY_PREFIX: &str = "thinclaw-secrets-v1";
const RECOVERY_CHECKSUM_DOMAIN: &[u8] = b"thinclaw-desktop-secret-recovery-v1";
const ROTATE_CONFIRMATION: &str = "ROTATE";
const IMPORT_CONFIRMATION: &str = "REPLACE";

fn rotation_guard() -> &'static Mutex<()> {
    static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
    GUARD.get_or_init(|| Mutex::new(()))
}

fn require_supported_platform() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(
            "Desktop secret-envelope recovery is unavailable because persistent envelope storage is currently implemented only for macOS Keychain."
                .to_string(),
        )
    }
}

fn recovery_checksum(key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(RECOVERY_CHECKSUM_DOMAIN);
    hasher.update(key);
    let digest = hasher.finalize();
    hex::encode(&digest[..8])
}

fn encode_recovery_key(key: &[u8]) -> Result<String, String> {
    if key.len() != 32 {
        return Err("The secret master key must contain exactly 32 bytes.".to_string());
    }
    Ok(format!(
        "{RECOVERY_KEY_PREFIX}:{}:{}",
        URL_SAFE_NO_PAD.encode(key),
        recovery_checksum(key)
    ))
}

fn decode_recovery_key(encoded: &str) -> Result<Vec<u8>, String> {
    let mut parts = encoded.trim().split(':');
    let prefix = parts.next();
    let Some(payload) = parts.next() else {
        return Err("Recovery key format is invalid.".to_string());
    };
    let Some(checksum) = parts.next() else {
        return Err("Recovery key format is invalid.".to_string());
    };
    if prefix != Some(RECOVERY_KEY_PREFIX) || parts.next().is_some() {
        return Err("Recovery key format is invalid.".to_string());
    }

    let mut key = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| "Recovery key payload is not valid base64url data.".to_string())?;
    if key.len() != 32 {
        key.zeroize();
        return Err("Recovery key payload must contain exactly 32 bytes.".to_string());
    }
    if !recovery_checksum(&key).eq_ignore_ascii_case(checksum) {
        key.zeroize();
        return Err("Recovery key checksum does not match.".to_string());
    }
    Ok(key)
}

fn require_confirmation(mut confirmation: String, expected: &str) -> Result<(), String> {
    let confirmed = confirmation.trim() == expected;
    confirmation.zeroize();
    if confirmed {
        Ok(())
    } else {
        Err(format!("Type {expected} to confirm this operation."))
    }
}

async fn replace_master_key(mut new_key: Vec<u8>) -> Result<SecretMasterKeyRotation, String> {
    let mut old_key = thinclaw_core::platform::secure_store::get_master_key()
        .await
        .map_err(|error| format!("Failed to read the current secret master key: {error}"))?;
    if old_key == new_key {
        old_key.zeroize();
        new_key.zeroize();
        return Err("The supplied recovery key is already active.".to_string());
    }

    let new_key_hex = hex::encode(&new_key);
    let new_crypto = match SecretsCrypto::new(SecretString::from(new_key_hex)) {
        Ok(crypto) => Arc::new(crypto),
        Err(error) => {
            old_key.zeroize();
            new_key.zeroize();
            return Err(format!(
                "Failed to initialize replacement encryption: {error}"
            ));
        }
    };

    if let Err(error) = thinclaw_core::platform::secure_store::store_master_key(&new_key).await {
        old_key.zeroize();
        new_key.zeroize();
        return Err(format!(
            "Failed to persist the replacement master key: {error}"
        ));
    }

    let rotation = keychain::rotate_encryption_key(new_crypto);
    new_key.zeroize();
    let (old_key_version, new_key_version, rotated_secrets) = match rotation {
        Ok(report) => report,
        Err(error) => {
            let restore = thinclaw_core::platform::secure_store::store_master_key(&old_key).await;
            old_key.zeroize();
            return match restore {
                Ok(()) => Err(format!("Secret-envelope rotation failed: {error}")),
                Err(restore_error) => Err(format!(
                    "Secret-envelope rotation failed ({error}) and restoring the previous master key also failed ({restore_error})."
                )),
            };
        }
    };
    old_key.zeroize();

    Ok(SecretMasterKeyRotation {
        old_key_version,
        new_key_version,
        rotated_secrets: rotated_secrets as u64,
        recovery_key: None,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_secret_recovery_status() -> Result<SecretRecoveryStatus, String> {
    let (key_version, stored_secrets) = keychain::encryption_metadata()?;
    let supported = cfg!(target_os = "macos");
    Ok(SecretRecoveryStatus {
        supported,
        unavailable_reason: (!supported).then(|| {
            "Persistent Desktop secret-envelope recovery is currently implemented only for macOS Keychain."
                .to_string()
        }),
        cipher: "AES-256-GCM".to_string(),
        kdf: "HKDF-SHA256".to_string(),
        key_version,
        stored_secrets: stored_secrets as u64,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_secret_recovery_export() -> Result<String, String> {
    require_supported_platform()?;
    let _guard = rotation_guard().lock().await;
    let mut key = thinclaw_core::platform::secure_store::get_master_key()
        .await
        .map_err(|error| format!("Failed to read the secret master key: {error}"))?;
    let encoded = encode_recovery_key(&key);
    key.zeroize();
    encoded
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_secret_master_key_rotate(
    confirmation: String,
) -> Result<SecretMasterKeyRotation, String> {
    require_supported_platform()?;
    require_confirmation(confirmation, ROTATE_CONFIRMATION)?;
    let _guard = rotation_guard().lock().await;
    let mut key = thinclaw_core::platform::secure_store::generate_master_key();
    let recovery_key = encode_recovery_key(&key)?;
    let mut report = replace_master_key(std::mem::take(&mut key)).await?;
    key.zeroize();
    report.recovery_key = Some(recovery_key);
    Ok(report)
}

#[tauri::command]
#[specta::specta]
pub async fn thinclaw_secret_recovery_import(
    mut recovery_key: String,
    confirmation: String,
) -> Result<SecretMasterKeyRotation, String> {
    require_supported_platform()?;
    require_confirmation(confirmation, IMPORT_CONFIRMATION)?;
    let decoded = decode_recovery_key(&recovery_key);
    recovery_key.zeroize();
    let key = decoded?;
    let _guard = rotation_guard().lock().await;
    replace_master_key(key).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_key_round_trips_and_detects_corruption() {
        let key = vec![0x5a; 32];
        let encoded = encode_recovery_key(&key).expect("encode recovery key");
        assert_eq!(
            decode_recovery_key(&encoded).expect("decode recovery key"),
            key
        );

        let mut corrupted = encoded;
        let final_index = corrupted.len() - 1;
        let replacement = if corrupted.as_bytes()[final_index] == b'0' {
            "1"
        } else {
            "0"
        };
        corrupted.replace_range(final_index.., replacement);
        assert!(decode_recovery_key(&corrupted).is_err());
    }

    #[test]
    fn destructive_operations_require_exact_confirmation() {
        assert!(require_confirmation("ROTATE".to_string(), ROTATE_CONFIRMATION).is_ok());
        assert!(require_confirmation("rotate".to_string(), ROTATE_CONFIRMATION).is_err());
        assert!(require_confirmation("REPLACE".to_string(), IMPORT_CONFIRMATION).is_ok());
    }
}
