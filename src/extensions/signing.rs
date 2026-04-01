//! ed25519 signature verification for extension manifests.
//!
//! Provides cryptographic verification for WASM extension and channel
//! manifests to ensure they haven't been tampered with.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};

/// Verify an ed25519 signature on a manifest.
///
/// # Arguments
/// - `public_key` — 32-byte ed25519 public key
/// - `manifest_bytes` — the raw manifest bytes that were signed
/// - `signature_bytes` — 64-byte ed25519 signature
///
/// Returns `true` if the signature is valid.
pub fn verify_manifest_signature(
    public_key: &[u8; 32],
    manifest_bytes: &[u8],
    signature_bytes: &[u8; 64],
) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let signature = Signature::from_bytes(signature_bytes);
    verifying_key.verify(manifest_bytes, &signature).is_ok()
}

/// Parse a hex-encoded signature string into bytes.
pub fn parse_hex_signature(hex_str: &str) -> Option<[u8; 64]> {
    let bytes = hex::decode(hex_str).ok()?;
    if bytes.len() != 64 {
        return None;
    }
    let mut arr = [0u8; 64];
    arr.copy_from_slice(&bytes);
    Some(arr)
}

/// Parse a hex-encoded public key string into bytes.
pub fn parse_hex_public_key(hex_str: &str) -> Option<[u8; 32]> {
    let bytes = hex::decode(hex_str).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    Some(arr)
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::*;

    /// Create a deterministic signing key from a seed byte.
    fn test_key(seed: u8) -> SigningKey {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        bytes[31] = seed.wrapping_mul(37);
        SigningKey::from_bytes(&bytes)
    }

    #[test]
    fn test_sign_and_verify_roundtrip() {
        let signing_key = test_key(1);
        let verifying_key = signing_key.verifying_key();
        let manifest = b"name = my_extension\nversion = 1.0.0";

        let signature = signing_key.sign(manifest);

        assert!(verify_manifest_signature(
            verifying_key.as_bytes(),
            manifest,
            &signature.to_bytes(),
        ));
    }

    #[test]
    fn test_wrong_key_fails() {
        let signing_key = test_key(1);
        let wrong_key = test_key(2);
        let manifest = b"name = my_extension";

        let signature = signing_key.sign(manifest);

        assert!(!verify_manifest_signature(
            wrong_key.verifying_key().as_bytes(),
            manifest,
            &signature.to_bytes(),
        ));
    }

    #[test]
    fn test_tampered_manifest_fails() {
        let signing_key = test_key(3);
        let verifying_key = signing_key.verifying_key();
        let manifest = b"name = my_extension";

        let signature = signing_key.sign(manifest);

        assert!(!verify_manifest_signature(
            verifying_key.as_bytes(),
            b"name = evil_extension", // Tampered
            &signature.to_bytes(),
        ));
    }

    #[test]
    fn test_parse_hex_signature() {
        let hex = "aa".repeat(64); // 64 bytes = 128 hex chars
        let result = parse_hex_signature(&hex);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 64);
    }

    #[test]
    fn test_parse_hex_signature_wrong_length() {
        assert!(parse_hex_signature("deadbeef").is_none());
    }

    #[test]
    fn test_parse_hex_public_key() {
        let hex = "bb".repeat(32); // 32 bytes = 64 hex chars
        let result = parse_hex_public_key(&hex);
        assert!(result.is_some());
        assert_eq!(result.unwrap().len(), 32);
    }

    #[test]
    fn test_parse_hex_public_key_wrong_length() {
        assert!(parse_hex_public_key("deadbeef").is_none());
    }
}
