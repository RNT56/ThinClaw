import Foundation
import Testing
import ThinClawAPI
import ThinClawCore

@testable import ThinClawTransport

@Suite("GatewaySession")
struct GatewaySessionTests {
    private static let fastPolicy = ReconnectPolicy(
        baseDelay: .milliseconds(1),
        maxDelay: .milliseconds(5),
        heartbeatTimeout: .milliseconds(500))

    private func makeSession(
        client: MockGatewayClient,
        connections: [ScriptedProvider.Connection]
    ) -> (GatewaySession, ScriptedProvider) {
        let provider = ScriptedProvider(connections)
        let stream = GatewayStream(
            provider: provider,
            token: { "tcd_test" },
            policy: Self.fastPolicy,
            clock: TestClock())
        let session = GatewaySession(
            client: client,
            stream: stream,
            clock: TestClock(),
            coalesceInterval: .milliseconds(20))
        return (session, provider)
    }

    // MARK: - Actions

    @Test("send forwards content and thread and returns the gateway message id")
    func sendPath() async throws {
        let client = MockGatewayClient()
        client.sendResponse = .init(messageId: "srv-7", status: "accepted")
        let (session, _) = makeSession(client: client, connections: [])

        let id = try await session.send("hello there", in: ThreadID("t1"))
        #expect(id == MessageID("srv-7"))
        #expect(client.sentMessages.count == 1)
        #expect(client.sentMessages[0].content == "hello there")
        #expect(client.sentMessages[0].thread == "t1")
    }

    @Test("abort forwards the thread id")
    func abortPath() async throws {
        let client = MockGatewayClient()
        let (session, _) = makeSession(client: client, connections: [])
        try await session.abort(thread: ThreadID("t9"))
        #expect(client.abortedThreads == ["t9"])
    }

    @Test("threads maps the gateway listing")
    func threadsPath() async throws {
        let client = MockGatewayClient()
        client.threadsResponse = .init(threads: [
            .init(
                createdAt: "2026-07-04T10:00:00Z", id: "t1", state: "idle",
                title: "One", turnCount: 1, updatedAt: "2026-07-04T10:00:00Z")
        ])
        let (session, _) = makeSession(client: client, connections: [])
        let threads = try await session.threads()
        #expect(threads.map(\.id) == [ThreadID("t1")])
        #expect(threads[0].title == "One")
    }

    @Test("history forwards thread, before, and limit as query parameters")
    func historyForwardsQueryParams() async throws {
        let client = MockGatewayClient()
        let (session, _) = makeSession(client: client, connections: [])

        let before = GatewayMapping.date("2026-07-04T10:00:00.000Z")
        _ = try await session.history(thread: ThreadID("t7"), before: before, limit: 25)

        #expect(client.historyQueries.count == 1)
        let query = client.historyQueries[0]
        #expect(query.threadId == "t7")
        #expect(query.limit == 25)
        // The cursor is serialized as an RFC 3339 timestamp with fractional
        // seconds, and must round-trip to the same instant.
        #expect(query.before == "2026-07-04T10:00:00.000Z")
    }

    @Test("history omits the before cursor when paging the head")
    func historyOmitsCursorForHead() async throws {
        let client = MockGatewayClient()
        let (session, _) = makeSession(client: client, connections: [])

        _ = try await session.history(thread: ThreadID("t7"))

        #expect(client.historyQueries.count == 1)
        // `before` must be nil (absent) for the head page, not an empty string,
        // so the gateway does not treat "" as a cursor.
        #expect(client.historyQueries[0].before == nil)
        #expect(client.historyQueries[0].threadId == "t7")
        #expect(client.historyQueries[0].limit == 50)
    }

    // MARK: - Reconcile

    @Test("reconcile fetches the history head and diffs against local items")
    func reconcileInvokesHistory() async throws {
        let client = MockGatewayClient()
        // Server head has the user turn plus an agent response.
        client.historyResponse = .init(
            threadId: "t1",
            turns: [
                .init(
                    response: "server answer",
                    startedAt: "2026-07-04T10:00:00Z",
                    state: "completed",
                    toolCalls: [],
                    turnNumber: 1,
                    userInput: "q")
            ])
        let (session, _) = makeSession(client: client, connections: [])

        // Local view has only the user row (matching id) — the agent row is
        // missing, so reconcile should upsert it.
        let userID = MessageID("t1#turn1#user")
        let local = [
            TimelineItem(
                id: userID,
                threadID: ThreadID("t1"),
                timestamp: GatewayMapping.date("2026-07-04T10:00:00Z"),
                kind: .userMessage(text: "q"))
        ]
        let result = try await session.reconcile(thread: ThreadID("t1"), against: local)
        #expect(result.threadID == ThreadID("t1"))
        #expect(result.upserted.map(\.id) == [MessageID("t1#turn1#agent")])
        #expect(result.removed.isEmpty)
    }

