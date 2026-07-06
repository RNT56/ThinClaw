import Foundation
import Testing
import ThinClawAuth

@testable import FeatureOnboarding

// MARK: - Test double

/// A scriptable ``GatewayDiscovering`` that replays caller-supplied sets over an
/// `AsyncStream`, so ``DiscoveryStore`` is exercised without a live `NWBrowser`.
final class FakeDiscovering: GatewayDiscovering, @unchecked Sendable {
    private let sets: [[DiscoveredGateway]]
    private let lock = NSLock()
    private var _stopped = false

    /// - Parameter sets: successive complete result sets to emit, in order.
    init(sets: [[DiscoveredGateway]]) {
        self.sets = sets
    }

    var wasStopped: Bool { lock.withLock { _stopped } }

    func gatewaySets() -> AsyncStream<[DiscoveredGateway]> {
        AsyncStream { continuation in
            for set in sets {
                continuation.yield(set)
            }
            continuation.finish()
        }
    }

    func stop() {
        lock.withLock { _stopped = true }
    }
}

private func gateway(
    _ name: String,
    host: String? = nil,
    port: Int? = nil,
    fp: String? = nil
) -> DiscoveredGateway {
    DiscoveredGateway(
        name: name,
        host: host,
        port: port,
        txt: DiscoveryTXTRecord(name: name, instanceFingerprint: fp))
}

// MARK: - Tests

@MainActor
@Suite("DiscoveryStore")
struct DiscoveryStoreTests {
    /// Drain the store's browse task by yielding until it observes the final
    /// emitted set (the fake finishes its stream synchronously, but the
    /// consuming Task hops back to the main actor to apply each set).
    private func settle(_ store: DiscoveryStore, expected: Int) async {
        for _ in 0..<100 {
            if store.gateways.count == expected && !store.isBrowsing { return }
            await Task.yield()
        }
    }

    @Test("start browses and republishes the latest set")
    func startPublishesSet() async {
        let store = DiscoveryStore(
            browser: FakeDiscovering(sets: [
                [gateway("alpha", host: "10.0.0.2", port: 3443)],
                [
                    gateway("alpha", host: "10.0.0.2", port: 3443),
                    gateway("bravo", host: "10.0.0.3", port: 3443),
                ],
            ]))
        store.start()
        await settle(store, expected: 2)
        #expect(store.gateways.map(\.name) == ["alpha", "bravo"])
    }

    @Test("a gateway going offline drops from the set (whole-set replace)")
    func offlineDrops() async {
        let store = DiscoveryStore(
            browser: FakeDiscovering(sets: [
                [gateway("alpha"), gateway("bravo")],
                [gateway("alpha")],
            ]))
        store.start()
        await settle(store, expected: 1)
        #expect(store.gateways.map(\.name) == ["alpha"])
    }

    @Test("stop clears state and tears down the browser")
    func stopClears() async {
        let fake = FakeDiscovering(sets: [[gateway("alpha", host: "10.0.0.2", port: 3443)]])
        let store = DiscoveryStore(browser: fake)
        store.start()
        await settle(store, expected: 1)
        store.stop()
        #expect(!store.isBrowsing)
        #expect(store.gateways.isEmpty)
        #expect(fake.wasStopped)
    }

    @Test("start is idempotent while already browsing")
    func startIdempotent() async {
        let store = DiscoveryStore(browser: FakeDiscovering(sets: [[gateway("alpha")]]))
        store.start()
        store.start()  // must not crash or spawn a second task
        await settle(store, expected: 1)
        #expect(store.gateways.count == 1)
    }

    @Test("a discovered gateway suggests an https base URL to pre-fill (locator only)")
    func suggestsBaseURLForPrefill() async {
        let store = DiscoveryStore(
            browser: FakeDiscovering(sets: [[gateway("alpha", host: "192.168.1.9", port: 3443)]]))
        store.start()
        await settle(store, expected: 1)
        let found = store.gateways.first
        #expect(found?.suggestedBaseURL == URL(string: "https://192.168.1.9:3443"))
        // The fingerprint is a locator hint, never an authenticator: a
        // fingerprint-less discovery still yields a usable pre-fill candidate.
        #expect(found?.instanceFingerprint == nil)
    }
}
