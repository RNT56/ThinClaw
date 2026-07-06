import Foundation

/// The route the watch chooses for a gateway RPC (docs/MOBILE_APP.md watch
/// section; docs/MOBILE_SECURITY.md D-K4).
///
/// Relay-first: WatchConnectivity through the paired phone is the primary
/// transport because there is no Tailscale on watchOS, so the phone is the only
/// reliable path to a tailnet gateway. Direct HTTP is a fallback used **only**
/// when the gateway is itself reachable (LAN / public HTTPS, pinned). When
/// neither is available the RPC is queued and the UI honestly shows
/// "pending sync".
public enum WatchRoute: Sendable, Hashable {
    case relay
    case direct
    case queued
}

/// The reachability facts the selector reasons over. Kept as a plain value so
/// the whole decision is a pure function, unit-tested on macOS without any
/// WatchConnectivity or networking.
public struct WatchReachability: Sendable, Equatable {
    /// `WCSession` is activated *and* the counterpart phone app is reachable
    /// right now (foreground, in range). Only then can a `sendMessage` relay
    /// get a reply within the interactive deadline.
    public var relayReachable: Bool
    /// The watch holds a companion credential with at least one base URL the
    /// D-X2 policy permits for a *direct* connection (pinned TLS, or tailnet —
    /// though the watch has no tailnet, so in practice pinned LAN/public HTTPS).
    public var directReachable: Bool

    public init(relayReachable: Bool, directReachable: Bool) {
        self.relayReachable = relayReachable
        self.directReachable = directReachable
    }
}

/// Pure route-decision policy for the watch. Separated from the WatchConnectivity
/// and URLSession machinery so relay-vs-direct-vs-queue selection — and the
/// fall-through order after a timeout — is exhaustively testable on a Mac host.
public enum WatchRouteSelector {
    /// The order of routes to try for a fresh RPC, most-preferred first.
    ///
    /// Relay is always first when reachable (it is the credential-safe,
    /// tailnet-capable path). Direct follows when the gateway is directly
    /// reachable. `queued` is the always-present terminal fallback so the
    /// caller can persist the RPC and surface "pending sync" rather than
    /// failing the user's tap.
    public static func routeOrder(for reachability: WatchReachability) -> [WatchRoute] {
        var order: [WatchRoute] = []
        if reachability.relayReachable { order.append(.relay) }
        if reachability.directReachable { order.append(.direct) }
        order.append(.queued)
        return order
    }

    /// The single route to attempt first.
    public static func primaryRoute(for reachability: WatchReachability) -> WatchRoute {
        routeOrder(for: reachability).first ?? .queued
    }

    /// Given the route that just failed (timeout or transport error) and the
    /// current reachability, the next route to try — or `nil` when only the
    /// terminal `queued` fallback remains (the caller then queues).
    ///
    /// This encodes the D-K4 fall-through: relay → direct → queue. A route is
    /// never retried, and `queued` is terminal.
    public static func nextRoute(
        after failed: WatchRoute,
        reachability: WatchReachability
    ) -> WatchRoute? {
        let order = routeOrder(for: reachability)
        guard let idx = order.firstIndex(of: failed), idx + 1 < order.count else {
            return nil
        }
        let next = order[idx + 1]
        return next == .queued ? nil : next
    }
}

/// Interactive-approval latency target from the brief: an approval round-trip
/// should complete in under this budget, and a route that does not answer
/// within it falls through (relay → direct → queue).
public enum WatchRelayTiming {
    /// Per-route deadline for an approval round-trip (< 5s target, split so the
    /// relay attempt plus a direct attempt still fit the overall budget).
    public static let routeDeadline: TimeInterval = 2.0
    /// Overall interactive budget for an approval before the UI must show a
    /// non-committal state.
    public static let approvalBudget: TimeInterval = 5.0
}
