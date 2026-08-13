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

        /// Called after any local deprovision (authenticated wipe, legacy/corrupt
        /// credential migration, or gateway 401/403) so in-memory UI state is
        /// cleared in the same transaction as Keychain and mirrored files.
        var onDeprovision: (@MainActor () -> Void)?

        /// Whether the `WCSession` is activated and the phone is reachable right
        /// now — the relay transport reads this for route selection.
        private(set) var isReachable = false

        private let keychain: any KeychainStoring
        private let snapshotStore: SnapshotStore?
        private let session: WCSession
        private let tombstoneDefaults: UserDefaults
        private var tombstone: WatchWipeTombstone?

        private static let tombstoneKey = "watch-wipe-tombstone-v1"
        private static let locallyRevokedCompanionKey = "watch-locally-revoked-companion-v1"
        private static let deprovisionedKey = "watch-is-deprovisioned-v1"

        init(
            keychain: any KeychainStoring = SharedGatewayConnection.keychain(),
            snapshotStore: SnapshotStore? = SnapshotStore(
                appGroupID: MirroredSnapshotProxy.watchAppGroupID),
            tombstoneDefaults: UserDefaults =
                UserDefaults(suiteName: MirroredSnapshotProxy.watchAppGroupID) ?? .standard,
            session: WCSession = .default
        ) {
            self.keychain = keychain
            self.snapshotStore = snapshotStore
            self.tombstoneDefaults = tombstoneDefaults
            self.session = session
            let loadedTombstone = Self.loadTombstone(from: tombstoneDefaults)
            self.tombstone = loadedTombstone

            var loadedCredential: WatchCompanionCredential?
            var requiresFailClosedMigration = false
            do {
                loadedCredential = try WatchCompanionCredential.load(from: keychain)
            } catch {
                // v1/corrupt credentials have no control generation/key. Never
                // leave their bearer or old mirrors resident after upgrading.
                requiresFailClosedMigration = true
            }
            if let loadedCredential, let loadedTombstone,
                loadedCredential.provisioningGeneration <= loadedTombstone.generation
            {
                requiresFailClosedMigration = true
            }
            if let loadedCredential,
                tombstoneDefaults.string(forKey: Self.locallyRevokedCompanionKey)
                    == loadedCredential.companionDeviceID
            {
                // A prior Keychain deletion may have failed. The durable local
                // revocation marker still prevents that bearer from resurrecting
                // on launch, and the wipe below retries deletion.
                requiresFailClosedMigration = true
            }
            self.credential = requiresFailClosedMigration ? nil : loadedCredential
            super.init()

            if requiresFailClosedMigration {
                performLocalWipe(notify: false)
            } else if loadedCredential != nil {
                tombstoneDefaults.set(false, forKey: Self.deprovisionedKey)
            }
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
            guard
                WatchWipeReceiver.acceptsProvisioning(
                    generation: provisioning.provisioningGeneration,
                    currentGeneration: credential?.provisioningGeneration,
                    tombstone: tombstone)
            else { return false }
            if tombstoneDefaults.string(forKey: Self.locallyRevokedCompanionKey)
                == provisioning.companionDeviceID
            {
                return false
            }
            let credential = WatchCompanionCredential(from: provisioning)
            do {
                try credential.save(to: keychain)
                self.credential = credential
                tombstoneDefaults.removeObject(forKey: Self.locallyRevokedCompanionKey)
                tombstoneDefaults.set(false, forKey: Self.deprovisionedKey)
                _ = tombstoneDefaults.synchronize()
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
            guard let snapshotStore, let credential,
                WatchSnapshotMirror.provisioningGeneration(from: context)
                    == credential.provisioningGeneration,
                WatchWipeReceiver.acceptsProvisioning(
                    generation: credential.provisioningGeneration,
                    currentGeneration: credential.provisioningGeneration,
                    tombstone: tombstone)
            else { return false }
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
            if let command = try? WatchWipeCommand.fromPayload(context) {
                if let acknowledgement = processWipe(command),
                    let payload = try? acknowledgement.messagePayload()
                {
                    session.transferUserInfo(payload)
                }
                return
            }
            // A context may carry a provisioning payload, a snapshot mirror, or
            // both. Handle each independently.
            if let provisioning = try? CompanionProvisioning.fromApplicationContext(context) {
                storeProvisioning(provisioning)
            }
            storeMirror(from: context)
        }

        fileprivate func processWipe(
            _ command: WatchWipeCommand
        ) -> WatchWipeAcknowledgement? {
            switch WatchWipeReceiver.evaluate(
                command,
                credentialMaterial: credential?.controlMaterial,
                tombstone: tombstone)
            {
            case .reject:
                return nil
            case .acknowledge(let acknowledgement):
                return acknowledgement
            case .apply(let tombstone):
                // Persist the replay marker before erasing the HMAC key. If that
                // persistence fails, still wipe for privacy but withhold the ack
                // so the phone keeps retrying rather than assuming durability.
                guard persist(tombstone) else {
                    performLocalWipe(notify: true)
                    return nil
                }
                self.tombstone = tombstone
                performLocalWipe(notify: true)
                return tombstone.acknowledgement
            }
        }

        /// A direct or relayed 401/403 proves the current Watch bearer is stale.
        /// Clear it and its mirrors immediately; remember its companion id so an
        /// out-of-order copy of the same provisioning payload cannot restore it.
        func deprovisionRevokedCredential() {
            if let companionDeviceID = credential?.companionDeviceID {
                tombstoneDefaults.set(
                    companionDeviceID,
                    forKey: Self.locallyRevokedCompanionKey)
                _ = tombstoneDefaults.synchronize()
            }
            performLocalWipe(notify: true)
            reportCredentialState()
        }

        private func performLocalWipe(notify: Bool) {
            // Extensions cannot inspect the Watch Keychain. Publish a privacy
            // gate in their shared container before attempting file deletion so
            // a complication never renders a mirror that survived an I/O error.
            tombstoneDefaults.set(true, forKey: Self.deprovisionedKey)
            _ = tombstoneDefaults.synchronize()
            try? WatchCompanionCredential.erase(from: keychain)
            credential = nil
            if let snapshotStore {
                try? snapshotStore.remove(AgentStatusSnapshot.self)
                try? snapshotStore.remove(PendingApprovalsSnapshot.self)
            }
            if notify { onDeprovision?() }
            reloadComplication()
        }

        private func persist(_ tombstone: WatchWipeTombstone) -> Bool {
            guard let data = try? JSONEncoder().encode(tombstone) else { return false }
            tombstoneDefaults.set(data, forKey: Self.tombstoneKey)
            return tombstoneDefaults.synchronize()
                && tombstoneDefaults.data(forKey: Self.tombstoneKey) == data
        }

        private static func loadTombstone(from defaults: UserDefaults) -> WatchWipeTombstone? {
            guard let data = defaults.data(forKey: tombstoneKey) else { return nil }
            return try? JSONDecoder().decode(WatchWipeTombstone.self, from: data)
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

        func session(
            _ session: WCSession,
            didReceiveMessage message: [String: Any],
            replyHandler: @escaping ([String: Any]) -> Void
        ) {
            Task { @MainActor in
                guard let command = try? WatchWipeCommand.fromPayload(message),
                    let acknowledgement = self.processWipe(command),
                    let payload = try? acknowledgement.messagePayload()
                else {
                    replyHandler([:])
                    return
                }
                replyHandler(payload)
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
            Task { @MainActor in
                if let command = try? WatchWipeCommand.fromPayload(userInfo) {
                    if let acknowledgement = self.processWipe(command),
                        let payload = try? acknowledgement.messagePayload()
                    {
                        session.transferUserInfo(payload)
                    }
                    return
                }
                // The phone also uses the budgeted complication/user-info
                // channel to push a mirror; apply the same generation gate.
                self.storeMirror(from: userInfo)
            }
        }
    }
#endif
