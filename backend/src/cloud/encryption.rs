//! Client-side encryption for cloud storage.
//!
//! All data is encrypted before upload and decrypted after download.
//! The cloud provider never sees plaintext.
//!
//! # Architecture
//!
//! - **Master key** (256-bit random) is stored in macOS Keychain
//! - **Per-file keys** are derived via HKDF-SHA256(master_key, file_path)
//! - **Algorithm:** AES-256-GCM with random 96-bit nonce
//! - **File format:** SCRY header + nonce + ciphertext + auth tag
//!
//! # Recovery Key
//!
//! The master key can be exported as a base64 "recovery key" for the user
//! to back up. Without this key, cloud data is irrecoverable (by design).

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroize;

/// Magic bytes identifying an encrypted Scrappy file.
const MAGIC: &[u8; 4] = b"SCRY";
/// Current encryption format version.
const FORMAT_VERSION: u16 = 1;
/// Header size: magic (4) + version (2) + reserved (10) = 16 bytes.
const HEADER_SIZE: usize = 16;
/// Nonce size for AES-256-GCM.
const NONCE_SIZE: usize = 12;
/// Auth tag is included in ciphertext by aes-gcm crate.

// ── Master Key Management ────────────────────────────────────────────────────

/// The encryption master key (256-bit / 32 bytes).
///
/// Zeroized on drop for security.
#[derive(Clone, Zeroize)]
#[zeroize(drop)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// Generate a new random master key.
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Self(key)
    }

    /// Import from raw bytes (e.g. from Keychain).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Import from a base64-encoded recovery key.
    pub fn from_recovery_key(recovery_key: &str) -> Result<Self, EncryptionError> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(recovery_key.trim())
            .map_err(|e| EncryptionError::InvalidKey(format!("Invalid base64: {}", e)))?;
        if bytes.len() != 32 {
            return Err(EncryptionError::InvalidKey(format!(
                "Expected 32 bytes, got {}",
                bytes.len()
            )));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }

    /// Export as a base64 recovery key (for user backup).
    pub fn to_recovery_key(&self) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(self.0)
    }

    /// Get raw bytes (for HKDF derivation).
    fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MasterKey([REDACTED])")
    }
}

// ── Key Derivation ───────────────────────────────────────────────────────────

/// Derive a per-file encryption key from the master key and file path.
///
/// Uses HKDF-SHA256 so the same file always gets the same derived key.
/// This enables incremental sync: unchanged files don't need re-encryption.
fn derive_file_key(master: &MasterKey, file_path: &str) -> Key<Aes256Gcm> {
    let hk = Hkdf::<Sha256>::new(None, master.as_bytes());
    let info = format!("scrappy-cloud-file:{}", file_path);
    let mut okm = [0u8; 32];
    hk.expand(info.as_bytes(), &mut okm)
        .expect("HKDF-SHA256 expand should not fail for 32-byte output");
    Key::<Aes256Gcm>::from(okm)
}

// ── Encrypt / Decrypt ────────────────────────────────────────────────────────

/// Encryption errors.
#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("Encryption failed: {0}")]
    EncryptFailed(String),

    #[error("Decryption failed: {0}")]
    DecryptFailed(String),

    #[error("Compression failed: {0}")]
    CompressionFailed(String),

    #[error("Invalid encrypted file: {0}")]
    InvalidFormat(String),

    #[error("Invalid key: {0}")]
    InvalidKey(String),

    #[error("Keychain error: {0}")]
    Keychain(String),
}

