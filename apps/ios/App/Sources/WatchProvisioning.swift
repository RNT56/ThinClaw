#if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAuth
    import ThinClawSnapshotKit
    import ThinClawWatchBridge
    import WatchConnectivity

    /// App-side owner of the watch companion relay (docs/MOBILE_SECURITY.md D-K4).
    ///
    /// This is the iOS-app hook for milestone M4: it builds and activates a
    /// ``WatchRelayHost`` while the phone is paired, so the paired watch can be
    /// provisioned with its own reduced-scope companion token and can relay
    /// approvals / quick-asks through the phone. On unpair it best-effort
    /// deprovisions the companion (an explicit `DELETE`; the parent-revoke cascade
    /// also covers it) and tears the host down.
    ///
    /// Deliberately thin: all testable logic lives in `ThinClawWatchBridge`
    /// (`WatchRelayResponder`, `CompanionProvisioner`, `WatchGatewayProxy`,
    /// `WatchRouteSelector`). This coordinator only owns the host's lifecycle and
    /// reads the phone's credential from the shared Keychain — the same seam the
    /// widgets/NSE use — so it never reaches into `AppDependencies` internals.
    @MainActor
    final class WatchProvisioning {
        private var host: WatchRelayHost?

        /// Whether a watch is even worth talking to on this device.
        private var isWatchSupported: Bool { WCSession.isSupported() }

        /// Build (if needed) and activate the relay host from the currently
        /// stored phone credential. Safe to call repeatedly (idempotent): a no-op
        /// when unsupported, unpaired, or already active.
        func activateIfPaired() {
            guard isWatchSupported, host == nil else {
                host?.activate()
                return
            }
            guard let credential = SharedGatewayConnection.loadCredential() else { return }
            let host = WatchRelayHost(
                parentCredential: credential,
                // The QR `iid` captured at pairing is the gateway instance id
                // (stored as `installationID`); the watch pins the same identity
                // for its direct route (D-X3).
                instanceID: credential.installationID,
                companionName: Self.companionName)
            self.host = host
            host.activate()
        }

        /// Push the freshest glanceable snapshot to the watch on a significant
        /// agent-state change (status / pending approvals). No-op when no host is
        /// active. Called by the app's snapshot pipeline hook.
        func mirror(
            status: AgentStatusSnapshot,
            approvals: PendingApprovalsSnapshot
        ) {
            host?.pushSnapshot(status: status, approvals: approvals)
        }

        /// Best-effort deprovision + teardown on unpair. Awaits the companion
        /// `DELETE` (bounded — the host swallows failures) before dropping the
        /// host so a still-valid parent token can authenticate the revoke.
        func deprovisionAndTearDown() async {
            await host?.deprovisionCompanion()
            host = nil
        }

        /// Human label for the minted companion, surfaced in the operator's
        /// device list.
        private static let companionName = "Apple Watch"
    }
#endif
