import Foundation

/// A coarse, glanceable reachability summary for the settings connection row,
/// derived from the live ``ConnectionState`` stream — **not** from any
/// `/api/gateway/status` call (the phone's device token cannot hit that admin
/// endpoint). Connection health comes only from the client's own stream state.
public enum GatewayReachability: Hashable, Sendable {
    /// The event stream is connected and heartbeating.
    case reachable
    /// A connection is in progress or retrying — reachable-but-not-yet-live.
    case degraded
    /// No live connection (idle, or gave up).
    case offline

    /// Collapse a live ``ConnectionState`` into the coarse settings summary.
    public init(_ state: ConnectionState) {
        switch state {
        case .connected:
            self = .reachable
        case .connecting, .reconnecting:
            self = .degraded
        case .idle, .failed:
            self = .offline
        }
    }

    /// A short operator-facing label.
    public var label: String {
        switch self {
        case .reachable: return "Reachable"
        case .degraded: return "Connecting…"
        case .offline: return "Offline"
        }
    }
}

/// The paired-gateway identity + connection summary shown on the settings
/// connection row. The identity fields come from the Keychain
/// ``ThinClawAuth.DeviceCredential`` captured at pairing (name, instance id,
/// URL, pin); the live ``reachability`` comes from the ``GatewaySession`` stream
/// state. The URL and pin are the biometric-gated (D-K3) reveal — they identify
/// the operator's gateway address, so ``revealedDetail`` is only populated after
/// a successful Face ID prompt.
public struct GatewayConnectionInfo: Hashable, Sendable {
    /// Gateway's human-readable name at pairing time.
    public var gatewayName: String
    /// Gateway installation id (`instance id`) this credential belongs to.
    public var instanceID: String
    /// Live reachability from the client's stream state.
    public var reachability: GatewayReachability
    /// The sensitive connection detail — gateway base URL + pinned TLS
    /// fingerprint — populated **only** after the biometric gate (D-K3). `nil`
    /// until revealed.
    public var revealedDetail: RevealedDetail?

    public init(
        gatewayName: String,
        instanceID: String,
        reachability: GatewayReachability = .offline,
        revealedDetail: RevealedDetail? = nil
    ) {
        self.gatewayName = gatewayName
        self.instanceID = instanceID
        self.reachability = reachability
        self.revealedDetail = revealedDetail
    }

    /// The Face-ID-gated connection detail (D-K3): the gateway URL and the pinned
    /// certificate fingerprint. Held separately so the store never surfaces it
    /// without a passing biometric result.
    public struct RevealedDetail: Hashable, Sendable {
        public var gatewayURL: String
        public var pinnedFingerprint: String?

        public init(gatewayURL: String, pinnedFingerprint: String?) {
            self.gatewayURL = gatewayURL
            self.pinnedFingerprint = pinnedFingerprint
        }
    }
}
