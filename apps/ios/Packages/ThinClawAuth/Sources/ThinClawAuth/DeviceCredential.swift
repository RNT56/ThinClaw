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
    /// Server-assigned device id (`device_id` from pair/complete), needed to
    /// address this device for self-revoke on unpair. Optional for forward
    /// compatibility with credentials stored before it was captured.
    public var deviceID: String?
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
        deviceID: String? = nil,
        deviceToken: String,
        gatewayURLs: [URL],
        serverFingerprint: String? = nil,
        gatewayName: String,
        pairedAt: Date
    ) {
        self.installationID = installationID
        self.deviceID = deviceID
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

    /// The first stored gateway URL that the D-X2 connection policy permits for
    /// this credential (pinned ⇒ TLS anywhere or tailnet HTTP; unpinned ⇒
    /// tailnet HTTP only), or `nil` if none qualify.
    ///
    /// Callers building a live session **must** use this rather than
    /// `gatewayURLs.first`: `PinnedSessionDelegate` only enforces the pin on a
    /// TLS server-trust challenge, so a plaintext `http://` LAN URL would never
    /// trip it and would carry the `tcd_` token in the clear. Selecting a
    /// policy-allowed URL here closes that gap even if a mixed-scheme list was
    /// somehow persisted (e.g. by an older build before pairing filtered it).
    public var preferredBaseURL: URL? {
        let hasPin = serverFingerprint != nil
        #if DEBUG
            let allowLoopbackHTTP = true
        #else
            let allowLoopbackHTTP = false
        #endif
        return gatewayURLs.first { url in
            switch ConnectionPolicy.evaluate(
                url: url, hasPin: hasPin, allowLoopbackHTTP: allowLoopbackHTTP)
            {
            case .allowedSecure, .allowedInsecure: return true
            case .refused: return false
            }
        }
    }
}
