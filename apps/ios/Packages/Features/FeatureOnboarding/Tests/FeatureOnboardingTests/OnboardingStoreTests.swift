import Foundation
import Testing
import ThinClawAuth

@testable import FeatureOnboarding

// MARK: - Test doubles

/// Scriptable ``PairingService`` fake: returns a canned result/error and
/// records the arguments each call received.
final class FakePairingService: PairingService, @unchecked Sendable {
    enum Outcome {
        case result(PairingResult)
        case failure(PairingError)
    }

    private let lock = NSLock()
    private var outcome: Outcome
    private(set) var calls: [(payload: PairingPayload, redemption: PairingRedemption, name: String)] =
        []

    init(_ outcome: Outcome) {
        self.outcome = outcome
    }

    func setOutcome(_ outcome: Outcome) {
        lock.withLock { self.outcome = outcome }
    }

    var callCount: Int { lock.withLock { calls.count } }
    var lastName: String? { lock.withLock { calls.last?.name } }
    var lastRedemption: PairingRedemption? { lock.withLock { calls.last?.redemption } }

    func pair(
        payload: PairingPayload,
        redemption: PairingRedemption,
        deviceName: String
    ) async throws(PairingError) -> PairingResult {
        let current: Outcome = lock.withLock {
            calls.append((payload, redemption, deviceName))
            return outcome
        }
        switch current {
        case .result(let result): return result
        case .failure(let error): throw error
        }
    }
}

// MARK: - Fixtures

@MainActor
private enum Fixture {
    static let pinnedPayload = PairingPayload(
        version: 1,
        urls: [URL(string: "https://100.100.1.2:3443")!],
        fingerprint: "qMnE3hSuF3zXV0AJIT9cKW0eGD6dV3nCFbYbBGDs0XU",
        installationID: "inst_9f8e",
        name: "home-server",
        secret: "pair_secret_abc",
        expiresAt: .distantFuture)

    static let credential = DeviceCredential(
        installationID: "inst_9f8e",
        deviceID: "dev_123",
        deviceToken: "tcd_abcdef",
        gatewayURLs: [URL(string: "https://100.100.1.2:3443")!],
        serverFingerprint: "qMnE3hSuF3zXV0AJIT9cKW0eGD6dV3nCFbYbBGDs0XU",
        gatewayName: "home-server",
        pairedAt: Date(timeIntervalSince1970: 1_700_000_000))

    /// Build a valid `thinclaw://pair?d=…` URL from a wire JSON dict.
    static func pairingURL(
        v: Int = 1,
        urls: [String] = ["https://100.100.1.2:3443"],
        fp: String? = "qMnE3hSuF3zXV0AJIT9cKW0eGD6dV3nCFbYbBGDs0XU",
        iid: String = "inst_9f8e",
        name: String = "home-server",
        sec: String = "pair_secret_abc",
        exp: Int = 4_000_000_000
    ) -> URL {
        var json: [String: Any] = [
            "v": v, "urls": urls, "iid": iid, "name": name, "sec": sec, "exp": exp,
        ]
        if let fp { json["fp"] = fp }
        let data = try! JSONSerialization.data(withJSONObject: json)
        let encoded = data.base64URLEncodedString()
        return URL(string: "thinclaw://pair?d=\(encoded)")!
    }

    @MainActor
    static func makeStore(
        _ service: FakePairingService,
        keychain: InMemoryKeychain = InMemoryKeychain(),
        deviceName: String = "My iPhone",
        onPaired: @escaping @MainActor (DeviceCredential) -> Void = { _ in }
    ) -> OnboardingStore {
        OnboardingStore(
            pairingService: service,
            keychain: keychain,
            deviceName: deviceName,
            onPaired: onPaired)
    }
}

// MARK: - handleScanned parsing

@MainActor
@Suite("OnboardingStore.handleScanned")
struct HandleScannedTests {
    @Test("valid QR advances to confirmGateway with pinned badge")
    func validQR() {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        store.handleScanned(Fixture.pairingURL())
        #expect(
            store.step
                == .confirmGateway(
                    name: "home-server", instanceID: "inst_9f8e", badge: .pinnedTLS))
        #expect(store.pendingPayload != nil)
    }

    @Test("a non-pairing URL fails with an actionable message")
    func notAPairingURL() {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        store.handleScanned(URL(string: "https://example.com")!)
        guard case .failed(let message) = store.step else {
            Issue.record("expected .failed, got \(store.step)")
            return
        }
        #expect(message.contains("ThinClaw pairing link"))
    }

    @Test("an expired QR fails with the expiry message")
    func expiredQR() {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        store.handleScanned(Fixture.pairingURL(exp: 1))  // 1970-ish, long past
        guard case .failed(let message) = store.step else {
            Issue.record("expected .failed, got \(store.step)")
            return
        }
        #expect(message.contains("expired"))
    }

    @Test("an unsupported version fails and asks to update")
    func unsupportedVersion() {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        store.handleScanned(Fixture.pairingURL(v: 99))
        guard case .failed(let message) = store.step else {
            Issue.record("expected .failed, got \(store.step)")
            return
        }
        #expect(message.contains("newer app"))
    }

    @Test("a fingerprint-less tailnet-http payload shows the vpn-http badge")
    func vpnHTTPBadge() {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        store.handleScanned(Fixture.pairingURL(urls: ["http://100.100.1.2:3000"], fp: nil))
        guard case .confirmGateway(_, _, let badge) = store.step else {
            Issue.record("expected .confirmGateway, got \(store.step)")
            return
        }
        #expect(badge == .vpnHTTPWarning)
    }
}

