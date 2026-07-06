#if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAuth
    import ThinClawSnapshotKit
    import ThinClawWatchBridge
    import WatchConnectivity

    /// The **live** ``WatchGatewayProxy`` backing the watch surface (replacing the
    /// read-only ``MirroredSnapshotProxy``).
    ///
    /// Writes (approve/deny, quick-ask) go through a ``WatchGatewayRouter`` over
    /// the live relay → direct → queue transports (docs/MOBILE_SECURITY.md D-K4):
    /// every relayed request carries the **watch's own** reduced-scope token, and
    /// the phone forwards it opaquely. Reads (the glanceable snapshot) still come
    /// from the App Group mirror the phone pushes — the router handles only the
    /// write RPCs, so the mirror stays the single snapshot source.
    ///
    /// The proxy re-reads the watch's credential from the session delegate on
    /// every call so a mid-session provisioning (or re-provisioning) is picked up
    /// without rebuilding the surface: the token is threaded fresh into each
    /// router, never captured stale.
    @MainActor
    final class RouterGatewayProxy: WatchGatewayProxy {
        private let delegate: WatchSessionDelegate
        private let session: WCSession
        private let snapshotStore: SnapshotStore?

        init(
            delegate: WatchSessionDelegate,
            session: WCSession = .default,
            snapshotStore: SnapshotStore? = SnapshotStore(
                appGroupID: MirroredSnapshotProxy.watchAppGroupID)
        ) {
            self.delegate = delegate
            self.session = session
            self.snapshotStore = snapshotStore
        }

        /// Build a router over the live transports with the current watch token.
        private func makeRouter() -> WatchGatewayRouter {
            let credential = delegate.credential
            return WatchGatewayRouter(
                relay: LiveWatchRelayTransport(session: session),
                direct: LiveWatchDirectTransport(credential: credential),
                queue: LiveWatchQueueTransport(session: session),
                watchToken: credential?.watchToken)
        }

        /// The route the next *write* would take, for an honest badge: relay when
        /// the phone is reachable and we hold a token, else direct when the
        /// gateway is directly reachable, else queued.
        func currentRoute() -> WatchRoute {
            let credential = delegate.credential
            let reachability = WatchReachability(
                relayReachable: delegate.isReachable && credential != nil,
                directReachable: credential?.deviceCredential.preferredBaseURL != nil)
            return WatchRouteSelector.primaryRoute(for: reachability)
        }

        func approve(id: String, action: String) async -> WatchRelayResponse {
            let outcome = await makeRouter().approve(
                requestID: id, threadID: nil, action: action)
            return Self.response(for: outcome)
        }

        func quickAsk(prompt: String) async -> WatchRelayResponse {
            let outcome = await makeRouter().quickAsk(prompt, threadID: nil)
            return Self.response(for: outcome)
        }

        /// Serve the freshest mirrored bundle from the watch App Group. The
        /// router's `refreshSnapshot` only nudges the phone; the phone answers by
        /// pushing a new mirror, which lands via the session delegate — so the
        /// read here is always from the mirror the phone last wrote.
        func refreshSnapshot() async -> WatchSnapshotBundle? {
            // Best-effort nudge for a fresh push; ignore the outcome (the mirror
            // is authoritative and a later push supersedes anything queued).
            _ = await makeRouter().refreshSnapshot()
            guard let snapshotStore else { return nil }
            let status = try? snapshotStore.load(AgentStatusSnapshot.self)
            let approvals = try? snapshotStore.load(PendingApprovalsSnapshot.self)
            if status == nil && approvals == nil { return nil }
            return WatchSnapshotBundle(
                status: status ?? nil, approvals: approvals ?? nil)
        }

        /// Map a router outcome to the UI-facing response the store renders.
        private static func response(for outcome: WatchRouteOutcome) -> WatchRelayResponse {
            switch outcome {
            case let .completed(_, response):
                return response
            case .queued:
                // A queued write was accepted for later delivery; the store
                // reports "will send when reachable" from its own route snapshot.
                return .accepted
            case .pendingSync:
                return .failed(reason: "pending-sync")
            }
        }
    }
#endif
