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
        private let controlStore: PhoneWatchControlStore
        private let session: WCSession
        private var wipeTransport: WatchWipeTransport?

        init(
            keychain: any KeychainStoring = SharedGatewayConnection.keychain(),
            session: WCSession = .default
        ) {
            self.controlStore = PhoneWatchControlStore(keychain: keychain)
            self.session = session
        }

        /// Whether a watch is even worth talking to on this device.
        private var isWatchSupported: Bool { WCSession.isSupported() }

        /// Build (if needed) and activate the relay host from the currently
        /// stored phone credential. Safe to call repeatedly (idempotent): a no-op
        /// when unsupported, unpaired, or already active.
        func activateIfPaired() {
            guard isWatchSupported, host == nil else {
                host?.activate()
                transmitPendingWipes(ownSessionDelegate: false)
                return
            }
            guard let credential = SharedGatewayConnection.loadCredential() else { return }
            guard let material = try? controlStore.prepare(for: credential) else { return }
            let host = WatchRelayHost(
                parentCredential: credential,
                // The QR `iid` captured at pairing is the gateway instance id
                // (stored as `installationID`); the watch pins the same identity
                // for its direct route (D-X3).
                instanceID: credential.installationID,
                companionName: Self.companionName,
                controlMaterial: material,
                lastProvisionedDeviceID: try? controlStore.activeCompanionDeviceID(
                    generation: material.generation),
                onCompanionProvisioned: { [weak self] deviceID, generation in
                    try? self?.controlStore.recordCompanion(
                        deviceID: deviceID, generation: generation)
                },
                onWipeAcknowledged: { [weak self] acknowledgement in
                    self?.accept(acknowledgement)
                },
                session: session)
            self.host = host
            host.activate()
            // A prior generation may still be awaiting an offline Watch ack.
            // The active host owns the WCSession delegate, while the transport
            // can still enqueue/send those token-free commands.
            transmitPendingWipes(ownSessionDelegate: false)
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
            guard let credential = SharedGatewayConnection.loadCredential() else {
                host = nil
                resumePendingWipes()
                return
            }
            guard let plan = try? controlStore.beginWipe(for: credential) else {
                // Server-side parent revoke remains authoritative if local
                // control-state persistence failed, but never retain a relay
                // host after local unpair.
                await host?.deprovisionCompanion()
                host = nil
                return
            }

            // Persisted above; now publish the deprovisioned application context,
            // queue an offline retry, and attempt a live round-trip before any
            // parent credential is revoked by AppDependencies.
            host?.beginDeprovisioning()
            let transport = ensureWipeTransport()
            transport.enqueue([plan], takeSessionDelegate: host == nil)

            if host == nil {
                let revokeHost = WatchRelayHost(
                    parentCredential: credential,
                    instanceID: credential.installationID,
                    companionName: Self.companionName,
                    controlMaterial: plan.material,
                    lastProvisionedDeviceID: plan.companionDeviceID,
                    onWipeAcknowledged: { [weak self] acknowledgement in
                        self?.accept(acknowledgement)
                    },
                    session: session)
                await revokeHost.deprovisionCompanion()
            } else {
                await host?.deprovisionCompanion()
            }
            host = nil
            // With the relay host gone, retain a control-only WCSession delegate
            // until the authenticated ack arrives (including a later app launch).
            transport.enqueue(
                (try? controlStore.pendingWipes()) ?? [],
                takeSessionDelegate: true)
        }

        /// Resume a crash/offline-safe pending wipe even when the phone is no
        /// longer paired. Called at app configuration and after an unpair.
        func resumePendingWipes() {
            transmitPendingWipes(ownSessionDelegate: host == nil)
        }

        private func transmitPendingWipes(ownSessionDelegate: Bool) {
            guard isWatchSupported,
                let plans = try? controlStore.pendingWipes(),
                !plans.isEmpty
            else { return }
            ensureWipeTransport().enqueue(
                plans, takeSessionDelegate: ownSessionDelegate)
        }

        private func ensureWipeTransport() -> WatchWipeTransport {
            if let wipeTransport { return wipeTransport }
            let transport = WatchWipeTransport(
                session: session,
                onAcknowledgement: { [weak self] acknowledgement in
                    self?.accept(acknowledgement) ?? false
                })
            wipeTransport = transport
            return transport
        }

        @discardableResult
        private func accept(_ acknowledgement: WatchWipeAcknowledgement) -> Bool {
            let accepted = (try? controlStore.acknowledge(acknowledgement)) == true
            if accepted {
                wipeTransport?.removeAcknowledged(commandID: acknowledgement.commandID)
            }
            return accepted
        }

        /// Human label for the minted companion, surfaced in the operator's
        /// device list.
        private static let companionName = "Apple Watch"
    }

    /// Token-free WatchConnectivity transport for durable wipe commands. The
    /// command is sent through all three delivery modes: application context
    /// (authoritative deprovisioned state), transferUserInfo (offline retry), and
    /// sendMessage (immediate ack when reachable).
    @MainActor
    private final class WatchWipeTransport: NSObject {
        private let session: WCSession
        private let onAcknowledgement: @MainActor (WatchWipeAcknowledgement) -> Bool
        private var pending: [String: WatchWipeCommand] = [:]
        private var queuedCommandIDs: Set<String> = []
        private var interactiveCommandIDs: Set<String> = []

        init(
            session: WCSession,
            onAcknowledgement: @escaping @MainActor (WatchWipeAcknowledgement) -> Bool
        ) {
            self.session = session
            self.onAcknowledgement = onAcknowledgement
            super.init()
        }

        func enqueue(
            _ plans: [PhoneWatchWipePlan],
            takeSessionDelegate: Bool
        ) {
            for plan in plans { pending[plan.command.commandID] = plan.command }
            guard WCSession.isSupported() else { return }
            if takeSessionDelegate {
                session.delegate = self
                if session.activationState != .activated { session.activate() }
            }
            transmit()
        }

        private func transmit() {
            guard session.activationState == .activated else { return }
            let commands = pending.values.sorted { $0.generation < $1.generation }
            // Application context is last-write-wins: publish the newest wipe.
            if let newest = commands.last,
                let context = try? newest.applicationContext()
            {
                try? session.updateApplicationContext(context)
            }
            for command in commands {
                guard let payload = try? command.messagePayload() else { continue }
                if queuedCommandIDs.insert(command.commandID).inserted {
                    session.transferUserInfo(payload)
                }
                guard session.isReachable,
                    interactiveCommandIDs.insert(command.commandID).inserted
                else { continue }
                session.sendMessage(
                    payload,
                    replyHandler: { [weak self] reply in
                        Task { @MainActor in self?.handleAcknowledgement(reply) }
                    },
                    errorHandler: { [weak self] _ in
                        Task { @MainActor in
                            self?.interactiveCommandIDs.remove(command.commandID)
                        }
                    })
            }
        }

        private func handleAcknowledgement(_ payload: [String: Any]) {
            guard
                let acknowledgement = try? WatchWipeAcknowledgement.fromPayload(payload),
                onAcknowledgement(acknowledgement)
            else { return }
            pending[acknowledgement.commandID] = nil
            interactiveCommandIDs.remove(acknowledgement.commandID)
        }

        func removeAcknowledged(commandID: String) {
            pending[commandID] = nil
            interactiveCommandIDs.remove(commandID)
        }
    }

    extension WatchWipeTransport: @preconcurrency WCSessionDelegate {
        nonisolated func session(
            _ session: WCSession,
            activationDidCompleteWith activationState: WCSessionActivationState,
            error: (any Error)?
        ) {
            Task { @MainActor in self.transmit() }
        }

        #if os(iOS)
            nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

            nonisolated func sessionDidDeactivate(_ session: WCSession) {
                session.activate()
            }

            nonisolated func sessionWatchStateDidChange(_ session: WCSession) {
                Task { @MainActor in self.transmit() }
            }
        #endif

        nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
            Task { @MainActor in self.transmit() }
        }

        nonisolated func session(
            _ session: WCSession,
            didReceiveApplicationContext applicationContext: [String: Any]
        ) {
            Task { @MainActor in self.handleAcknowledgement(applicationContext) }
        }

        nonisolated func session(
            _ session: WCSession,
            didReceiveUserInfo userInfo: [String: Any] = [:]
        ) {
            Task { @MainActor in self.handleAcknowledgement(userInfo) }
        }

        nonisolated func session(
            _ session: WCSession,
            didReceiveMessage message: [String: Any],
            replyHandler: @escaping ([String: Any]) -> Void
        ) {
            Task { @MainActor in
                self.handleAcknowledgement(message)
                replyHandler([:])
            }
        }
    }
#endif