// MARK: - confirmAndPair happy path

@MainActor
@Suite("OnboardingStore pairing")
struct PairingTests {
    @Test("happy path pairs, persists the credential, fires onPaired, lands done")
    func happyPath() async {
        let keychain = InMemoryKeychain()
        var paired: DeviceCredential?
        let service = FakePairingService(.result(.paired(Fixture.credential)))
        let store = Fixture.makeStore(service, keychain: keychain, deviceName: "Ridley's iPhone") {
            paired = $0
        }
        store.handleScanned(Fixture.pairingURL())
        await store.confirmAndPair()

        #expect(store.step == .done)
        #expect(service.callCount == 1)
        #expect(service.lastName == "Ridley's iPhone")
        // secret redemption from the QR
        #expect(service.lastRedemption == .secret("pair_secret_abc"))
        // credential persisted + callback fired
        let stored = (try? DeviceCredential.load(from: keychain)) ?? nil
        #expect(stored?.deviceToken == "tcd_abcdef")
        #expect(paired?.deviceToken == "tcd_abcdef")
    }

    @Test("blank device name falls back to a default, never empty")
    func blankNameFallback() async {
        let service = FakePairingService(.result(.paired(Fixture.credential)))
        let store = Fixture.makeStore(service, deviceName: "   ")
        store.handleScanned(Fixture.pairingURL())
        await store.confirmAndPair()
        #expect(service.lastName == "iPhone")
    }

    @Test("require_confirm mode parks in pendingApproval, stores nothing")
    func pendingApproval() async {
        let keychain = InMemoryKeychain()
        let service = FakePairingService(.result(.pendingConfirmation(pairingID: "pair_42")))
        let store = Fixture.makeStore(service, keychain: keychain)
        store.handleScanned(Fixture.pairingURL())
        await store.confirmAndPair()
        #expect(store.step == .pendingApproval(pairingID: "pair_42"))
        #expect(keychain.count == 0)
    }

    @Test("confirmAndPair with no pending payload fails instead of crashing")
    func noPending() async {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        await store.confirmAndPair()
        guard case .failed = store.step else {
            Issue.record("expected .failed, got \(store.step)")
            return
        }
    }
}

// MARK: - Failure mapping (each PairingError -> actionable failed())

@MainActor
@Suite("OnboardingStore failure messages")
struct FailureTests {
    private func message(for error: PairingError) async -> String {
        let store = Fixture.makeStore(FakePairingService(.failure(error)))
        store.handleScanned(Fixture.pairingURL())
        await store.confirmAndPair()
        guard case .failed(let message) = store.step else { return "" }
        return message
    }

    @Test("rejected credential message tells the operator to re-pair")
    func rejected() async {
        let message = await message(for: .rejectedCredential)
        #expect(message.contains("used already") || message.contains("expired"))
    }

    @Test("rate limited message asks to wait")
    func rateLimited() async {
        let message = await message(for: .rateLimited(retryAfter: 30))
        #expect(message.contains("Too many attempts"))
    }

    @Test("pin mismatch message warns about TLS identity")
    func pinMismatch() async {
        let message = await message(for: .pinMismatch)
        #expect(message.contains("TLS identity"))
    }

    @Test("transport failure message asks to check the connection")
    func transport() async {
        let message = await message(for: .transport(.timedOut))
        #expect(message.contains("reach the gateway"))
    }

    @Test("no reachable endpoint message mentions network/VPN")
    func noEndpoint() async {
        let message = await message(for: .noReachableEndpoint)
        #expect(message.contains("network") || message.contains("VPN"))
    }

    @Test("server error message includes the status")
    func server() async {
        let message = await message(for: .server(status: 503))
        #expect(message.contains("503"))
    }

    @Test("storage failure message admits the pairing but the save failed")
    func storageFailure() async {
        let message = await message(for: .credentialStorageFailed)
        #expect(message.contains("save"))
    }

