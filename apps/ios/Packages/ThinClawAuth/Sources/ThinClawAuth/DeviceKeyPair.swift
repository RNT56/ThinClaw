#if canImport(CryptoKit) && canImport(Security)
    import CryptoKit
    import Foundation
    import Security

    /// The device's P-256 identity key submitted at pairing (D-P2): the gateway
    /// stores the public SPKI now and can later flip on proof-of-possession
    /// without re-pairing.
    ///
    /// Prefers a Secure Enclave key (`kSecAttrTokenIDSecureEnclave`, private key
    /// non-exportable), and transparently falls back to a software CryptoKit
    /// key on platforms/targets without an enclave (the simulator). Either way
    /// the caller gets the **public** key as base64 SPKI for the pairing
    /// request body.
    public enum DeviceKeyPair {
        public enum Backing: Sendable, Equatable {
            case secureEnclave
            case software
        }

        /// A generated (or loaded) device key, described by what the pairing
        /// request needs.
        public struct Handle: Sendable, Equatable {
            /// Base64 (standard, padded — matches the gateway's `pubkey`
            /// field, which is plain base64 not base64url) SPKI of the public
            /// key.
            public var spkiBase64: String
            /// Where the private key lives.
            public var backing: Backing

            public init(spkiBase64: String, backing: Backing) {
                self.spkiBase64 = spkiBase64
                self.backing = backing
            }
        }

        public enum KeyError: Error, Equatable {
            case generationFailed(status: OSStatus)
            case publicKeyUnavailable
            case spkiEncodingFailed
        }

        /// Tag used for the Secure Enclave private key in the keychain.
        public static let keychainTag = "com.thinclaw.ios.device-key"

        /// Generate the device identity key, Secure Enclave when available.
        ///
        /// - Parameter forceSoftware: test/simulator hook to skip the enclave
        ///   attempt entirely.
        public static func generate(forceSoftware: Bool = false) throws -> Handle {
            if !forceSoftware, let handle = try? generateSecureEnclave() {
                return handle
            }
            return try generateSoftware()
        }

        // MARK: - Secure Enclave

        private static func generateSecureEnclave() throws -> Handle {
            guard
                let access = SecAccessControlCreateWithFlags(
                    nil,
                    kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
                    [.privateKeyUsage],
                    nil)
            else {
                throw KeyError.generationFailed(status: errSecParam)
            }

            let attributes: [String: Any] = [
                kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
                kSecAttrKeySizeInBits as String: 256,
                kSecAttrTokenID as String: kSecAttrTokenIDSecureEnclave,
                kSecPrivateKeyAttrs as String: [
                    kSecAttrIsPermanent as String: true,
                    kSecAttrApplicationTag as String: Data(keychainTag.utf8),
                    kSecAttrAccessControl as String: access,
                ],
            ]

            var error: Unmanaged<CFError>?
            guard let privateKey = SecKeyCreateRandomKey(attributes as CFDictionary, &error) else {
                throw KeyError.generationFailed(status: errSecParam)
            }
            return try handle(from: privateKey, backing: .secureEnclave)
        }

        // MARK: - Software fallback (simulator / no enclave)

        private static func generateSoftware() throws -> Handle {
            let privateKey = P256.Signing.PrivateKey()
            // CryptoKit exposes the SPKI DER directly as `derRepresentation`.
            let spki = privateKey.publicKey.derRepresentation
            return Handle(
                spkiBase64: spki.base64EncodedString(),
                backing: .software)
        }

        // MARK: - SPKI extraction from a SecKey

        private static func handle(from privateKey: SecKey, backing: Backing) throws -> Handle {
            guard let publicKey = SecKeyCopyPublicKey(privateKey) else {
                throw KeyError.publicKeyUnavailable
            }
            guard let algorithm = SPKIEncoder.algorithm(of: publicKey) else {
                throw KeyError.spkiEncodingFailed
            }
            var error: Unmanaged<CFError>?
            guard let raw = SecKeyCopyExternalRepresentation(publicKey, &error) as Data?,
                let spki = SPKIEncoder.spkiDER(rawKey: raw, algorithm: algorithm)
            else {
                throw KeyError.spkiEncodingFailed
            }
            return Handle(spkiBase64: spki.base64EncodedString(), backing: backing)
        }
    }
#endif
