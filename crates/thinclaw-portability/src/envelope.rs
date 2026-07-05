//! Passphrase-based authenticated encryption for bundles.
//!
//! A bundle payload is sealed with XChaCha20-Poly1305 under a key derived from
//! the operator's passphrase via scrypt. Unlike the at-rest secrets crypto
//! (which uses a machine master key + HKDF), this is *portable*: the recipient
//! decrypts with the passphrase alone, on any machine.
//!
//! On-disk layout: `header || ciphertext`, where the plaintext header is also
//! the AEAD associated data, so any tampering with the KDF parameters, salt, or
//! nonce is detected as an authentication failure.
//!
//! ```text
//! offset  size  field
//! 0       8     magic  b"TCLAWBK1"
//! 8       1     kdf id (1 = scrypt)
//! 9       1     scrypt log_n
//! 10      4     scrypt r   (u32 little-endian)
//! 14      4     scrypt p   (u32 little-endian)
//! 18      16    salt
//! 34      24    nonce (XChaCha20-Poly1305)
//! 58      ..    ciphertext (includes 16-byte Poly1305 tag)
//! ```

use chacha20poly1305::aead::AeadInPlace;
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};

use crate::error::{PortabilityError, Result};

const MAGIC: &[u8; 8] = b"TCLAWBK1";
const KDF_SCRYPT: u8 = 1;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
const HEADER_LEN: usize = 8 + 1 + 1 + 4 + 4 + SALT_LEN + NONCE_LEN; // 58

/// scrypt cost parameters. `log_n = 15` ⇒ N = 32768 (~32 MiB), a sensible
/// interactive default for a backup passphrase.
const SCRYPT_LOG_N: u8 = 15;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;

/// Seal `plaintext` under `passphrase`, returning `header || ciphertext`.
pub fn seal(passphrase: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut salt);
    rand::Rng::fill_bytes(&mut rand::rng(), &mut nonce);

    let key = derive_key(passphrase, &salt, SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P)?;
    let header = build_header(&salt, &nonce, SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P);

    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
    let mut buffer = plaintext.to_vec();
    cipher
        .encrypt_in_place(XNonce::from_slice(&nonce), &header, &mut buffer)
        .map_err(|_| PortabilityError::KeyDerivation("AEAD encryption failed".to_string()))?;

    let mut out = Vec::with_capacity(header.len() + buffer.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(&buffer);
    Ok(out)
}

/// Open a sealed bundle, returning the plaintext. Returns
/// [`PortabilityError::Decryption`] for a wrong passphrase or any tampering
/// (the two are deliberately indistinguishable).
pub fn open(passphrase: &str, sealed: &[u8]) -> Result<Vec<u8>> {
    if sealed.len() < HEADER_LEN + TAG_LEN {
        return Err(PortabilityError::BadFormat(
            "input too short to be a bundle".to_string(),
        ));
    }
    let (header, ciphertext) = sealed.split_at(HEADER_LEN);
    if &header[0..8] != MAGIC {
        return Err(PortabilityError::BadFormat(
            "missing bundle magic".to_string(),
        ));
    }
    if header[8] != KDF_SCRYPT {
        return Err(PortabilityError::BadFormat(format!(
            "unknown KDF id {}",
            header[8]
        )));
    }
    let log_n = header[9];
    let r = u32::from_le_bytes([header[10], header[11], header[12], header[13]]);
    let p = u32::from_le_bytes([header[14], header[15], header[16], header[17]]);
    let salt = &header[18..18 + SALT_LEN];
    let nonce = &header[34..34 + NONCE_LEN];

    let key = derive_key(passphrase, salt, log_n, r, p)?;
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key));
    let mut buffer = ciphertext.to_vec();
    cipher
        .decrypt_in_place(XNonce::from_slice(nonce), header, &mut buffer)
        .map_err(|_| PortabilityError::Decryption)?;
    Ok(buffer)
}

fn build_header(salt: &[u8], nonce: &[u8], log_n: u8, r: u32, p: u32) -> Vec<u8> {
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(KDF_SCRYPT);
    header.push(log_n);
    header.extend_from_slice(&r.to_le_bytes());
    header.extend_from_slice(&p.to_le_bytes());
    header.extend_from_slice(salt);
    header.extend_from_slice(nonce);
    debug_assert_eq!(header.len(), HEADER_LEN);
    header
}

fn derive_key(passphrase: &str, salt: &[u8], log_n: u8, r: u32, p: u32) -> Result<[u8; KEY_LEN]> {
    let params = scrypt::Params::new(log_n, r, p, KEY_LEN)
        .map_err(|e| PortabilityError::KeyDerivation(format!("invalid scrypt params: {e}")))?;
    let mut key = [0u8; KEY_LEN];
    scrypt::scrypt(passphrase.as_bytes(), salt, &params, &mut key)
        .map_err(|e| PortabilityError::KeyDerivation(format!("scrypt failed: {e}")))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let plaintext = b"the quick brown fox jumps over the lazy dog";
        let sealed = seal("correct horse battery staple", plaintext).unwrap();
        assert!(sealed.len() > plaintext.len());
        let opened = open("correct horse battery staple", &sealed).unwrap();
        assert_eq!(opened, plaintext);
    }

    #[test]
    fn wrong_passphrase_fails() {
        let sealed = seal("hunter2", b"secret payload").unwrap();
        let err = open("hunter3", &sealed).unwrap_err();
        assert!(matches!(err, PortabilityError::Decryption));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let mut sealed = seal("pw", b"payload").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xFF;
        assert!(matches!(
            open("pw", &sealed).unwrap_err(),
            PortabilityError::Decryption
        ));
    }

    #[test]
    fn tampered_header_fails() {
        // Flip a bit in the salt: the key changes AND the AAD no longer matches.
        let mut sealed = seal("pw", b"payload").unwrap();
        sealed[20] ^= 0x01;
        assert!(matches!(
            open("pw", &sealed).unwrap_err(),
            PortabilityError::Decryption
        ));
    }

    #[test]
    fn truncated_input_is_bad_format() {
        let sealed = seal("pw", b"payload").unwrap();
        assert!(matches!(
            open("pw", &sealed[..10]).unwrap_err(),
            PortabilityError::BadFormat(_)
        ));
    }

    #[test]
    fn non_bundle_is_bad_format() {
        let junk = vec![0u8; 200];
        assert!(matches!(
            open("pw", &junk).unwrap_err(),
            PortabilityError::BadFormat(_)
        ));
    }

    #[test]
    fn distinct_seals_differ_but_open_same() {
        // Fresh salt+nonce each time ⇒ different ciphertext for identical input.
        let a = seal("pw", b"same").unwrap();
        let b = seal("pw", b"same").unwrap();
        assert_ne!(a, b);
        assert_eq!(open("pw", &a).unwrap(), open("pw", &b).unwrap());
    }

    #[test]
    fn empty_payload_round_trips() {
        let sealed = seal("pw", b"").unwrap();
        assert_eq!(open("pw", &sealed).unwrap(), b"");
    }
}
