import Foundation

/// Client-side constants for ThinClaw device tokens.
public enum DeviceToken {
    /// Every gateway-issued device token starts with this prefix, so logs
    /// and secret scanners can recognize it (`tcd_` = "ThinClaw device").
    public static let prefix = "tcd_"

    /// Structural sanity check (prefix + non-empty body). NOT a validity
    /// check — only the gateway can decide that.
    public static func isWellFormed(_ token: String) -> Bool {
        token.hasPrefix(prefix) && token.count > prefix.count
            && !token.dropFirst(prefix.count).contains(where: \.isWhitespace)
    }

    /// Redacted form safe for logs: `tcd_ab…` (prefix + 2 chars).
    public static func redacted(_ token: String) -> String {
        guard isWellFormed(token) else { return "<malformed-token>" }
        return String(token.prefix(prefix.count + 2)) + "…"
    }
}

/// Well-known keychain keys for ThinClaw secrets.
public enum KeychainKey {
    /// JSON-encoded ``DeviceCredential``.
    public static let deviceCredential = "device-credential"
}

/// The durable result of a successful pairing: everything the device needs
/// to talk to its gateway. Stored as JSON in the keychain under
/// ``KeychainKey/deviceCredential``.
public struct DeviceCredential: Codable, Sendable, Equatable {
    /// Gateway installation this credential belongs to.
    public var installationID: String
    /// Bearer token (`tcd_…`) presented on every gateway request.
    public var deviceToken: String
    /// Gateway base URLs in preference order (from pairing; may be
    /// re-ordered later by reachability probing).
    public var gatewayURLs: [URL]
    /// Optional pinned TLS certificate fingerprint from pairing.
    public var serverFingerprint: String?
    /// Gateway's human-readable name at pairing time.
    public var gatewayName: String
    public var pairedAt: Date

    public init(
        installationID: String,
        deviceToken: String,
        gatewayURLs: [URL],
        serverFingerprint: String? = nil,
        gatewayName: String,
        pairedAt: Date
    ) {
        self.installationID = installationID
        self.deviceToken = deviceToken
        self.gatewayURLs = gatewayURLs
        self.serverFingerprint = serverFingerprint
        self.gatewayName = gatewayName
        self.pairedAt = pairedAt
    }
}

extension DeviceCredential {
    /// Load the stored credential, if the device is paired.
    public static func load(from keychain: some KeychainStoring) throws -> DeviceCredential? {
        try keychain.codable(DeviceCredential.self, for: KeychainKey.deviceCredential)
    }

    /// Persist this credential as the device's pairing.
    public func save(to keychain: some KeychainStoring) throws {
        try keychain.setCodable(self, for: KeychainKey.deviceCredential)
    }

    /// Forget the pairing (sign out / unpair).
    public static func erase(from keychain: some KeychainStoring) throws {
        try keychain.removeSecret(for: KeychainKey.deviceCredential)
    }
}
