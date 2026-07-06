import SwiftUI

@main
struct ThinClawWatchApp: App {
    /// The gateway proxy backing the whole surface. Defaults to the read-only
    /// ``MirroredSnapshotProxy`` (renders the mirrored App Group bundle, queues
    /// writes) until `ThinClawWatchBridge` provides its relay/direct proxy —
    /// swapping it here is the only wiring change (docs/MOBILE_SECURITY.md
    /// D-K4).
    @State private var store = WatchStore(proxy: MirroredSnapshotProxy())

    var body: some Scene {
        WindowGroup {
            WatchRootView(store: store)
        }
    }
}