    // MARK: - Connection state

    @Test("connectionState reports connecting then connected")
    func connectionStateTransitions() async {
        let client = MockGatewayClient()
        let responseSSE =
            "data: {\"type\":\"response\",\"content\":\"hi\",\"thread_id\":\"t1\"}\n\n"
        let (session, _) = makeSession(client: client, connections: [.emitThenHang(responseSSE)])

        let stateStream = await session.connectionState
        let collector = Task { () -> [ConnectionState] in
            var seen: [ConnectionState] = []
            for await state in stateStream {
                seen.append(state)
                if state == .connected { break }
            }
            return seen
        }
        await session.start()

        let deadline = Task {
            try? await Task.sleep(for: .seconds(5))
            collector.cancel()
        }
        let states = await collector.value
        deadline.cancel()
        await session.shutdown()

        #expect(states.contains(.connecting))
        #expect(states.contains(.connected))
    }

    // MARK: - Per-thread event routing + coalescing

    @Test("stream chunks are coalesced and routed to the owning thread")
    func routesAndCoalescesChunks() async {
        let client = MockGatewayClient()
        // Three chunks for t1, then a final response — all on one connection
        // that hangs afterward so the session stays up.
        let sse =
            "data: {\"type\":\"stream_chunk\",\"content\":\"Hel\",\"thread_id\":\"t1\"}\n\n"
            + "data: {\"type\":\"stream_chunk\",\"content\":\"lo \",\"thread_id\":\"t1\"}\n\n"
            + "data: {\"type\":\"stream_chunk\",\"content\":\"world\",\"thread_id\":\"t1\"}\n\n"
            + "data: {\"type\":\"response\",\"content\":\"Hello world\",\"thread_id\":\"t1\"}\n\n"
        let (session, _) = makeSession(client: client, connections: [.emitThenHang(sse)])

        let events = await session.events(in: ThreadID("t1"))
        let collector = Task { () -> [AgentEvent] in
            var seen: [AgentEvent] = []
            for await event in events {
                seen.append(event)
                if case .response = event { break }
            }
            return seen
        }
        await session.start()

        let deadline = Task {
            try? await Task.sleep(for: .seconds(5))
            collector.cancel()
        }
        let seen = await collector.value
        deadline.cancel()
        await session.shutdown()

        // The final coalesced chunk before the response must carry the full
        // accumulated text, and the terminal response must arrive.
        let lastChunk = seen.last {
            if case .streamChunk = $0 { return true } else { return false }
        }
        if case .streamChunk(let text, let thread) = lastChunk {
            #expect(text == "Hello world")
            #expect(thread == ThreadID("t1"))
        } else {
            Issue.record("expected at least one coalesced stream chunk")
        }
        #expect(seen.contains { if case .response = $0 { return true } else { return false } })
    }

    @Test("events for one thread are not delivered to another thread's subscriber")
    func routingIsThreadScoped() async {
        let client = MockGatewayClient()
        let sse =
            "data: {\"type\":\"response\",\"content\":\"for t1\",\"thread_id\":\"t1\"}\n\n"
        let (session, _) = makeSession(client: client, connections: [.emitThenHang(sse)])

        let otherEvents = await session.events(in: ThreadID("t2"))
        let otherCollector = Task { () -> Int in
            var count = 0
            for await _ in otherEvents { count += 1 }
            return count
        }
        let t1Events = await session.events(in: ThreadID("t1"))
        let t1Collector = Task { () -> Bool in
            for await event in t1Events {
                if case .response = event { return true }
            }
            return false
        }
        await session.start()

        let gotT1 = await withTaskGroup(of: Bool.self) { group in
            group.addTask { await t1Collector.value }
            group.addTask {
                try? await Task.sleep(for: .seconds(5))
                t1Collector.cancel()
                return false
            }
            let result = await group.next() ?? false
            group.cancelAll()
            return result
        }
        await session.shutdown()
        let otherCount = await otherCollector.value

        #expect(gotT1)
        #expect(otherCount == 0)
    }
}
