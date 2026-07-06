#if canImport(Security) && canImport(CryptoKit)
    import Foundation
    import ThinClawAuth

    /// Decides whether the paired phone must (re-)mint a watch companion and, if
    /// so, mints it and builds the ``CompanionProvisioning`` payload to hand to
    /// the watch over `updateApplicationContext`.
    ///
    /// Pure of WatchConnectivity: it takes the watch's reported credential state
    /// and the phone's own ``DeviceCredential`` and returns either a payload to
    /// deliver or `nil` (already provisioned). The `WatchRelayHost` transport
    /// shell calls this and performs the actual context update.
    public struct CompanionProvisioner: Sendable {
        private let gateway: any WatchBridgeGateway
        private let parentCredential: DeviceCredential
        /// Human label for the minted companion (e.g. "Apple Watch").
        private let companionName: String

        public init(
            gateway: any WatchBridgeGateway,
            parentCredential: DeviceCredential,
            companionName: String
        ) {
            self.gateway = gateway
            self.parentCredential = parentCredential
            self.companionName = companionName
        }

        /// Given the watch's reported credential state and the id the phone last
        /// minted (if any), return a fresh provisioning payload when the watch
        /// needs one, else `nil`.
        ///
        /// When minting, the parent credential must carry the gateway identity
        /// (instance id) needed for the watch's pinned direct route; if the
        /// parent has no instance id recorded, it is omitted and the watch is
        /// relay-only until re-pair. Mints only against the parent's own
        /// `devices:self` scope.
        public func provisionIfNeeded(
            watchState: CompanionCredentialState,
            lastProvisionedDeviceID: String?,
            instanceID: String
        ) async throws -> CompanionProvisioning? {
            guard
                watchState.needsProvisioning(
                    lastProvisionedDeviceID: lastProvisionedDeviceID)
            else { return nil }

            let created = try await gateway.mintCompanion(name: companionName)
            return CompanionProvisioning(
                watchToken: created.token,
                companionDeviceID: created.deviceID,
                parentDeviceID: created.parentDeviceID,
                gatewayURLs: parentCredential.gatewayURLs,
                serverFingerprint: parentCredential.serverFingerprint,
                instanceID: instanceID,
                installationID: parentCredential.installationID)
        }

        /// Revoke the watch's companion on the gateway when the phone unpairs (or
        /// the watch is unpaired). Best-effort; a cascade also covers this if the
        /// parent itself is revoked, but an explicit delete keeps a still-paired
        /// phone's device list clean (D-K4).
        public func deprovision(companionDeviceID: String) async throws {
            try await gateway.revokeCompanion(deviceID: companionDeviceID)
        }
    }
#endif
