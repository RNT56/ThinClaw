import Foundation
import Testing

@testable import ThinClawWatchBridge

@Suite("CompanionProvisioning application-context coding")
struct CompanionProvisioningTests {
    private func sample() -> CompanionProvisioning {
        CompanionProvisioning(
            watchToken: "tcd_watch_companion_abc",
            companionDeviceID: "dev-watch-1",
            parentDeviceID: "dev-phone-1",
            gatewayURLs: [URL(string: "https://host.local:3443")!],
            serverFingerprint: "fp-base64url",
            instanceID: "inst-xyz",
            installationID: "install-1")
    }

    @Test("Provisioning payload round-trips through an application context")
    func roundTripsThroughContext() throws {
        let payload = sample()
        let context = try payload.applicationContext()
        let back = try CompanionProvisioning.fromApplicationContext(context)
        #expect(back == payload)
    }

    @Test("Provisioning carries the watch's OWN token and its parent id")
    func carriesWatchTokenAndParent() throws {
        let payload = sample()
        #expect(payload.watchToken.hasPrefix("tcd_"))
        #expect(payload.parentDeviceID == "dev-phone-1")
        #expect(payload.companionDeviceID == "dev-watch-1")
    }

    @Test("A context with no provisioning payload decodes to nil")
    func absentPayloadIsNil() throws {
        let context: [String: Any] = ["somethingElse": Data()]
        #expect(try CompanionProvisioning.fromApplicationContext(context) == nil)
    }

    @Test("An unknown provisioning version is rejected")
    func unknownVersionRejected() throws {
        var payload = sample()
        payload.version = 42
        let data = try JSONEncoder().encode(payload)
        let context = [CompanionProvisioning.contextKey: data]
        #expect(throws: WatchRelayError.unsupportedVersion(42)) {
            try CompanionProvisioning.fromApplicationContext(context)
        }
    }
}

@Suite("CompanionCredentialState re-provision policy")
struct CompanionCredentialStateTests {
    @Test("A watch with no credential always needs provisioning")
    func noCredentialNeedsProvisioning() {
        let state = CompanionCredentialState(hasCredential: false)
        #expect(state.needsProvisioning(lastProvisionedDeviceID: "dev-watch-1"))
        #expect(state.needsProvisioning(lastProvisionedDeviceID: nil))
    }

    @Test("A matching credential does not need re-provisioning")
    func matchingCredentialIsStable() {
        let state = CompanionCredentialState(
            hasCredential: true, companionDeviceID: "dev-watch-1")
        #expect(!state.needsProvisioning(lastProvisionedDeviceID: "dev-watch-1"))
    }

    @Test("A stale/foreign credential id triggers re-provisioning")
    func staleCredentialReprovisioned() {
        let state = CompanionCredentialState(
            hasCredential: true, companionDeviceID: "dev-old")
        #expect(state.needsProvisioning(lastProvisionedDeviceID: "dev-watch-1"))
    }

    @Test("A credential the phone never minted is re-provisioned authoritatively")
    func unknownProvenanceReprovisioned() {
        let state = CompanionCredentialState(
            hasCredential: true, companionDeviceID: "dev-watch-1")
        #expect(state.needsProvisioning(lastProvisionedDeviceID: nil))
    }
}
