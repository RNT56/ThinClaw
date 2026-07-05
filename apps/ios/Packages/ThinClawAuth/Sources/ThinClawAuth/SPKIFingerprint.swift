#if canImport(CryptoKit)
    import CryptoKit
    import Foundation

    /// Computes the pinned-TLS fingerprint the way the gateway does: the bare
    /// base64url SHA-256 of a certificate's SubjectPublicKeyInfo (SPKI) DER
    /// (docs/MOBILE_SECURITY.md D-X1/D-X2). The QR payload's `fp` field carries
    /// exactly this value with no prefix.
    ///
    /// Pure over bytes so it unit-tests without a live `URLSession`: the
    /// delegate extracts the server leaf's SPKI DER, this hashes it, and the
    /// result is compared (constant-time) against the stored pin.
    public enum SPKIFingerprint {
        /// base64url(SHA-256(spkiDER)), unpadded — matches the gateway's `fp`.
        public static func base64url(spkiDER: Data) -> String {
            let digest = SHA256.hash(data: spkiDER)
            return Data(digest).base64URLEncodedString()
        }

        /// Constant-time comparison of a computed fingerprint against a stored
        /// pin. Both are short base64url ASCII strings; we still avoid an
        /// early-exit `==` so a timing side channel can't probe the pin.
        public static func matches(computed: String, pin: String) -> Bool {
            let a = Array(computed.utf8)
            let b = Array(pin.utf8)
            guard a.count == b.count else { return false }
            var diff: UInt8 = 0
            for index in a.indices {
                diff |= a[index] ^ b[index]
            }
            return diff == 0
        }
    }
#endif
