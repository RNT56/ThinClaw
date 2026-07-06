#if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAuth
    import ThinClawSnapshotKit
    import ThinClawWatchBridge
    import WatchConnectivity

    #if canImport(WidgetKit)
        import WidgetKit
    #endif

    /// Watch-side `WCSession` lifecycle host (docs/MOBILE_SECURITY.md D-K4).
    ///
    /// Responsibilities, all of them transport glue over the pure bridge seams:
    ///  1. **Activate** the `WCSession` and, on activation, report the watch's
    ///     current credential state to the phone (via `updateApplicationContext`)
    ///     so the phone can (re-)provision without a round-trip.
    ///  2. **Receive provisioning** — decode a ``CompanionProvisioning`` from the
    ///     phone's application context and persist it as a
    ///     ``WatchCompanionCredential`` in the **watch's own** keychain
    ///     (`AfterFirstUnlockThisDeviceOnly`, D-K2). The token is never the
    ///     phone's and is independently revocable.
    ///  3. **Receive mirrors** — decode the agent-status / pending-approvals
    ///     snapshots the phone pushes and write them to the watch App Group so the
    ///     root view and the complication read live data. Reload the complication
    ///     timeline on every fresh mirror.
    ///
    /// The delegate publishes the loaded credential so ``WatchApp`` can build a
    /// live ``WatchGatewayRouter`` whose relayed requests carry the watch's own
    /// token. All the credential-selection / route logic lives in
    /// ``ThinClawWatchBridge`` (macOS-tested); this class only owns the WCSession
    /// callbacks and keychain/App-Group I/O.
    @MainActor
    @Observable
    final class WatchSessionDelegate: NSObject {
        /// The watch's own companion credential, once provisioned by the phone.
        /// `nil` until the first provisioning context arrives (or is loaded from
        /// the keychain at launch). Observed so the router picks up a fresh token.
        private(set) var credential: WatchCompanionCredential?

        /// Called after a fresh snapshot mirror lands, so the store can refresh
        /// its rendered bundle without polling. Set by ``WatchApp``.
        var onMirror: (@MainActor () -> Void)?

        /// Whether the `WCSession` is activated and the phone is reachable right
        /// now — the relay transport reads this for route selection.
        private(set) var isReachable = false

        private let keychain: any KeychainStoring
        private let snapshotStore: SnapshotStore?
        private let session: WCSession

        init(
            keychain: any KeychainStoring = SharedGatewayConnection.keychain(),
            snapshotStore: SnapshotStore? = SnapshotStore(
                appGroupID: MirroredSnapshotProxy.watchAppGroupID),
            session: WCSession = .default
        ) {
            self.keychain = keychain
            self.snapshotStore = snapshotStore
            self.session = session
            super.init()
            // Load any previously-provisioned credential so a relaunched watch
            // relays immediately, before the phone re-sends its context.
            self.credential = try? WatchCompanionCredential.load(from: keychain)
        }

        /// Set the delegate and activate the session. Idempotent — safe to call
        /// on every foreground. A no-op when WCSession is unsupported.
        func activate() {
            guard WCSession.isSupported() else { return }
            session.delegate = self
            if session.activationState != .activated {
                session.activate()
            }
        }

        // MARK: - Credential-state reporting

        /// Report the watch's current credential state to the phone so it can
        /// decide whether to (re-)provision. Best-effort; merged under its own
        /// key so a concurrent phone→watch context is not what we send.
        private func reportCredentialState() {
            guard session.activationState == .activated else { return }
            let state =
                credential?.reportedState ?? CompanionCredentialState(hasCredential: false)
            guard let data = try? JSONEncoder().encode(state) else { return }
            try? session.updateApplicationContext([
                CompanionCredentialState.contextKey: data
            ])
        }

        // MARK: - Inbound context handling

        /// Persist a received provisioning payload into the watch keychain and
        /// publish the credential. Returns whether a credential was stored.
        @discardableResult
        private func storeProvisioning(_ provisioning: CompanionProvisioning) -> Bool {
            let credential = WatchCompanionCredential(from: provisioning)
            do {
                try credential.save(to: keychain)
                self.credential = credential
                return true
            } catch {
                // Leave the watch unprovisioned; it will re-report `hasCredential:
                // false` and the phone re-mints on the next reachability change.
                return false
            }
        }

        /// Persist a received snapshot mirror into the watch App Group and reload
        /// the complication. Returns whether anything was written.
        @discardableResult
        private func storeMirror(from context: [String: Any]) -> Bool {
            guard let snapshotStore else { return false }
            var wroteAnything = false
            if let status = WatchSnapshotMirror.status(from: context) {
                try? snapshotStore.save(status)
                wroteAnything = true
            }
            if let approvals = WatchSnapshotMirror.approvals(from: context) {
                try? snapshotStore.save(approvals)
                wroteAnything = true
            }
            if wroteAnything {
                onMirror?()
                reloadComplication()
            }
            return wroteAnything
        }

        private func reloadComplication() {
            #if canImport(WidgetKit)
                WidgetCenter.shared.reloadAllTimelines()
            #endif
        }

        // MARK: - Main-actor handlers behind the nonisolated callbacks

        fileprivate func handleActivation(activated: Bool, reachable: Bool) {
            isReachable = activated && reachable
            // Tell the phone what we hold so it provisions/re-provisions.
            if activated { reportCredentialState() }
        }

        fileprivate func handleReachabilityChange(activated: Bool, reachable: Bool) {
            isReachable = activated && reachable
            // A newly-reachable phone can now provision us if we have no token.
            if isReachable, credential == nil { reportCredentialState() }
        }

        fileprivate func handleContext(_ context: [String: Any]) {
            // A context may carry a provisioning payload, a snapshot mirror, or
            // both. Handle each independently.
            if let provisioning = try? CompanionProvisioning.fromApplicationContext(context) {
                storeProvisioning(provisioning)
            }
            storeMirror(from: context)
        }
    }

    // MARK: - WCSessionDelegate

    // `@preconcurrency` conformance: the WatchConnectivity callbacks are invoked
    // on a background queue with non-`Sendable` `WCSession` / `[String: Any]`
    // arguments, so the methods stay `nonisolated` and hop to the main actor
    // (mirroring `WatchRelayHost`). Only the values actually needed cross the hop.
    extension WatchSessionDelegate: @preconcurrency WCSessionDelegate {
        func session(
            _ session: WCSession,
            activationDidCompleteWith activationState: WCSessionActivationState,
            error: (any Error)?
        ) {
            let activated = activationState == .activated
            let reachable = session.isReachable
            Task { @MainActor in
                self.handleActivation(activated: activated, reachable: reachable)
            }
        }

        func sessionReachabilityDidChange(_ session: WCSession) {
            let activated = session.activationState == .activated
            let reachable = session.isReachable
            Task { @MainActor in
                self.handleReachabilityChange(activated: activated, reachable: reachable)
            }
        }

        func session(
            _ session: WCSession,
            didReceiveApplicationContext applicationContext: [String: Any]
        ) {
            Task { @MainActor in
                self.handleContext(applicationContext)
            }
        }

        func session(
            _ session: WCSession,
            didReceiveUserInfo userInfo: [String: Any] = [:]
        ) {
            // The phone also uses the budgeted complication/user-info channel to
            // push a mirror; treat it the same as an application context.
            Task { @MainActor in
                self.storeMirror(from: userInfo)
            }
        }
    }
#endif
