#if canImport(Security) && canImport(CryptoKit)
    import CryptoKit
    import Foundation
    import Security

    /// `URLSessionDelegate` enforcing the ThinClaw transport policy
    /// (docs/MOBILE_SECURITY.md **D-X2**):
    ///
    /// - **Pinned:** when a fingerprint is stored, the server leaf's SPKI
    ///   SHA-256 must match. Chain validation is *bypassed* for the pinned
    ///   anchor — a self-signed gateway cert has no public chain to validate,
    ///   and the pin is a strictly stronger check than a CA chain (T3/T4).
    /// - **No pin (`vpn-http` pairing):** plaintext HTTP is permitted only to
    ///   the Tailscale CGNAT space (and, in DEBUG, loopback); everything else
    ///   is refused. TLS to such gateways still validates via the standard
    ///   chain.
    /// - **Standard TLS:** always allowed as an alternative to a pin (public
    ///   chain), for operators fronting the gateway with a real certificate.
    ///
    /// The scheme/host policy lives in ``ConnectionPolicy``; this delegate owns
    /// only the parts that need the live `SecTrust` (SPKI extraction + hash
    /// compare).
    public final class PinnedSessionDelegate: NSObject, URLSessionDelegate, @unchecked Sendable {
        /// Stored SPKI pin (bare base64url SHA-256), or `nil` for a `vpn-http` /
        /// public-chain gateway.
        private let pinnedFingerprint: String?

        public init(pinnedFingerprint: String?) {
            self.pinnedFingerprint = pinnedFingerprint
        }

        /// Convenience: build a `URLSession` that enforces this policy.
        public func makeSession(
            configuration: URLSessionConfiguration = .ephemeral
        ) -> URLSession {
            URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        }

        public func urlSession(
            _ session: URLSession,
            didReceive challenge: URLAuthenticationChallenge,
            completionHandler:
                @escaping @Sendable (
                    URLSession.AuthChallengeDisposition, URLCredential?
                ) -> Void
        ) {
            guard
                challenge.protectionSpace.authenticationMethod
                    == NSURLAuthenticationMethodServerTrust,
                let serverTrust = challenge.protectionSpace.serverTrust
            else {
                completionHandler(.performDefaultHandling, nil)
                return
            }

            guard let pin = pinnedFingerprint else {
                // No pin: defer to the system's standard chain validation
                // (D-X2 "Public-chain TLS" column). This path is reached only
                // for https; plaintext-http gateways never open a TLS
                // challenge, and `ConnectionPolicy` gates whether the request
                // is even attempted.
                completionHandler(.performDefaultHandling, nil)
                return
            }

            switch Self.evaluatePin(serverTrust: serverTrust, expected: pin) {
            case .match:
                completionHandler(.useCredential, URLCredential(trust: serverTrust))
            case .mismatch, .unextractable:
                completionHandler(.cancelAuthenticationChallenge, nil)
            }
        }

        // MARK: - Pin evaluation (pure over a SecTrust)

        enum PinResult: Equatable {
            case match
            case mismatch
            /// Could not extract/encode the leaf SPKI (unknown key algorithm,
            /// missing key). Treated as a failure — never fall through to
            /// trusting an anchor whose pin we couldn't verify.
            case unextractable
        }

        /// Extract the leaf public key, reconstruct its SPKI DER, hash it, and
        /// compare (constant-time) against the expected pin.
        static func evaluatePin(serverTrust: SecTrust, expected: String) -> PinResult {
            guard let leafKey = SecTrustCopyKey(serverTrust) else {
                return .unextractable
            }
            guard let spkiDER = spkiDER(for: leafKey) else {
                return .unextractable
            }
            let computed = SPKIFingerprint.base64url(spkiDER: spkiDER)
            return SPKIFingerprint.matches(computed: computed, pin: expected)
                ? .match : .mismatch
        }

        /// Rebuild the SPKI DER for a leaf `SecKey`. Public so pin evaluation
        /// can be exercised against synthetic keys in tests.
        static func spkiDER(for key: SecKey) -> Data? {
            guard let algorithm = SPKIEncoder.algorithm(of: key) else { return nil }
            var error: Unmanaged<CFError>?
            guard let raw = SecKeyCopyExternalRepresentation(key, &error) as Data? else {
                return nil
            }
            return SPKIEncoder.spkiDER(rawKey: raw, algorithm: algorithm)
        }
    }
#endif
