#if canImport(Security)
    import Foundation
    import Security

    /// Shared, content-free Keychain item used by the notification extension to
    /// distinguish locked from unlocked state without exposing user data.
    public enum DeviceUnlockProbe {
        public static let service = "com.thinclaw.lockprobe"
        private static let account = "availability"

        public static func provision() throws {
            let query = baseQuery()
            SecItemDelete(query as CFDictionary)
            var attributes = query
            attributes[kSecValueData as String] = Data([1])
            attributes[kSecAttrAccessible as String] =
                kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            let status = SecItemAdd(attributes as CFDictionary, nil)
            guard status == errSecSuccess else {
                throw KeychainStoreError.unhandled(status: status)
            }
        }

        public static func isUnlocked() -> Bool {
            var query = baseQuery()
            query[kSecReturnData as String] = kCFBooleanFalse
            query[kSecMatchLimit as String] = kSecMatchLimitOne
            return SecItemCopyMatching(query as CFDictionary, nil) == errSecSuccess
        }

        private static func baseQuery() -> [String: Any] {
            [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecAttrAccount as String: account,
            ]
        }
    }
#endif
