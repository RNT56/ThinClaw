import Foundation
import Testing
import ThinClawCore

@testable import ThinClawTransport

@Suite("GatewayStream reconnect + watchdog")
struct GatewayStreamTests {
    /// A near-instant backoff so reconnect scenarios don't wait on wall clock.
    private static let fastPolicy = ReconnectPolicy(
        baseDelay: .milliseconds(1),
        maxDelay: .milliseconds(5),
        multiplier: 2,
        heartbeatTimeout: .milliseconds(200))

    /// One SSE `response` event for thread `t1`.
    private static let responseSSE =
        "data: {\"type\":\"response\",\"content\":\"hello\",\"thread_id\":\"t1\"}\n\n"

    /// Collect states from `stream` until `predicate` matches or `limit` states
    /// have arrived, whichever comes first. Runs under a wall-clock deadline so
    /// a wedged stream fails fast instead of hanging the suite.
    private func collect(
        from stream: GatewayStream,
        until predicate: @escaping @Sendable ([StreamState]) -> Bool,
        limit: Int = 50,
        deadline: Duration = .seconds(5)
    ) async -> [StreamState] {
        let states = await stream.start()
        let collector = Task { () -> [StreamState] in
            var collected: [StreamState] = []
            for await state in states {
                collected.append(state)
                if predicate(collected) || collected.count >= limit { break }
            }
            return collected
        }
        let watchdog = Task {
            try? await Task.sleep(for: deadline)
            collector.cancel()
        }
        let result = await collector.value
        watchdog.cancel()
        return result
    }

    @Test("emits connected then decoded events for a live stream")
    func connectsAndEmitsEvents() async {
        let provider = ScriptedProvider([.emit(Self.responseSSE)])
        let stream = GatewayStream(
            provider: provider,
            token: { "tcd_test" },
            policy: Self.fastPolicy,
            clock: TestClock())

        let states = await collect(from: stream) { states in
            states.contains { if case .event(.response) = $0 { return true } else { return false } }
        }
        await stream.shutdown()

        #expect(states.contains { if case .connected = $0 { return true } else { return false } })
        let hasResponse = states.contains {
            if case .event(.response(let text, let thread)) = $0 {
                return text == "hello" && thread == ThreadID("t1")
            }
            return false
        }
        #expect(hasResponse)
    }

    @Test("attaches the bearer token to each connection (never the query)")
    func usesTokenPerConnection() async {
        let provider = ScriptedProvider([.emit(Self.responseSSE)])
        let stream = GatewayStream(
            provider: provider,
            token: { "tcd_secret" },
            policy: Self.fastPolicy,
            clock: TestClock())
        _ = await collect(from: stream) { states in
            states.contains { if case .connected = $0 { return true } else { return false } }
        }
        await stream.shutdown()
        #expect(provider.tokensSeen.first == "tcd_secret")
    }

    @Test("a mid-stream drop reconnects and re-establishes connected")
    func dropReconnects() async {
        // First connection errors mid-stream; second connects cleanly.
        let provider = ScriptedProvider([
            .emitThenError(Self.responseSSE),
            .emit(Self.responseSSE),
        ])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            policy: Self.fastPolicy,
            clock: TestClock())

        // Wait until we have seen two `.connected` transitions (before + after
        // the reconnect).
        let states = await collect(from: stream) { states in
            states.filter { if case .connected = $0 { return true } else { return false } }.count >= 2
        }
        await stream.shutdown()

