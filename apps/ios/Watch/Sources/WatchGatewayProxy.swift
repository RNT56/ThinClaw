import Foundation
import ThinClawSnapshotKit
import ThinClawWatchBridge

/// UI-facing contract the watch app codes against for every gateway action.
///
/// The **concrete** proxy is provided by `ThinClawWatchBridge` (the relay /
/// direct-HTTP transport that attaches the watch's OWN reduced-scope token —
/// docs/MOBILE_SECURITY.md D-K4). This protocol is the thin seam between that
/// transport and the SwiftUI surface here: the views and view models depend
/// only on this shape, so the app compiles and previews without the transport
/// wired, and the bridge's proxy conforms to it once it lands.
///
/// Every call is relay-first: `currentRoute()` reports whether the request
/// will go through the paired iPhone (`.relay`), straight to a reachable
/// gateway (`.direct`), or be queued until reachable (`.queued`). Decisions
/// carry the watch token inside the relay envelope; the phone forwards it
/// opaquely and never uses it as its own credential.
@MainActor
public protocol WatchGatewayProxy: AnyObject, Sendable {
    /// The transport route the next request would take, for an honest badge.
    func currentRoute() -> WatchRoute

    /// Approve or deny a pending tool request. `action` is `"approve"` or
    /// `"deny"`. The watch may only *approve* low-risk entries (D-K3/D-K4);
    /// the low-risk gate is enforced in the UI here and re-enforced
    /// server-side. `deny` is always permitted.
    func approve(id: String, action: String) async -> WatchRelayResponse

    /// Send a dictated quick prompt. The answer arrives later as a push or a
    /// refreshed snapshot; this call only reports acceptance / queueing.
    func quickAsk(prompt: String) async -> WatchRelayResponse

    /// Pull the freshest mirrored snapshot bundle (status + pending approvals),
    /// e.g. from the watch App Group after a relay push. Returns `nil` when no
    /// snapshot has been mirrored yet.
    func refreshSnapshot() async -> WatchSnapshotBundle?
}

/// The glanceable state the watch renders: the agent status projection plus
/// the pending-approvals projection, mirrored from the phone over
/// WatchConnectivity (or read directly after a direct-HTTP refresh).
public struct WatchSnapshotBundle: Sendable, Equatable {
    public var status: AgentStatusSnapshot?
    public var approvals: PendingApprovalsSnapshot?

    public init(
        status: AgentStatusSnapshot? = nil,
        approvals: PendingApprovalsSnapshot? = nil
    ) {
        self.status = status
        self.approvals = approvals
    }
}
