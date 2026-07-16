#if canImport(Security)
    import Foundation
    import Security

    /// Real keychain-backed ``KeychainStoring`` using SecItem generic
    /// passwords.
    ///
    /// Configured with the shared keychain access group so the app, its
    /// widget extension, and app intents all read the same device
    /// credential. The access-group string passed to SecItem at runtime must
    /// be the *resolved* form `<TeamID>.com.thinclaw.shared` — the
    /// `$(AppIdentifierPrefix)` macro only exists in entitlement files, so
    /// callers pass `nil` (default private group) or the resolved string.
    public struct SecItemKeychainStore: KeychainStoring {
        /// Suffix of the shared access group; the app-identifier prefix
        /// (team id + ".") is prepended by the platform. Mirrors
        /// `$(AppIdentifierPrefix)com.thinclaw.shared` in the entitlements.
        public static let sharedAccessGroupSuffix = "com.thinclaw.shared"

        /// `kSecAttrService` namespace for all ThinClaw secrets.
        public let service: String
        /// Resolved keychain access group, or `nil` for the default group.
        public let accessGroup: String?

        public init(service: String = "com.thinclaw.ios", accessGroup: String? = nil) {
            self.service = service
            self.accessGroup = accessGroup
        }

        public func setSecret(_ data: Data, for key: String) throws {
            try setSecret(data, for: key, accessibility: .afterFirstUnlockDeviceOnly)
        }

        public func setSecret(
            _ data: Data,
            for key: String,
            accessibility: KeychainAccessibility
        ) throws {
            // Delete-then-add keeps the logic simple and attribute-complete
            // (SecItemUpdate cannot change accessibility attributes).
            try removeSecret(for: key)

            var attributes = baseQuery(for: key)
            attributes[kSecValueData as String] = data
            // Device-only, available after first unlock: background refresh
            // and push handlers need the credential without UI unlock.
            attributes[kSecAttrAccessible as String] =
                switch accessibility {
                case .afterFirstUnlockDeviceOnly:
                    kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
                case .whenUnlockedDeviceOnly:
                    kSecAttrAccessibleWhenUnlockedThisDeviceOnly
                }

            let status = SecItemAdd(attributes as CFDictionary, nil)
            guard status == errSecSuccess else {
                throw KeychainStoreError.unhandled(status: status)
            }
        }

        public func secret(for key: String) throws -> Data? {
            var query = baseQuery(for: key)
            query[kSecReturnData as String] = kCFBooleanTrue
            query[kSecMatchLimit as String] = kSecMatchLimitOne

            var result: CFTypeRef?
            let status = SecItemCopyMatching(query as CFDictionary, &result)
            switch status {
            case errSecSuccess:
                guard let data = result as? Data else {
                    throw KeychainStoreError.invalidData
                }
                return data
            case errSecItemNotFound:
                return nil
            default:
                throw KeychainStoreError.unhandled(status: status)
            }
        }

        public func removeSecret(for key: String) throws {
            let status = SecItemDelete(baseQuery(for: key) as CFDictionary)
            guard status == errSecSuccess || status == errSecItemNotFound else {
                throw KeychainStoreError.unhandled(status: status)
            }
        }

        private func baseQuery(for key: String) -> [String: Any] {
            var query: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccount as String: key,
            ]
            if let accessGroup {
                query[kSecAttrAccessGroup as String] = accessGroup
            }
            return query
        }
    }
#endif
