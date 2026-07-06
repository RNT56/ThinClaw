import SwiftUI

/// The ThinClaw watch app.
///
/// When WatchConnectivity is available (the real watchOS target), the surface is
/// driven **live**: a ``WatchSessionDelegate`` activates the `WCSession`,
/// receives the phone-provisioned companion credential (stored in the watch
/// keychain) and mirrored snapshots (written to the watch App Group), and a
/// ``RouterGatewayProxy`` routes approve/deny/quick-ask over relay → direct →
/// queue with the watch's OWN token (docs/MOBILE_SECURITY.md D-K4). The read-only
/// ``MirroredSnapshotProxy`` remains the fallback for build targets without
/// WatchConnectivity (kept so the surface still compiles under plain macOS).
@main
struct ThinClawWatchApp: App {
    #if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
        @State private var live = LiveWatchSurface()

        var body: some Scene {
            WindowGroup {
                WatchRootView(store: live.store)
                    .task { live.activate() }
            }
        }
    #else
        @State private var store = WatchStore(proxy: MirroredSnapshotProxy())

        var body: some Scene {
            WindowGroup {
                WatchRootView(store: store)
            }
        }
    #endif
}

#if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
    /// Owns the live watch surface graph: the `WCSession` delegate, the
    /// router-backed proxy, and the store — wired so a fresh snapshot mirror
    /// refreshes the store and a fresh provisioning updates the router token.
    @MainActor
    @Observable
    final class LiveWatchSurface {
        let store: WatchStore
        private let delegate: WatchSessionDelegate

        init() {
            let delegate = WatchSessionDelegate()
            let proxy = RouterGatewayProxy(delegate: delegate)
            let store = WatchStore(proxy: proxy)
            self.delegate = delegate
            self.store = store
            // Re-render the surface whenever a fresh mirror lands over
            // WatchConnectivity, without polling.
            delegate.onMirror = { [weak store] in
                Task { await store?.refresh() }
            }
        }

        /// Activate the `WCSession` (idempotent). Called on first appearance.
        func activate() {
            delegate.activate()
        }
    }
#endif