/// Encrypt data for upload to cloud storage.
///
/// 1. Compress with zstd (level 3 — fast + good ratio)
/// 2. Derive per-file key via HKDF
/// 3. Encrypt with AES-256-GCM
/// 4. Prepend SCRY header + nonce
///
/// Returns the encrypted blob ready for upload.
pub fn encrypt(
    master_key: &MasterKey,
    file_path: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    // 1. Compress
    let compressed = zstd::encode_all(plaintext, 3)
        .map_err(|e| EncryptionError::CompressionFailed(e.to_string()))?;

    // 2. Derive per-file key
    let key = derive_file_key(master_key, file_path);

    // 3. Generate random nonce
    let nonce_bytes: [u8; NONCE_SIZE] = {
        use rand::RngCore;
        let mut buf = [0u8; NONCE_SIZE];
        OsRng.fill_bytes(&mut buf);
        buf
    };
    let nonce = Nonce::from_slice(&nonce_bytes);

    // 4. Encrypt (ciphertext includes auth tag)
    let cipher = Aes256Gcm::new(&key);
    let ciphertext = cipher
        .encrypt(nonce, compressed.as_ref())
        .map_err(|e| EncryptionError::EncryptFailed(format!("AES-GCM encrypt: {}", e)))?;

    // 5. Build output: header + nonce + ciphertext
    let mut output = Vec::with_capacity(HEADER_SIZE + NONCE_SIZE + ciphertext.len());

    // Header (16 bytes)
    output.extend_from_slice(MAGIC); // 4 bytes
    output.extend_from_slice(&FORMAT_VERSION.to_le_bytes()); // 2 bytes
    output.extend_from_slice(&[0u8; 10]); // 10 bytes reserved

    // Nonce (12 bytes)
    output.extend_from_slice(&nonce_bytes);

    // Ciphertext + auth tag
    output.extend_from_slice(&ciphertext);

    Ok(output)
}

/// Decrypt data downloaded from cloud storage.
///
/// 1. Parse SCRY header + nonce
/// 2. Derive per-file key via HKDF
/// 3. Decrypt with AES-256-GCM
/// 4. Decompress with zstd
///
/// Returns the original plaintext.
pub fn decrypt(
    master_key: &MasterKey,
    file_path: &str,
    encrypted: &[u8],
) -> Result<Vec<u8>, EncryptionError> {
    // 1. Validate minimum size
    let min_size = HEADER_SIZE + NONCE_SIZE + 16; // at minimum: header + nonce + auth tag
    if encrypted.len() < min_size {
        return Err(EncryptionError::InvalidFormat(format!(
            "File too small: {} bytes (minimum {})",
            encrypted.len(),
            min_size
        )));
    }

    // 2. Parse header
    let magic = &encrypted[0..4];
    if magic != MAGIC {
        return Err(EncryptionError::InvalidFormat(format!(
            "Bad magic bytes: {:?} (expected {:?})",
            magic, MAGIC
        )));
    }

    let version = u16::from_le_bytes([encrypted[4], encrypted[5]]);
    if version != FORMAT_VERSION {
        return Err(EncryptionError::InvalidFormat(format!(
            "Unsupported format version: {} (expected {})",
            version, FORMAT_VERSION
        )));
    }

    // 3. Extract nonce
    let nonce_start = HEADER_SIZE;
    let nonce_end = nonce_start + NONCE_SIZE;
    let nonce = Nonce::from_slice(&encrypted[nonce_start..nonce_end]);

    // 4. Extract ciphertext (rest of file)
    let ciphertext = &encrypted[nonce_end..];

    // 5. Derive per-file key
    let key = derive_file_key(master_key, file_path);

    // 6. Decrypt
    let cipher = Aes256Gcm::new(&key);
    let compressed = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        EncryptionError::DecryptFailed(
            "AES-GCM decryption failed (wrong key or corrupted data)".into(),
        )
    })?;

    // 7. Decompress
    let plaintext = zstd::decode_all(compressed.as_slice())
        .map_err(|e| EncryptionError::DecryptFailed(format!("Decompression failed: {}", e)))?;

    Ok(plaintext)
}

// ── Keychain Integration (macOS) ─────────────────────────────────────────────

/// Keychain service name for the cloud encryption master key.
const KEYCHAIN_SERVICE: &str = "com.scrappy.cloud-key";
const KEYCHAIN_ACCOUNT: &str = "master-key";

