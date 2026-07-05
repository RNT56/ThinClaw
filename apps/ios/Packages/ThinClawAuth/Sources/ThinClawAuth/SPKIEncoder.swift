#if canImport(Security)
    import Foundation
    import Security

    /// Rebuilds a SubjectPublicKeyInfo (SPKI) DER blob from the *raw* public key
    /// bytes that `SecKeyCopyExternalRepresentation` returns.
    ///
    /// `SecTrustCopyKey` + `SecKeyCopyExternalRepresentation` give the bare key,
    /// not the SPKI wrapper the gateway hashes (docs/MOBILE_SECURITY.md pins the
    /// SHA-256 of the *SPKI* DER). For the algorithms the gateway's rcgen
    /// self-signed cert uses (P-256 today; RSA left in for public-chain certs),
    /// the SPKI wrapper is a fixed ASN.1 prefix over those raw bytes, so we can
    /// reconstruct it without a full ASN.1 encoder.
    enum SPKIEncoder {
        /// The public-key algorithm of a server leaf, as reported by
        /// `SecKeyCopyAttributes`.
        enum KeyAlgorithm {
            case ecP256
            case ecP384
            case ecP521
            case rsa(sizeInBits: Int)
        }

        /// Wrap raw external-representation key bytes in their SPKI DER.
        ///
        /// - Returns: the DER-encoded SubjectPublicKeyInfo, or `nil` for an
        ///   algorithm/size we don't have a fixed prefix for. When a pin is
        ///   configured this fails **closed**: `PinnedSessionDelegate` maps a
        ///   `nil` SPKI to `.unextractable` and *cancels* the TLS challenge
        ///   (never falling through to standard chain validation), because a
        ///   pin we cannot verify must not be trusted.
        static func spkiDER(rawKey: Data, algorithm: KeyAlgorithm) -> Data? {
            switch algorithm {
            case .ecP256:
                // SEQ { SEQ { OID ecPublicKey, OID prime256v1 }, BIT STRING }
                guard rawKey.count == 65 else { return nil }
                return prefixEC(
                    curveOID: [0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07],
                    rawKey: rawKey)
            case .ecP384:
                guard rawKey.count == 97 else { return nil }
                return prefixEC(
                    curveOID: [0x2B, 0x81, 0x04, 0x00, 0x22],
                    rawKey: rawKey)
            case .ecP521:
                guard rawKey.count == 133 else { return nil }
                return prefixEC(
                    curveOID: [0x2B, 0x81, 0x04, 0x00, 0x23],
                    rawKey: rawKey)
            case .rsa:
                // rawKey is a PKCS#1 RSAPublicKey (SEQ { modulus, exponent }).
                // SPKI = SEQ { SEQ { OID rsaEncryption, NULL }, BIT STRING(rawKey) }.
                return wrapRSA(pkcs1: rawKey)
            }
        }

        /// Build an EC SPKI: the AlgorithmIdentifier is
        /// `SEQ { OID id-ecPublicKey, <curveOID> }`, and the subjectPublicKey
        /// BIT STRING is the uncompressed point (`rawKey`, starting `0x04`).
        private static func prefixEC(curveOID: [UInt8], rawKey: Data) -> Data {
            let ecPublicKeyOID: [UInt8] = [0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01]

            let algorithmIdentifier =
                der(tag: 0x30, contents: derOID(ecPublicKeyOID) + derOID(curveOID))
            let subjectPublicKey = der(tag: 0x03, contents: [0x00] + Array(rawKey))
            return Data(der(tag: 0x30, contents: algorithmIdentifier + subjectPublicKey))
        }

        private static func wrapRSA(pkcs1: Data) -> Data {
            let rsaEncryptionOID: [UInt8] = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x01]
            let algorithmIdentifier =
                der(tag: 0x30, contents: derOID(rsaEncryptionOID) + [0x05, 0x00])  // NULL params
            let subjectPublicKey = der(tag: 0x03, contents: [0x00] + Array(pkcs1))
            return Data(der(tag: 0x30, contents: algorithmIdentifier + subjectPublicKey))
        }

        // MARK: - Tiny DER helpers

        private static func derOID(_ body: [UInt8]) -> [UInt8] {
            der(tag: 0x06, contents: body)
        }

        /// TLV with a definite-length encoding.
        private static func der(tag: UInt8, contents: [UInt8]) -> [UInt8] {
            [tag] + derLength(contents.count) + contents
        }

        private static func derLength(_ length: Int) -> [UInt8] {
            if length < 0x80 {
                return [UInt8(length)]
            }
            var value = length
            var bytes: [UInt8] = []
            while value > 0 {
                bytes.insert(UInt8(value & 0xFF), at: 0)
                value >>= 8
            }
            return [0x80 | UInt8(bytes.count)] + bytes
        }

        /// Read the public-key algorithm off a `SecKey`.
        static func algorithm(of key: SecKey) -> KeyAlgorithm? {
            guard let attributes = SecKeyCopyAttributes(key) as? [CFString: Any],
                let keyType = attributes[kSecAttrKeyType] as? String,
                let sizeInBits = attributes[kSecAttrKeySizeInBits] as? Int
            else { return nil }

            if keyType == (kSecAttrKeyTypeECSECPrimeRandom as String)
                || keyType == (kSecAttrKeyTypeEC as String)
            {
                switch sizeInBits {
                case 256: return .ecP256
                case 384: return .ecP384
                case 521: return .ecP521
                default: return nil
                }
            }
            if keyType == (kSecAttrKeyTypeRSA as String) {
                return .rsa(sizeInBits: sizeInBits)
            }
            return nil
        }
    }
#endif
