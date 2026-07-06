#if canImport(Security) && canImport(CryptoKit)
    import Foundation
    import Testing

    @testable import ThinClawWatchBridge

    private final class StubRelay: WatchRelayTransport, @unchecked Sendable {
        var isReachable: Bool
        var result: Result<WatchRelayResponse, Error>
        private(set) var relayedEnvelope: WatchRelayEnvelope?

        init(reachable: Bool, result: Result<WatchRelayResponse, Error>) {
            self.isReachable = reachable
            self.result = result
        }
        func relay(_ envelope: WatchRelayEnvelope) async throws -> WatchRelayResponse {
            relayedEnvelope = envelope
            return try result.get()
        }
    }

    private final class StubDirect: WatchDirectTransport, @unchecked Sendable {
        var isReachable: Bool
        var result: Result<WatchRelayResponse, Error>
        private(set) var called = false

        init(reachable: Bool, result: Result<WatchRelayResponse, Error>) {
            self.isReachable = reachable
            self.result = result
        }
        func direct(_ request: WatchRelayRequest) async throws -> WatchRelayResponse {
            called = true
            return try result.get()
        }
    }

    private final class SpyQueue: WatchQueueTransport, @unchecked Sendable {
        private(set) var enqueued: [WatchRelayEnvelope] = []
        func enqueue(_ envelope: WatchRelayEnvelope) async { enqueued.append(envelope) }
    }

    @Suite("WatchGatewayRouter route selection and fall-through")
    struct WatchGatewayRouterTests {
        private func proxy(
            relay: StubRelay, direct: StubDirect, queue: SpyQueue,
            token: String? = "tcd_watch"
        ) -> WatchGatewayRouter {
            WatchGatewayRouter(relay: relay, direct: direct, queue: queue, watchToken: token)
        }

        @Test("Reachable relay wins and forwards the watch token")
        func relayWins() async {
            let relay = StubRelay(reachable: true, result: .success(.accepted))
            let direct = StubDirect(reachable: true, result: .success(.accepted))
            let queue = SpyQueue()
            let outcome = await proxy(relay: relay, direct: direct, queue: queue)
                .approve(requestID: "r", threadID: nil, action: "approve")

            #expect(outcome == .completed(route: .relay, response: .accepted))
            #expect(relay.relayedEnvelope?.watchToken == "tcd_watch")
            #expect(!direct.called)
            #expect(queue.enqueued.isEmpty)
        }

        @Test("Relay timeout falls through to direct")
        func relayTimeoutFallsToDirect() async {
            let relay = StubRelay(reachable: true, result: .failure(WatchRelayError.timedOut))
            let direct = StubDirect(reachable: true, result: .success(.accepted))
            let queue = SpyQueue()
            let outcome = await proxy(relay: relay, direct: direct, queue: queue)
                .approve(requestID: "r", threadID: nil, action: "approve")

            #expect(outcome == .completed(route: .direct, response: .accepted))
            #expect(direct.called)
            #expect(queue.enqueued.isEmpty)
        }

        @Test("Relay + direct both fail: the request is queued")
        func bothFailQueues() async {
            let relay = StubRelay(reachable: true, result: .failure(WatchRelayError.timedOut))
            let direct = StubDirect(
                reachable: true, result: .failure(WatchRelayError.timedOut))
            let queue = SpyQueue()
            let outcome = await proxy(relay: relay, direct: direct, queue: queue)
                .quickAsk("hi", threadID: nil)

            #expect(outcome == .queued)
            #expect(queue.enqueued.count == 1)
            #expect(queue.enqueued.first?.watchToken == "tcd_watch")
        }

        @Test("No route reachable: the request is queued immediately")
        func nothingReachableQueues() async {
            let relay = StubRelay(reachable: false, result: .success(.accepted))
            let direct = StubDirect(reachable: false, result: .success(.accepted))
            let queue = SpyQueue()
            let outcome = await proxy(relay: relay, direct: direct, queue: queue)
                .approve(requestID: "r", threadID: nil, action: "deny")

            #expect(outcome == .queued)
            #expect(queue.enqueued.count == 1)
        }

        @Test("An unprovisioned watch cannot relay; it falls through to direct")
        func unprovisionedSkipsRelay() async {
            // No watch token ⇒ relay is not attributable, so relayReachable is
            // false even though the phone is reachable; direct carries it.
            let relay = StubRelay(reachable: true, result: .success(.accepted))
            let direct = StubDirect(reachable: true, result: .success(.accepted))
            let queue = SpyQueue()
            let outcome = await proxy(relay: relay, direct: direct, queue: queue, token: nil)
                .approve(requestID: "r", threadID: nil, action: "approve")

            #expect(outcome == .completed(route: .direct, response: .accepted))
            #expect(relay.relayedEnvelope == nil)
        }

        @Test("refreshSnapshot never queues — it reports pendingSync when offline")
        func snapshotDoesNotQueue() async {
            let relay = StubRelay(reachable: false, result: .success(.accepted))
            let direct = StubDirect(reachable: false, result: .success(.accepted))
            let queue = SpyQueue()
            let outcome = await proxy(relay: relay, direct: direct, queue: queue)
                .refreshSnapshot()

            #expect(outcome == .pendingSync)
            #expect(queue.enqueued.isEmpty)
        }
    }
#endif
