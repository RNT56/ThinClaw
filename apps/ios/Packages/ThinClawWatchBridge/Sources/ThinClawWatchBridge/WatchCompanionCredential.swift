#if canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAuth

    /// The watch's persisted view of its companion credential — the durable form
    /// of a received ``CompanionProvisioning``. Stored in the **watch's own**
    /// keychain (`AfterFirstUnlockThisDeviceOnly`, D-K2), never shared with the
    /// phone: the watch holds its own independently-revocable token.
    public struct WatchCompanionCredential: Codable, Sendable, Equatable {
        public var watchToken: String
        public var companionDeviceID: String
        public var parentDeviceID: String
        public var gatewayURLs: [URL]
        public var serverFingerprint: String?
        public var instanceID: String
        public var installationID: String

        public init(from provisioning: CompanionProvisioning) {
            self.watchToken = provisioning.watchToken
            self.companionDeviceID = provisioning.companionDeviceID
            self.parentDeviceID = provisioning.parentDeviceID
            self.gatewayURLs = provisioning.gatewayURLs
            self.serverFingerprint = provisioning.serverFingerprint
            self.instanceID = provisioning.instanceID
            self.installationID = provisioning.installationID
        }

        /// The `DeviceCredential` shape the shared connection/policy layers
        /// (`preferredBaseURL`, `PinnedSessionDelegate`) understand, so the watch's
        /// direct route pins and URL-selects exactly like the phone (D-X2). The
        /// watch's own token rides as `deviceToken`.
        public var deviceCredential: DeviceCredential {
            DeviceCredential(
                installationID: installationID,
                deviceID: companionDeviceID,
                deviceToken: watchToken,
                gatewayURLs: gatewayURLs,
                serverFingerprint: serverFingerprint,
                gatewayName: "",
                pairedAt: Date(timeIntervalSince1970: 0))
        }

        /// Keychain key for the watch companion credential (distinct from the
        /// phone's `KeychainKey.deviceCredential`).
        public static let keychainKey = "watch-companion-credential"

        public static func load(
            from keychain: some KeychainStoring
        ) throws -> WatchCompanionCredential? {
            try keychain.codable(WatchCompanionCredential.self, for: keychainKey)
        }

        public func save(to keychain: some KeychainStoring) throws {
            try keychain.setCodable(self, for: Self.keychainKey)
        }

        public static func erase(from keychain: some KeychainStoring) throws {
            try keychain.removeSecret(for: keychainKey)
        }

        /// The state the watch reports to the phone so it can decide whether to
        /// (re-)provision.
        public var reportedState: CompanionCredentialState {
            CompanionCredentialState(hasCredential: true, companionDeviceID: companionDeviceID)
        }
    }
#endif
