import Foundation

/// RPCs the watch sends through the phone (or directly, when reachable).
/// Payloads are small Codable envelopes; the watch token rides inside so the
/// phone can forward opaquely without ever using it as its own credential.
public enum WatchRelayRequest: Codable, Sendable {
    case approve(requestID: String, threadID: String?, action: String)
    case quickAsk(prompt: String)
    case snapshotRefresh
}

public enum WatchRelayResponse: Codable, Sendable {
    case accepted
    case failed(reason: String)
}

/// Route selection on the watch side (docs/MOBILE_APP.md, watch section):
/// WCSession reachable → relay; else direct gateway with a short probe;
/// else queue and render "pending sync" honestly.
public enum WatchRoute: Sendable, Hashable {
    case relay
    case direct
    case queued
}

#if canImport(WatchConnectivity)
    import WatchConnectivity

    /// iOS-side host: answers watch RPCs by forwarding them to the gateway
    /// session and pushes snapshot updates via `updateApplicationContext`.
    /// Fleshed out at milestone M4.
    @MainActor
    public final class WatchRelayHost: NSObject {
        public override init() {
            super.init()
        }

        public func activate() {
            guard WCSession.isSupported() else { return }
            // M4: WCSession.default.delegate = self; WCSession.default.activate()
        }
    }
#endif