/// Load the master key from macOS Keychain.
///
/// Returns `None` if no key has been stored yet.
#[cfg(target_os = "macos")]
pub fn load_master_key_from_keychain() -> Result<Option<MasterKey>, EncryptionError> {
    use security_framework::passwords::get_generic_password;

    match get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT) {
        Ok(data) => {
            if data.len() != 32 {
                return Err(EncryptionError::Keychain(format!(
                    "Keychain key has wrong size: {} bytes (expected 32)",
                    data.len()
                )));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&data);
            Ok(Some(MasterKey::from_bytes(key)))
        }
        Err(e) => {
            let code = e.code();
            if code == -25300 {
                // errSecItemNotFound
                Ok(None)
            } else {
                Err(EncryptionError::Keychain(format!(
                    "Failed to read from Keychain: {} (code: {})",
                    e, code
                )))
            }
        }
    }
}

/// Store the master key in macOS Keychain.
#[cfg(target_os = "macos")]
pub fn save_master_key_to_keychain(key: &MasterKey) -> Result<(), EncryptionError> {
    use security_framework::passwords::{delete_generic_password, set_generic_password};

    // Delete any existing key first (set_generic_password fails if exists)
    let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);

    set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, key.as_bytes())
        .map_err(|e| EncryptionError::Keychain(format!("Failed to save to Keychain: {}", e)))?;

    Ok(())
}

/// Delete the master key from macOS Keychain.
#[cfg(target_os = "macos")]
pub fn delete_master_key_from_keychain() -> Result<(), EncryptionError> {
    use security_framework::passwords::delete_generic_password;

    delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| EncryptionError::Keychain(format!("Failed to delete from Keychain: {}", e)))?;

    Ok(())
}

