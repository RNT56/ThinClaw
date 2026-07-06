#if canImport(Security) && canImport(CryptoKit)
    import Foundation
    import Testing
    import ThinClawAuth

    @testable import ThinClawWatchBridge

    private final class MintGateway: WatchBridgeGateway, @unchecked Sendable {
        private(set) var mintCount = 0
        private(set) var revokedID: String?
        var minted = CreatedCompanion(
            deviceID: "dev-watch-1", parentDeviceID: "dev-phone-1", token: "tcd_watch_new")

        func mintCompanion(name: String) async throws -> CreatedCompanion {
            mintCount += 1
            return minted
        }
        func revokeCompanion(deviceID: String) async throws { revokedID = deviceID }
        func forwardApproval(
            watchToken: String, requestID: String, threadID: String?, action: String
        ) async throws {}
        func forwardQuickAsk(
            watchToken: String, prompt: String, threadID: String?
        ) async throws -> String { "" }
    }

    @Suite("CompanionProvisioner mint/skip/deprovision")
    struct CompanionProvisionerTests {
        private func parentCredential() -> DeviceCredential {
            DeviceCredential(
                installationID: "install-1",
                deviceID: "dev-phone-1",
                deviceToken: "tcd_phone",
                gatewayURLs: [URL(string: "https://host.local:3443")!],
                serverFingerprint: "fp",
                gatewayName: "Mini",
                pairedAt: Date(timeIntervalSince1970: 0))
        }

        private func provisioner(_ gateway: MintGateway) -> CompanionProvisioner {
            CompanionProvisioner(
                gateway: gateway,
                parentCredential: parentCredential(),
                companionName: "Apple Watch")
        }

        @Test("Mints and builds a payload when the watch has no credential")
        func mintsWhenMissing() async throws {
            let gateway = MintGateway()
            let payload = try await provisioner(gateway).provisionIfNeeded(
                watchState: CompanionCredentialState(hasCredential: false),
                lastProvisionedDeviceID: nil,
                instanceID: "inst-1")

            #expect(gateway.mintCount == 1)
            #expect(payload?.watchToken == "tcd_watch_new")
            #expect(payload?.companionDeviceID == "dev-watch-1")
            #expect(payload?.parentDeviceID == "dev-phone-1")
            // The pin + gateway identity ride along for the watch's direct route.
            #expect(payload?.serverFingerprint == "fp")
            #expect(payload?.instanceID == "inst-1")
            #expect(payload?.installationID == "install-1")
        }

        @Test("Skips minting when the watch already holds the current credential")
        func skipsWhenCurrent() async throws {
            let gateway = MintGateway()
            let payload = try await provisioner(gateway).provisionIfNeeded(
                watchState: CompanionCredentialState(
                    hasCredential: true, companionDeviceID: "dev-watch-1"),
                lastProvisionedDeviceID: "dev-watch-1",
                instanceID: "inst-1")

            #expect(payload == nil)
            #expect(gateway.mintCount == 0)
        }

        @Test("Re-mints when the watch reports a stale credential id")
        func reMintsWhenStale() async throws {
            let gateway = MintGateway()
            let payload = try await provisioner(gateway).provisionIfNeeded(
                watchState: CompanionCredentialState(
                    hasCredential: true, companionDeviceID: "dev-old"),
                lastProvisionedDeviceID: "dev-watch-1",
                instanceID: "inst-1")

            #expect(gateway.mintCount == 1)
            #expect(payload?.companionDeviceID == "dev-watch-1")
        }

        @Test("Deprovision revokes the companion by id")
        func deprovisionRevokes() async throws {
            let gateway = MintGateway()
            try await provisioner(gateway).deprovision(companionDeviceID: "dev-watch-1")
            #expect(gateway.revokedID == "dev-watch-1")
        }
    }
#endif
