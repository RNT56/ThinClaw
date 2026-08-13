#if canImport(WatchConnectivity) && canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAuth
    import ThinClawSnapshotKit
    import WatchConnectivity

    /// iOS-side host of the watch relay (docs/MOBILE_SECURITY.md D-K4).
    ///
    /// Responsibilities:
    ///  1. **Provision** — when the watch is paired/reachable and reports it has
    ///     no (or a stale) companion credential, mint one via
    ///     `POST /api/devices/me/companions` and push it to the watch over
    ///     `updateApplicationContext` (token + gateway URLs + SPKI pin +
    ///     instance id). The watch stores it in its **own** keychain.
    ///  2. **Relay** — answer watch RPCs by forwarding them to the gateway with
    ///     the **watch's own token** (never the phone's), via ``WatchRelayResponder``.
    ///  3. **Mirror** — push the latest agent-status / pending-approvals snapshot
    ///     to the watch on significant changes.
    ///  4. **Deprovision** — DELETE the companion when the phone unpairs.
    ///
    /// The class is a thin `WCSessionDelegate` transport shell over the pure
    /// ``WatchRelayResponder`` / ``CompanionProvisioner`` seams, which carry the
    /// testable logic. It is created only while the phone is paired (it needs the
    /// parent ``DeviceCredential``); on unpair the app tears it down after a best-
    /// effort `deprovision`.
    @MainActor
    public final class WatchRelayHost: NSObject {
        private let parentCredential: DeviceCredential
        private let instanceID: String
        private let responder: WatchRelayResponder
        private let provisioner: CompanionProvisioner
        private let session: WCSession
        private let controlMaterial: WatchControlMaterial
        private let onCompanionProvisioned: @MainActor @Sendable (String, UInt64) -> Void
        private let onWipeAcknowledged: @MainActor @Sendable (WatchWipeAcknowledgement) -> Void

        /// The companion device id the phone last minted this run — used to
        /// decide whether the watch's reported credential is stale, and to
        /// deprovision on unpair.
        private var lastProvisionedDeviceID: String?
        /// Once unpair begins, never mint, mirror, or start another relayed
        /// action while the explicit companion revoke is in flight.
        private var isDeprovisioning = false

        /// Build a host over the phone's paired credential. `instanceID` is the
        /// gateway instance id captured at pairing (D-X3) — forwarded to the
        /// watch so its direct route pins the same identity. `session` defaults
        /// to `.default`; tests inject nothing (they exercise the pure seams).
        public init(
            parentCredential: DeviceCredential,
            instanceID: String,
            companionName: String,
            controlMaterial: WatchControlMaterial,
            lastProvisionedDeviceID: String? = nil,
            onCompanionProvisioned: @escaping @MainActor @Sendable (String, UInt64) -> Void = {
                _, _ in
            },
            onWipeAcknowledged:
                @escaping @MainActor @Sendable (
                    WatchWipeAcknowledgement
                ) -> Void = { _ in },
            gateway: (any WatchBridgeGateway)? = nil,
            session: WCSession = .default
        ) {
            self.parentCredential = parentCredential
            self.instanceID = instanceID
            let gateway = gateway ?? LiveWatchBridgeGateway(parentCredential: parentCredential)
            self.responder = WatchRelayResponder(gateway: gateway)
            self.provisioner = CompanionProvisioner(
                gateway: gateway,
                parentCredential: parentCredential,
                companionName: companionName)
            self.session = session
            self.controlMaterial = controlMaterial
            self.lastProvisionedDeviceID = lastProvisionedDeviceID
            self.onCompanionProvisioned = onCompanionProvisioned
            self.onWipeAcknowledged = onWipeAcknowledged
            super.init()
        }

        /// Begin relaying: set the delegate and activate the session. Idempotent.
        public func activate() {
            guard WCSession.isSupported() else { return }
            session.delegate = self
            if session.activationState != .activated {
                session.activate()
            }
        }

        /// Close the relay/provisioning boundary synchronously before unpair
        /// starts any asynchronous network revoke.
        public func beginDeprovisioning() {
            isDeprovisioning = true
        }

        /// Best-effort deprovision (DELETE the companion) before the app tears
        /// the host down on unpair. The parent-revoke cascade would also cover
        /// this, but an explicit delete keeps a still-paired phone tidy.
        public func deprovisionCompanion() async {
            beginDeprovisioning()
            guard let deviceID = lastProvisionedDeviceID else { return }
            try? await provisioner.deprovision(companionDeviceID: deviceID)
            lastProvisionedDeviceID = nil
        }

        /// Push the freshest snapshot to the watch as application context, so a
        /// glance shows current status/approvals without a round-trip. Called on
        /// significant agent-state changes by the app.
        public func pushSnapshot(
            status: AgentStatusSnapshot,
            approvals: PendingApprovalsSnapshot
        ) {
            guard !isDeprovisioning, session.activationState == .activated else { return }
            guard
                let context = try? WatchSnapshotMirror.applicationContext(
                    status: status,
                    approvals: approvals,
                    provisioningGeneration: controlMaterial.generation)
            else { return }
            try? session.updateApplicationContext(context)
            // Complications refresh on a separate, budgeted channel. The
            // complication-transfer API is iOS-only (`__WATCHOS_UNAVAILABLE`);
            // this host only ever runs on the phone, but the file also compiles
            // for the watch target, so gate the phone-only call.
            #if os(iOS)
                if session.isComplicationEnabled {
                    session.transferCurrentComplicationUserInfo(context)
                }
            #endif
        }

        // MARK: - Provisioning

        /// Mint + deliver a companion credential to the watch when it reports it
        /// needs one. Merges the provisioning payload into the current app
        /// context so a concurrent snapshot push is not clobbered.
        private func provisionIfNeeded(reportedBy watchState: CompanionCredentialState) {
            guard !isDeprovisioning else { return }
            Task { @MainActor in
                do {
                    guard !isDeprovisioning else { return }
                    guard
                        let payload = try await provisioner.provisionIfNeeded(
                            watchState: watchState,
                            lastProvisionedDeviceID: lastProvisionedDeviceID,
                            instanceID: instanceID,
                            controlMaterial: controlMaterial)
                    else { return }
                    guard !isDeprovisioning else {
                        // A mint already in flight when unpair began must not
                        // leave a fresh companion behind or publish its token.
                        try? await provisioner.deprovision(
                            companionDeviceID: payload.companionDeviceID)
                        return
                    }
                    lastProvisionedDeviceID = payload.companionDeviceID
                    onCompanionProvisioned(
                        payload.companionDeviceID,
                        payload.provisioningGeneration)
                    if let context = try? payload.applicationContext() {
                        try? session.updateApplicationContext(context)
                    }
                } catch {
                    // Leave the watch unprovisioned; it will re-report and we
                    // retry on the next reachability/state message.
                }
            }
        }

        // MARK: - Relay

        private func handle(
            message: [String: Any],
            reply: @escaping ([String: Any]) -> Void
        ) {
            if let acknowledgement = try? WatchWipeAcknowledgement.fromPayload(message) {
                onWipeAcknowledged(acknowledgement)
                reply([:])
                return
            }
            guard !isDeprovisioning else {
                reply((try? WatchRelayResponse.reprovisionRequired.messagePayload()) ?? [:])
                return
            }
            guard let envelope = try? WatchRelayEnvelope.fromMessage(message) else {
                reply((try? WatchRelayResponse.failed(reason: "malformed").messagePayload()) ?? [:])
                return
            }
            let phoneToken = parentCredential.deviceToken
            Task { @MainActor in
                guard !isDeprovisioning else {
                    reply((try? WatchRelayResponse.reprovisionRequired.messagePayload()) ?? [:])
                    return
                }
                let response = await responder.answer(envelope, phoneToken: phoneToken)
                reply((try? response.messagePayload()) ?? [:])
            }
        }
    }

    // MARK: - WCSessionDelegate

    extension WatchRelayHost: @preconcurrency WCSessionDelegate {
        public func session(
            _ session: WCSession,
            activationDidCompleteWith activationState: WCSessionActivationState,
            error: (any Error)?
        ) {}

        // These three delegate callbacks are iOS-only (`__WATCHOS_UNAVAILABLE`):
        // they cover the phone re-pairing to a different watch and watch-state
        // transitions. `WatchRelayHost` is only ever instantiated on the phone,
        // but the file compiles for the watch target too (shared package,
        // `canImport(WatchConnectivity)`), so the overrides must be elided there.
        #if os(iOS)
            // Required no-op stubs on iOS: the session can re-pair to a new watch.
            public func sessionDidBecomeInactive(_ session: WCSession) {}
            public func sessionDidDeactivate(_ session: WCSession) {
                // Re-activate for a newly-paired watch.
                session.activate()
            }

            public func sessionWatchStateDidChange(_ session: WCSession) {
                // A newly-paired/reachable watch may need provisioning; it reports
                // its credential state, but nudge by treating an activated+paired
                // watch with no known provision as needing one.
                if session.isPaired, lastProvisionedDeviceID == nil {
                    provisionIfNeeded(reportedBy: CompanionCredentialState(hasCredential: false))
                }
            }
        #endif

        public func session(
            _ session: WCSession,
            didReceiveMessage message: [String: Any],
            replyHandler: @escaping ([String: Any]) -> Void
        ) {
            handle(message: message, reply: replyHandler)
        }

        public func session(
            _ session: WCSession,
            didReceiveApplicationContext applicationContext: [String: Any]
        ) {
            if let acknowledgement = try? WatchWipeAcknowledgement.fromPayload(
                applicationContext)
            {
                onWipeAcknowledged(acknowledgement)
            }
            guard !isDeprovisioning else { return }
            // The watch reports its credential state via context so the phone can
            // (re-)provision without a round-trip.
            if let data = applicationContext[CompanionCredentialState.contextKey] as? Data,
                let state = try? JSONDecoder().decode(CompanionCredentialState.self, from: data)
            {
                provisionIfNeeded(reportedBy: state)
            }
        }

        public func session(
            _ session: WCSession,
            didReceiveUserInfo userInfo: [String: Any] = [:]
        ) {
            if let acknowledgement = try? WatchWipeAcknowledgement.fromPayload(userInfo) {
                onWipeAcknowledged(acknowledgement)
                return
            }
            guard !isDeprovisioning else { return }
            // A queued watch RPC arriving via transferUserInfo (no live reply
            // channel). Forward it; the outcome flows back as a snapshot/context.
            guard let envelope = try? WatchRelayEnvelope.fromMessage(userInfo) else { return }
            let phoneToken = parentCredential.deviceToken
            Task { @MainActor in
                guard !isDeprovisioning else { return }
                _ = await responder.answer(envelope, phoneToken: phoneToken)
            }
        }
    }
#endif
