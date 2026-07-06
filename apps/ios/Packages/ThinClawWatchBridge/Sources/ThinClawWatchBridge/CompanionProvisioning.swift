import Foundation

/// The credential the phone provisions onto the watch over
/// `WCSession.updateApplicationContext` (docs/MOBILE_SECURITY.md, D-K4).
///
/// This is the watch's **own** reduced-scope companion credential, minted by
/// the paired phone at `POST /api/devices/me/companions` and delivered as
/// non-secret-on-the-wire application context (WatchConnectivity is a local,
/// paired, encrypted channel). The watch persists the token in its **own**
/// keychain (`AfterFirstUnlockThisDeviceOnly`) — it is never the phone's token
/// and is independently revocable (cascade-revoked with its parent).
///
/// The SPKI pin + gateway URLs + instance id ride along so the watch's *direct*
/// fallback route can pin exactly like the phone (D-X2) instead of trusting a
/// re-discovered endpoint.
public struct CompanionProvisioning: Codable, Sendable, Equatable {
    /// Provisioning payload schema version.
    public var version: Int
    /// The watch's own `tcd_…` companion token. Scoped `chat` + `approvals`
    /// only (low-risk approvals enforced server-side by device class).
    public var watchToken: String
    /// Server-assigned companion `device_id` — lets the watch report which
    /// credential it holds and lets the phone address it for revoke.
    public var companionDeviceID: String
    /// The parent (phone) device id that minted this companion, echoed so the
    /// watch/phone can reason about the cascade relationship.
    public var parentDeviceID: String
    /// Gateway base URLs in preference order (for the direct fallback route).
    public var gatewayURLs: [URL]
    /// Pinned TLS SPKI fingerprint (base64url sha256), when the gateway serves
    /// pinned TLS. `nil` only in an explicit `vpn-http` deployment.
    public var serverFingerprint: String?
    /// Stable gateway instance id from pairing (evil-twin defense, D-X3).
    public var instanceID: String
    /// Gateway installation id this credential belongs to.
    public var installationID: String

    public static let currentVersion = 1

    public init(
        version: Int = CompanionProvisioning.currentVersion,
        watchToken: String,
        companionDeviceID: String,
        parentDeviceID: String,
        gatewayURLs: [URL],
        serverFingerprint: String?,
        instanceID: String,
        installationID: String
    ) {
        self.version = version
        self.watchToken = watchToken
        self.companionDeviceID = companionDeviceID
        self.parentDeviceID = parentDeviceID
        self.gatewayURLs = gatewayURLs
        self.serverFingerprint = serverFingerprint
        self.instanceID = instanceID
        self.installationID = installationID
    }

    // MARK: - Application-context coding

    /// The `updateApplicationContext` key namespace. Keeping the whole payload
    /// under one JSON blob means the context dictionary stays a single stable
    /// property-list value regardless of the payload schema.
    public static let contextKey = "companionProvisioning"

    /// Render for `WCSession.updateApplicationContext([String: Any])`.
    public func applicationContext() throws -> [String: Any] {
        [Self.contextKey: try JSONEncoder().encode(self)]
    }

    /// Decode from a received application context, or `nil` if this context
    /// carries no provisioning payload (contexts are also used for snapshots).
    public static func fromApplicationContext(
        _ context: [String: Any]
    ) throws -> CompanionProvisioning? {
        guard let data = context[contextKey] as? Data else { return nil }
        let payload = try JSONDecoder().decode(CompanionProvisioning.self, from: data)
        guard payload.version == currentVersion else {
            throw WatchRelayError.unsupportedVersion(payload.version)
        }
        return payload
    }
}

/// What the watch reports back to the phone about its credential state, so the
/// phone knows whether it must (re-)provision. Sent over `sendMessage` or as
/// part of the watch's application context.
public struct CompanionCredentialState: Codable, Sendable, Equatable {
    /// Whether the watch currently holds a stored companion credential.
    public var hasCredential: Bool
    /// The companion `device_id` the watch believes it holds, if any — lets
    /// the phone detect a stale credential (mismatched parent, revoked id).
    public var companionDeviceID: String?

    public static let contextKey = "companionCredentialState"

    public init(hasCredential: Bool, companionDeviceID: String? = nil) {
        self.hasCredential = hasCredential
        self.companionDeviceID = companionDeviceID
    }

    /// Whether the phone should mint/re-mint a companion for this watch given
    /// the watch's reported state and the phone's own view of what it last
    /// provisioned.
    ///
    /// Re-provision when the watch has no credential, or when the watch holds a
    /// credential whose id does not match what the phone last minted (a stale
    /// or foreign credential — e.g. after the phone re-paired to a new gateway).
    public func needsProvisioning(lastProvisionedDeviceID: String?) -> Bool {
        guard hasCredential else { return true }
        guard let expected = lastProvisionedDeviceID else {
            // Watch claims a credential but the phone has no record of minting
            // one (fresh phone install / re-pair): re-mint to be authoritative.
            return true
        }
        return companionDeviceID != expected
    }
}