        let connectedCount = states.filter {
            if case .connected = $0 { return true } else { return false }
        }.count
        #expect(connectedCount >= 2)
        #expect(
            states.contains {
                if case .degraded(.transportError) = $0 { return true } else { return false }
            })
        #expect(
            states.contains {
                if case .reconnecting = $0 { return true } else { return false }
            })
        #expect(provider.openCount >= 2)
    }

    @Test("a clean server EOF is surfaced as streamEnded then reconnects")
    func serverEOFReconnects() async {
        let provider = ScriptedProvider([
            .emit(Self.responseSSE),  // ends cleanly (EOF)
            .emit(Self.responseSSE),
        ])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            policy: Self.fastPolicy,
            clock: TestClock())

        let states = await collect(from: stream) { states in
            states.contains {
                if case .degraded(.streamEnded) = $0 { return true } else { return false }
            }
        }
        await stream.shutdown()
        #expect(
            states.contains {
                if case .degraded(.streamEnded) = $0 { return true } else { return false }
            })
    }

    @Test("watchdog trips on silence and forces a reconnect")
    func watchdogFiresOnSilence() async {
        // First connection hangs after the initial event (no heartbeats) so the
        // watchdog window elapses; the second connects.
        let provider = ScriptedProvider([
            .emitThenHang(Self.responseSSE),
            .emit(Self.responseSSE),
        ])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            // Short watchdog so the silence window elapses quickly.
            policy: ReconnectPolicy(
                baseDelay: .milliseconds(1),
                maxDelay: .milliseconds(5),
                heartbeatTimeout: .milliseconds(120)),
            clock: TestClock())

        // Wait until the watchdog tripped AND a second connection came up, so
        // the reconnect has demonstrably happened before we assert.
        let states = await collect(
            from: stream,
            until: { states in
                let tripped = states.contains {
                    if case .degraded(.heartbeatTimeout) = $0 { return true } else { return false }
                }
                let reconnected =
                    states.filter { if case .connected = $0 { return true } else { return false } }
                    .count >= 2
                return tripped && reconnected
            },
            deadline: .seconds(5))
        await stream.shutdown()

        #expect(
            states.contains {
                if case .degraded(.heartbeatTimeout) = $0 { return true } else { return false }
            })
        // It went on to reconnect after the watchdog tripped.
        #expect(provider.openCount >= 2)
    }

    @Test("comment keep-alives keep an idle connection alive past the watchdog")
    func commentKeepAlivesResetWatchdog() async {
        // The gateway's idle keep-alive is an SSE *comment* line (leading `:`),
        // which yields no decoded event. Drip one every 80 ms — comfortably
        // inside the 200 ms watchdog window — for well over two windows, then
        // hang. A watchdog that reset only on decoded events would trip; one
        // that resets on any line (comments included) must not.
        let policy = ReconnectPolicy(
            baseDelay: .milliseconds(1),
            maxDelay: .milliseconds(5),
            heartbeatTimeout: .milliseconds(200))
        let provider = ScriptedProvider([
            .drip(line: ": keep-alive\n", every: .milliseconds(80), count: 8)
        ])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            policy: policy,
            clock: TestClock())

        let states = await stream.start()
        // Observe for ~640 ms (three-plus watchdog windows) while keep-alives
        // drip, then stop and assert the connection was never torn down.
        let collector = Task { () -> [StreamState] in
            var collected: [StreamState] = []
            for await state in states {
                collected.append(state)
            }
            return collected
        }
        try? await Task.sleep(for: .milliseconds(640))
        await stream.shutdown()
        let observed = await collector.value

        // Comment keep-alives produce no decoded events...
        #expect(
            !observed.contains { if case .event = $0 { return true } else { return false } })
        // ...but the watchdog never tripped and no reconnect occurred.
        #expect(
            !observed.contains {
                if case .degraded(.heartbeatTimeout) = $0 { return true } else { return false }
            })
        #expect(provider.openCount == 1)
    }

    @Test("true silence (no keep-alives) still trips the watchdog and reconnects")
    func trueSilenceStillReconnects() async {
        // A connection that emits nothing at all (not even comments) must still
        // be torn down by the watchdog and reconnected.
        let provider = ScriptedProvider([
            .emitThenHang(""),  // opens, then dead silence
            .emit(Self.responseSSE),
        ])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            policy: ReconnectPolicy(
                baseDelay: .milliseconds(1),
                maxDelay: .milliseconds(5),
                heartbeatTimeout: .milliseconds(120)),
            clock: TestClock())

        let states = await collect(
            from: stream,
            until: { states in
                states.contains {
                    if case .degraded(.heartbeatTimeout) = $0 { return true } else { return false }
                }
                    && states.filter {
                        if case .connected = $0 { return true } else { return false }
                    }.count >= 2
            },
            deadline: .seconds(5))
        await stream.shutdown()

        #expect(
            states.contains {
                if case .degraded(.heartbeatTimeout) = $0 { return true } else { return false }
            })
        #expect(provider.openCount >= 2)
    }

    @Test("a failed connect backs off and retries")
    func failedConnectRetries() async {
        let provider = ScriptedProvider([
            .failConnect,
            .emit(Self.responseSSE),
        ])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            policy: Self.fastPolicy,
            clock: TestClock())

        let states = await collect(from: stream) { states in
            states.contains { if case .connected = $0 { return true } else { return false } }
        }
        await stream.shutdown()
        #expect(states.contains { if case .reconnecting = $0 { return true } else { return false } })
        #expect(states.contains { if case .connected = $0 { return true } else { return false } })
    }

    @Test("shutdown finishes the stream")
    func shutdownFinishes() async {
        let provider = ScriptedProvider([.emitThenHang(Self.responseSSE)])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            policy: Self.fastPolicy,
            clock: TestClock())
        let states = await stream.start()
        let collector = Task { () -> Int in
            var count = 0
            for await _ in states { count += 1 }
            return count
        }
        // Give it a moment to connect, then shut down; the stream must finish.
        try? await Task.sleep(for: .milliseconds(50))
        await stream.shutdown()
        let count = await collector.value
        #expect(count >= 1)
    }

    @Test("a malformed event payload degrades without dropping the connection")
    func malformedEventDoesNotDrop() async {
        let malformed =
            "data: {\"type\":\"response\"}\n\n"  // missing required `content`
        let good = Self.responseSSE
        let provider = ScriptedProvider([.emit(malformed + good)])
        let stream = GatewayStream(
            provider: provider,
            token: { "t" },
            policy: Self.fastPolicy,
            clock: TestClock())

        let states = await collect(from: stream) { states in
            states.contains { if case .event(.response) = $0 { return true } else { return false } }
        }
        await stream.shutdown()

        #expect(
            states.contains {
                if case .degraded(.malformedEvent) = $0 { return true } else { return false }
            })
        // The good event after the malformed one still came through on the same
        // connection (openCount stays 1).
        #expect(provider.openCount == 1)
    }
}
