import Foundation
import ThinClawSnapshotKit
import ThinClawWatchBridge

/// A read-only ``WatchGatewayProxy`` that serves the mirrored snapshot bundle
/// from the watch's own App Group and **queues** every write.
///
/// This is the honest default until `ThinClawWatchBridge` lands its real
/// relay/direct proxy (the other M4 half, `WatchRelayHost`/route selection):
/// the watch can *render* whatever the phone last mirrored into the App Group,
/// but it cannot yet send a decision or a prompt, so every action reports
/// `.queued` and every route reads `.queued`. Swapping in the bridge proxy at
/// `WatchApp` construction is the only change needed — the UI already codes
/// against the protocol.
///
/// The App Group `group.com.thinclaw.shared.watch` is the watch-local mirror
/// container (distinct from the phone's `group.com.thinclaw.shared`); the
/// bridge writes into it over WatchConnectivity.
@MainActor
final class MirroredSnapshotProxy: WatchGatewayProxy {
    /// The watch-side App Group that mirrors phone snapshots. Matches
    /// `Watch/Watch.entitlements` and `WatchWidgets/WatchWidgets.entitlements`.
    ///
    /// `nonisolated` so it is usable from the `nonisolated` default-argument
    /// context in ``init(store:)`` under the Swift 6 language mode.
    nonisolated static let watchAppGroupID = "group.com.thinclaw.shared.watch"

    private let store: SnapshotStore?

    init(store: SnapshotStore? = SnapshotStore(appGroupID: watchAppGroupID)) {
        self.store = store
    }

    func currentRoute() -> WatchRoute {
        // No transport wired yet: everything is pending sync until the bridge
        // proxy replaces this one.
        .queued
    }

    func approve(id: String, action: String) async -> WatchRelayResponse {
        .failed(reason: "Watch relay not connected yet")
    }

    func quickAsk(prompt: String) async -> WatchRelayResponse {
        .failed(reason: "Watch relay not connected yet")
    }

    func refreshSnapshot() async -> WatchSnapshotBundle? {
        guard let store else { return nil }
        let status = try? store.load(AgentStatusSnapshot.self)
        let approvals = try? store.load(PendingApprovalsSnapshot.self)
        if status == nil && approvals == nil { return nil }
        return WatchSnapshotBundle(status: status ?? nil, approvals: approvals ?? nil)
    }
}
