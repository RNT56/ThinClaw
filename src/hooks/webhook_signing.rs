//! HMAC-based webhook signing and verification.
//!
//! Used for outbound webhook signing (routines, notifications) and
//! inbound webhook signature verification (channel integrations).
//!
//! Uses HMAC-SHA256 with constant-time comparison for security.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Sign a webhook payload with HMAC-SHA256.
///
/// Returns a hex-encoded signature prefixed with `sha256=`.
pub fn sign_payload(secret: &[u8], payload: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC-SHA256 accepts any key length");
    mac.update(payload);
    let result = mac.finalize();
    format!("sha256={}", hex::encode(result.into_bytes()))
}

/// Verify a webhook signature using constant-time comparison.
///
/// The `signature` parameter should be in `sha256=<hex>` format.
pub fn verify_signature(secret: &[u8], payload: &[u8], signature: &str) -> bool {
    let expected = sign_payload(secret, payload);
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), signature.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sign_and_verify() {
        let secret = b"my_webhook_secret";
        let payload = b"hello world";
        let sig = sign_payload(secret, payload);

        assert!(sig.starts_with("sha256="));
        assert!(verify_signature(secret, payload, &sig));
    }

    #[test]
    fn test_wrong_secret_fails() {
        let payload = b"hello world";
        let sig = sign_payload(b"secret1", payload);
        assert!(!verify_signature(b"secret2", payload, &sig));
    }

    #[test]
    fn test_wrong_payload_fails() {
        let secret = b"my_secret";
        let sig = sign_payload(secret, b"payload1");
        assert!(!verify_signature(secret, b"payload2", &sig));
    }

    #[test]
    fn test_tampered_signature_fails() {
        let secret = b"my_secret";
        let payload = b"hello";
        let sig = sign_payload(secret, payload);
        let tampered = format!("{}0", sig); // Append extra char
        assert!(!verify_signature(secret, payload, &tampered));
    }

    #[test]
    fn test_empty_payload() {
        let secret = b"key";
        let sig = sign_payload(secret, b"");
        assert!(sig.starts_with("sha256="));
        assert!(verify_signature(secret, b"", &sig));
    }

    #[test]
    fn test_deterministic() {
        let secret = b"key";
        let payload = b"data";
        let sig1 = sign_payload(secret, payload);
        let sig2 = sign_payload(secret, payload);
        assert_eq!(sig1, sig2);
    }
}
