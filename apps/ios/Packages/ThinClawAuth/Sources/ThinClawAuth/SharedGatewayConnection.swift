#if canImport(Security) && canImport(CryptoKit)
    import Foundation

    /// The one place the app **and** its extensions (Notification Service
    /// Extension, widgets) build their view of the paired gateway: the shared
    /// Keychain store, the stored ``DeviceCredential``, a **pinned** `URLSession`,
    /// and the policy-allowed base URL.
    ///
    /// The Notification Service Extension runs in a separate process from the app
    /// and must reach the gateway on its own (docs/MOBILE_SECURITY.md **D-N1**:
    /// "a Notification Service Extension fetches real content from the gateway
    /// over the pinned connection"). It reads the same device token from the
    /// shared Keychain access group and connects through the same pin, so this
    /// helper centralises that assembly instead of each surface re-deriving it
    /// (and risking an *unpinned* session — the D-X2 hazard called out on
    /// ``PinnedSessionDelegate`` and ``GatewayClient``).
    public enum SharedGatewayConnection {
        /// The shared Keychain access group holding the device credential
        /// (D-K1/D-K2). Passing `nil` lets `SecItem` default to the app's first
        /// entitled group, which — because the app, NSE, and widgets all declare
        /// only `$(AppIdentifierPrefix)com.thinclaw.shared` — resolves to the
        /// shared group in every process without hardcoding the runtime-only
        /// team prefix.
        public static func keychain() -> SecItemKeychainStore {
            SecItemKeychainStore()
        }

        /// Load the stored device credential from the shared Keychain, or `nil`
        /// when the device is not paired.
        public static func loadCredential() -> DeviceCredential? {
            (try? DeviceCredential.load(from: keychain())) ?? nil
        }

        /// A pinned `URLSession` for `credential` (SPKI pin when the credential
        /// carries a fingerprint, standard TLS otherwise), matching the app's
        /// production graph so extension traffic never bypasses TLS pinning
        /// (D-X2). The default `.ephemeral` configuration keeps no on-disk cache,
        /// which suits the short-lived NSE.
        public static func pinnedSession(
            for credential: DeviceCredential,
            configuration: URLSessionConfiguration = .ephemeral
        ) -> URLSession {
            PinnedSessionDelegate(pinnedFingerprint: credential.serverFingerprint)
                .makeSession(configuration: configuration)
        }
    }
#endif
