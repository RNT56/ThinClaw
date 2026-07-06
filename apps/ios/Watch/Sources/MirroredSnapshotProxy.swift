import Foundation
import ThinClawSnapshotKit
import ThinClawWatchBridge

/// A read-only ``WatchGatewayProxy`` that serves the mirrored snapshot bundle
/// from the watch's own App Group and **queues** every write.
///
/// This is now the **fallback** proxy for build targets without
/// WatchConnectivity (e.g. a plain macOS host): the live surface uses
/// ``RouterGatewayProxy`` over a real ``WatchGatewayRouter`` instead
/// (``WatchApp``). It renders whatever the phone last mirrored into the App
/// Group, but it cannot send a decision or a prompt, so every action reports
/// `.queued` and every route reads `.queued`.
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