    @Test("every PairingError yields a non-empty user message")
    func allNonEmpty() {
        let errors: [PairingError] = [
            .invalidPayload(.notAPairingURL), .invalidPayload(.malformedPayload),
            .invalidPayload(.unsupportedVersion(2)), .invalidPayload(.expired(.distantPast)),
            .invalidPayload(.noUsableURLs), .noReachableEndpoint, .rejectedCredential,
            .rateLimited(retryAfter: nil), .server(status: 500), .pinMismatch,
            .transport(.notConnectedToInternet), .keyGenerationFailed,
            .credentialStorageFailed, .unexpected(status: 418),
        ]
        for error in errors {
            #expect(!error.userMessage.isEmpty, "empty message for \(error)")
        }
    }
}

// MARK: - Retry

@MainActor
@Suite("OnboardingStore retry")
struct RetryTests {
    @Test("retry re-runs the last attempt and can succeed the second time")
    func retrySucceeds() async {
        let keychain = InMemoryKeychain()
        let service = FakePairingService(.failure(.transport(.timedOut)))
        let store = Fixture.makeStore(service, keychain: keychain)
        store.handleScanned(Fixture.pairingURL())
        await store.confirmAndPair()
        guard case .failed = store.step else {
            Issue.record("expected .failed after first attempt")
            return
        }
        // Gateway comes back; retry uses the retained payload.
        service.setOutcome(.result(.paired(Fixture.credential)))
        await store.retry()
        #expect(store.step == .done)
        #expect(service.callCount == 2)
    }

    @Test("retry with nothing pending resets to welcome")
    func retryNoPending() async {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        await store.retry()
        #expect(store.step == .welcome)
    }

    @Test("reset clears the pending payload and returns to welcome")
    func resetClears() {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        store.handleScanned(Fixture.pairingURL())
        #expect(store.pendingPayload != nil)
        store.reset()
        #expect(store.step == .welcome)
        #expect(store.pendingPayload == nil)
    }
}

// MARK: - Manual (no-camera) paths

@MainActor
@Suite("OnboardingStore manual entry")
struct ManualEntryTests {
    @Test("pasted link behaves exactly like a scan")
    func pastedLink() {
        let store = Fixture.makeStore(FakePairingService(.result(.paired(Fixture.credential))))
        store.handlePastedLink("  \(Fixture.pairingURL().absoluteString)  ")
        guard case .confirmGateway = store.step else {
            Issue.record("expected .confirmGateway, got \(store.step)")
            return
        }
    }

    @Test("manual code + https gateway pairs via the code redemption path")
    func manualCodeSucceeds() async {
        let keychain = InMemoryKeychain()
        let service = FakePairingService(.result(.paired(Fixture.credential)))
        let store = Fixture.makeStore(service, keychain: keychain)
        await store.pairWithManualCode(
            gatewayURL: "https://gateway.local:3443", code: "482913")
        #expect(store.step == .done)
        #expect(service.lastRedemption == .code("482913"))
        #expect(service.calls.last?.payload.fingerprint == nil)
    }

    @Test("manual code with a non-http URL fails before any network call")
    func manualCodeBadURL() async {
        let service = FakePairingService(.result(.paired(Fixture.credential)))
        let store = Fixture.makeStore(service)
        await store.pairWithManualCode(gatewayURL: "ftp://nope", code: "482913")
        guard case .failed = store.step else {
            Issue.record("expected .failed, got \(store.step)")
            return
        }
        #expect(service.callCount == 0)
    }

    @Test("manual code with an empty code fails before any network call")
    func manualCodeEmptyCode() async {
        let service = FakePairingService(.result(.paired(Fixture.credential)))
        let store = Fixture.makeStore(service)
        await store.pairWithManualCode(gatewayURL: "https://gateway.local:3443", code: "   ")
        guard case .failed = store.step else {
            Issue.record("expected .failed, got \(store.step)")
            return
        }
        #expect(service.callCount == 0)
    }
}

// MARK: - Badge classification

@MainActor
@Suite("OnboardingStore.badge")
struct BadgeTests {
    @Test("a pinned payload is always pinnedTLS")
    func pinned() {
        #expect(OnboardingStore.badge(for: Fixture.pinnedPayload) == .pinnedTLS)
    }

    @Test("https without a pin is public-chain TLS, not a warning")
    func httpsNoPin() {
        let payload = PairingPayload(
            version: 1, urls: [URL(string: "https://gw.example.com")!], fingerprint: nil,
            installationID: "i", name: "n", secret: "s", expiresAt: .distantFuture)
        #expect(OnboardingStore.badge(for: payload) == .pinnedTLS)
    }

    @Test("plaintext tailnet without a pin is the badged vpn-http path")
    func tailnetHTTP() {
        let payload = PairingPayload(
            version: 1, urls: [URL(string: "http://100.100.1.2:3000")!], fingerprint: nil,
            installationID: "i", name: "n", secret: "s", expiresAt: .distantFuture)
        #expect(OnboardingStore.badge(for: payload) == .vpnHTTPWarning)
    }
}