// ── Non-macOS stubs ──────────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
pub fn load_master_key_from_keychain() -> Result<Option<MasterKey>, EncryptionError> {
    Err(EncryptionError::Keychain(
        "Keychain is only supported on macOS".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn save_master_key_to_keychain(_key: &MasterKey) -> Result<(), EncryptionError> {
    Err(EncryptionError::Keychain(
        "Keychain is only supported on macOS".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn delete_master_key_from_keychain() -> Result<(), EncryptionError> {
    Err(EncryptionError::Keychain(
        "Keychain is only supported on macOS".into(),
    ))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = MasterKey::generate();
        let path = "documents/test.pdf";
        let data = b"Hello, cloud storage!";

        let encrypted = encrypt(&key, path, data).unwrap();
        let decrypted = decrypt(&key, path, &encrypted).unwrap();

        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_encrypt_decrypt_large_data() {
        let key = MasterKey::generate();
        let path = "db/openclaw.db";
        let data: Vec<u8> = (0..100_000).map(|i| (i % 256) as u8).collect();

        let encrypted = encrypt(&key, path, &data).unwrap();
        let decrypted = decrypt(&key, path, &encrypted).unwrap();

        assert_eq!(decrypted, data);
        // Compressed + encrypted should be smaller than raw for repetitive data
        assert!(encrypted.len() < data.len());
    }

    #[test]
    fn test_different_paths_different_ciphertext() {
        let key = MasterKey::generate();
        let data = b"Same data, different paths";

        let enc1 = encrypt(&key, "path/a.txt", data).unwrap();
        let enc2 = encrypt(&key, "path/b.txt", data).unwrap();

        // Different per-file keys → different ciphertext
        assert_ne!(enc1, enc2);

        // But both decrypt correctly with their own path
        assert_eq!(decrypt(&key, "path/a.txt", &enc1).unwrap(), data);
        assert_eq!(decrypt(&key, "path/b.txt", &enc2).unwrap(), data);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = MasterKey::generate();
        let key2 = MasterKey::generate();
        let data = b"Secret data";

        let encrypted = encrypt(&key1, "test.txt", data).unwrap();
        let result = decrypt(&key2, "test.txt", &encrypted);

        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_path_fails() {
        let key = MasterKey::generate();
        let data = b"Secret data";

        let encrypted = encrypt(&key, "correct/path.txt", data).unwrap();
        let result = decrypt(&key, "wrong/path.txt", &encrypted);

        assert!(result.is_err());
    }

    #[test]
    fn test_corrupted_data_fails() {
        let key = MasterKey::generate();
        let data = b"Secret data";

        let mut encrypted = encrypt(&key, "test.txt", data).unwrap();
        // Corrupt a byte in the ciphertext
        let last = encrypted.len() - 1;
        encrypted[last] ^= 0xFF;

        let result = decrypt(&key, "test.txt", &encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn test_bad_magic_fails() {
        let result = decrypt(
            &MasterKey::generate(),
            "test.txt",
            b"BADMxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
        );
        assert!(matches!(result, Err(EncryptionError::InvalidFormat(_))));
    }

    #[test]
    fn test_recovery_key_roundtrip() {
        let key = MasterKey::generate();
        let recovery = key.to_recovery_key();
        let restored = MasterKey::from_recovery_key(&recovery).unwrap();
        assert_eq!(key.as_bytes(), restored.as_bytes());
    }

    #[test]
    fn test_too_short_fails() {
        let result = decrypt(&MasterKey::generate(), "test.txt", b"SCRY");
        assert!(matches!(result, Err(EncryptionError::InvalidFormat(_))));
    }

    // ── A6-1: Extended encryption tests ──────────────────────────────────

    #[test]
    fn test_hkdf_deterministic_key_derivation() {
        // Same master key + same path → same derived key every time
        let master = MasterKey::from_bytes([42u8; 32]);
        let k1 = derive_file_key(&master, "images/photo.png");
        let k2 = derive_file_key(&master, "images/photo.png");
        assert_eq!(k1.as_slice(), k2.as_slice());

        // Different path → different derived key
        let k3 = derive_file_key(&master, "images/other.png");
        assert_ne!(k1.as_slice(), k3.as_slice());

        // Different master → different derived key for same path
        let master2 = MasterKey::from_bytes([99u8; 32]);
        let k4 = derive_file_key(&master2, "images/photo.png");
        assert_ne!(k1.as_slice(), k4.as_slice());
    }

    #[test]
    fn test_encrypted_file_format_structure() {
        // Verify the SCRY file format header layout
        let key = MasterKey::generate();
        let data = b"test payload";
        let encrypted = encrypt(&key, "test.txt", data).unwrap();

        // Header: SCRY magic (4) + version u16 LE (2) + reserved (10) = 16
        assert!(encrypted.len() >= HEADER_SIZE + NONCE_SIZE + 16); // min: header + nonce + GCM tag

        // Magic bytes
        assert_eq!(&encrypted[0..4], b"SCRY");

        // Version = 1 (little-endian u16)
        assert_eq!(encrypted[4], 1);
        assert_eq!(encrypted[5], 0);

        // Reserved bytes should be zero
        assert_eq!(&encrypted[6..16], &[0u8; 10]);

        // Nonce is 12 bytes starting at offset 16
        let nonce_region = &encrypted[16..28];
        // Nonce should not be all zeros (random, exceedingly unlikely)
        assert_ne!(nonce_region, &[0u8; 12]);

        // Ciphertext follows at offset 28
        assert!(encrypted.len() > 28);
    }

    #[test]
    fn test_zstd_compression_effective() {
        let key = MasterKey::generate();
        // Highly compressible data (all zeros)
        let data = vec![0u8; 10_000];
        let encrypted = encrypt(&key, "zeros.bin", &data).unwrap();

        // With zstd compression, 10KB of zeros → much less than 10KB
        // Header (16) + nonce (12) + compressed+encrypted should be << 10_000
        assert!(
            encrypted.len() < 500,
            "10KB of zeros should compress to <<500 bytes, got {}",
            encrypted.len()
        );

        // Roundtrip works
        let decrypted = decrypt(&key, "zeros.bin", &encrypted).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_empty_data_roundtrip() {
        let key = MasterKey::generate();
        let data: &[u8] = b"";

        let encrypted = encrypt(&key, "empty.txt", data).unwrap();
        let decrypted = decrypt(&key, "empty.txt", &encrypted).unwrap();

        assert_eq!(decrypted, data);
        // Even empty data has header + nonce + zstd frame + GCM tag
        assert!(encrypted.len() >= HEADER_SIZE + NONCE_SIZE);
    }

    #[test]
    fn test_large_binary_roundtrip() {
        // 1 MB of random-ish binary data (not compressible)
        let key = MasterKey::generate();
        let data: Vec<u8> = (0..1_000_000_u64)
            .map(|i| {
                // Simple deterministic pseudo-random (not crypto, just for test variety)
                let x = i
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (x >> 33) as u8
            })
            .collect();

        let encrypted = encrypt(&key, "big.bin", &data).unwrap();
        let decrypted = decrypt(&key, "big.bin", &encrypted).unwrap();

        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_recovery_key_invalid_base64() {
        let result = MasterKey::from_recovery_key("not valid base64!!!");
        assert!(matches!(result, Err(EncryptionError::InvalidKey(_))));
    }

    #[test]
    fn test_recovery_key_wrong_length() {
        use base64::Engine;
        // 16 bytes instead of 32
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        let result = MasterKey::from_recovery_key(&short);
        assert!(matches!(result, Err(EncryptionError::InvalidKey(_))));

        // 64 bytes instead of 32
        let long = base64::engine::general_purpose::STANDARD.encode([0u8; 64]);
        let result = MasterKey::from_recovery_key(&long);
        assert!(matches!(result, Err(EncryptionError::InvalidKey(_))));
    }

    #[test]
    fn test_version_mismatch_rejected() {
        let key = MasterKey::generate();
        let data = b"test";
        let mut encrypted = encrypt(&key, "test.txt", data).unwrap();

        // Tamper version to 99
        encrypted[4] = 99;
        encrypted[5] = 0;

        let result = decrypt(&key, "test.txt", &encrypted);
        assert!(matches!(result, Err(EncryptionError::InvalidFormat(_))));
        if let Err(EncryptionError::InvalidFormat(msg)) = result {
            assert!(
                msg.contains("version"),
                "Error should mention version: {}",
                msg
            );
        }
    }

    #[test]
    fn test_nonce_uniqueness() {
        // Encrypting the same data with the same key+path should produce
        // different ciphertexts (different random nonces each time)
        let key = MasterKey::generate();
        let data = b"identical content";
        let path = "same/path.txt";

        let enc1 = encrypt(&key, path, data).unwrap();
        let enc2 = encrypt(&key, path, data).unwrap();

        // Nonces (bytes 16..28) should differ
        assert_ne!(
            &enc1[HEADER_SIZE..HEADER_SIZE + NONCE_SIZE],
            &enc2[HEADER_SIZE..HEADER_SIZE + NONCE_SIZE],
            "Random nonces should differ between encryptions"
        );

        // Ciphertext differs (because nonce differs)
        assert_ne!(enc1, enc2);

        // Both decrypt correctly
        assert_eq!(decrypt(&key, path, &enc1).unwrap(), data);
        assert_eq!(decrypt(&key, path, &enc2).unwrap(), data);
    }

    #[test]
    fn test_truncated_ciphertext_fails() {
        let key = MasterKey::generate();
        let data = b"some data to encrypt";
        let encrypted = encrypt(&key, "trunc.txt", data).unwrap();

        // Truncate to just header + nonce (no ciphertext — but still at least GCM tag size)
        // Cut off the last 10 bytes of ciphertext/tag
        let truncated = &encrypted[..encrypted.len() - 10];
        let result = decrypt(&key, "trunc.txt", truncated);
        assert!(
            result.is_err(),
            "Truncated ciphertext should fail decryption"
        );
    }
}
