import Foundation
import ThinClawCore

/// One observation from the live gateway event stream.
///
/// The stream is a supervised, self-reconnecting sequence: it interleaves
/// lifecycle transitions (``connected``, ``reconnecting(attempt:)``,
/// ``degraded(reason:)``) with decoded agent ``event(_:)`` values. Consumers
/// render events and drive their connection-status UI from the lifecycle
/// cases; ``AgentEvent/heartbeat`` is delivered like any other event. The
/// watchdog resets on every completed SSE line, including comment keep-alives.
public enum StreamState: Sendable {
    /// A fresh connection is established and events are flowing.
    case connected
    /// A decoded agent event.
    case event(AgentEvent)
    /// The connection dropped or a decode failed; a retry is scheduled with
    /// this 0-based consecutive-failure count.
    case reconnecting(attempt: Int)
    /// A non-fatal condition worth surfacing (e.g. watchdog silence, a bad
    /// event payload) that does not by itself tear the connection down.
    case degraded(reason: DegradeReason)
}

/// Why the stream reported ``StreamState/degraded(reason:)``.
public enum DegradeReason: Hashable, Sendable {
    /// No stream activity (event or comment keep-alive) arrived within the
    /// watchdog window; the connection is presumed dead and will reconnect.
    case heartbeatTimeout
    /// The byte stream ended cleanly from the server side (EOF); will reconnect.
    case streamEnded
    /// The transport threw while reading (connection lost, timeout, TLS).
    case transportError
    /// A single event payload failed to decode; skipped, connection kept.
    case malformedEvent
}

/// A supervised, self-reconnecting connection to the gateway SSE event stream
/// (`GET {base}/api/chat/events`).
///
/// Responsibilities:
/// - open the byte stream through an injected ``ByteStreamProvider`` (the sole
///   networking seam — production wraps `URLSession.bytes`, tests replay
///   fixtures);
/// - decode bytes into ``AgentEvent`` via ``SSEClient`` + ``AgentEventDecoder``
///   and publish them as ``StreamState/event(_:)``;
/// - reconnect on drop with full-jitter exponential backoff from
///   ``ReconnectPolicy``;
/// - run a heartbeat watchdog that forces a reconnect after the policy's
///   silence window elapses with no stream activity (the gateway heartbeats
///   well inside it, so crossing it means multiple keep-alives were missed);
/// - shut down cleanly on ``shutdown()`` or consumer cancellation.
///
/// The reconnect attempt counter resets to zero on each successful connect.
public actor GatewayStream {
    private let provider: any ByteStreamProvider
    private let token: @Sendable () -> String?
    private let policy: ReconnectPolicy
    private let clock: any StreamClock
    private var rng: any RandomNumberGenerator

    private var supervisor: Task<Void, Never>?
    private var continuation: AsyncStream<StreamState>.Continuation?

    /// - Parameters:
    ///   - provider: Opens authenticated event byte streams. Inject a
    ///     `URLSessionByteStreamProvider` in production, a fixture provider in
    ///     tests.
    ///   - token: Supplies the current device bearer token per attempt, or
    ///     `nil` if unavailable (the attempt is skipped and retried).
    ///   - policy: Backoff + watchdog constants. Defaults to
    ///     ``ReconnectPolicy/default``.
    ///   - clock: Time seam for backoff sleeps and the watchdog. Defaults to
    ///     the system continuous clock.
    ///   - randomNumberGenerator: Jitter source. Defaults to the system RNG;
    ///     tests inject a seeded generator for deterministic backoff.
    public init(
        provider: any ByteStreamProvider,
        token: @escaping @Sendable () -> String?,
        policy: ReconnectPolicy = .default,
        clock: any StreamClock = SystemStreamClock(),
        randomNumberGenerator: any RandomNumberGenerator = SystemRandomNumberGenerator()
    ) {
        self.provider = provider
        self.token = token
        self.policy = policy
        self.clock = clock
        self.rng = randomNumberGenerator
    }

    /// Begin streaming. The returned ``AsyncStream`` yields lifecycle
    /// transitions and decoded events until ``shutdown()`` is called or the
    /// consumer stops iterating. Calling `start()` more than once returns a
    /// stream that finishes immediately.
    public func start() -> AsyncStream<StreamState> {
        guard supervisor == nil else {
            return AsyncStream { $0.finish() }
        }
        let (stream, continuation) = AsyncStream<StreamState>.makeStream()
        self.continuation = continuation
        continuation.onTermination = { [weak self] _ in
            Task { await self?.shutdown() }
        }
        supervisor = Task { await self.superviseLoop() }
        return stream
    }

    /// Stop streaming and release resources. Idempotent.
    public func shutdown() {
        supervisor?.cancel()
        supervisor = nil
        continuation?.finish()
        continuation = nil
    }

    // MARK: - Supervision

    private func superviseLoop() async {
        var attempt = 0
        while !Task.isCancelled {
            let token = self.token()
            guard let token else {
                // No credential yet; back off and retry rather than spin.
                emit(.reconnecting(attempt: attempt))
                if await backoff(attempt: attempt) == false { return }
                attempt += 1
                continue
            }

            let outcome = await runConnection(token: token, onConnect: { attempt = 0 })

            if Task.isCancelled { return }
            switch outcome {
            case .clean:
                // EOF from the server; treat like a drop and reconnect.
                emit(.degraded(reason: .streamEnded))
            case .heartbeatTimeout:
                emit(.degraded(reason: .heartbeatTimeout))
            case .transportError:
                emit(.degraded(reason: .transportError))
            case .connectFailed:
                break
            }

            emit(.reconnecting(attempt: attempt))
            if await backoff(attempt: attempt) == false { return }
            attempt += 1
        }
    }

    /// The reason one connection attempt ended.
    private enum ConnectionOutcome {
        case clean
        case heartbeatTimeout
        case transportError
        case connectFailed
    }

    /// Open one connection and pump it until it ends. `onConnect` fires once,
    /// after the byte stream opens, so the caller can reset the attempt count.
    ///
    /// The pump and the heartbeat watchdog run as two racing children in a
    /// task group; the first to produce an outcome wins and the group cancels
    /// the other. The pump records the arrival time of every completed SSE
    /// line on the actor (`lastEventAt`); the watchdog re-reads it and
    /// re-sleeps until the silence window is exhausted.
    private func runConnection(
        token: String,
        onConnect: () -> Void
    ) async -> ConnectionOutcome {
        let byteStream: ByteStream
        do {
            byteStream = try await provider.open(token: token)
        } catch {
            return .connectFailed
        }
        if Task.isCancelled { return .clean }

        onConnect()
        emit(.connected)
        lastEventAt = clock.nowSeconds()

        let watchdogSeconds = policy.heartbeatTimeout.timeIntervalValue

        return await withTaskGroup(of: ConnectionOutcome.self) { group in
            group.addTask { await self.pump(byteStream) }
            group.addTask { await self.runWatchdog(windowSeconds: watchdogSeconds) }
            // First child to finish decides the outcome; cancel the other.
            let outcome = await group.next() ?? .clean
            group.cancelAll()
            return outcome
        }
    }

    /// Read the byte stream, decode events, and emit them until it ends.
    private func pump(_ byteStream: ByteStream) async -> ConnectionOutcome {
        let client = SSEClient()
        let decoder = AgentEventDecoder()
        // Reset the watchdog on *any* stream activity — including comment
        // keep-alives that yield no event. The gateway's idle keep-alive is an
        // SSE comment line, so a watchdog that only reset on decoded events
        // would needlessly tear down a healthy but quiet connection.
        let onActivity: @Sendable () async -> Void = { [weak self] in
            await self?.markActivity()
        }
        do {
            for try await sse in await client.events(from: byteStream, onActivity: onActivity) {
                if Task.isCancelled { return .clean }
                lastEventAt = clock.nowSeconds()
                do {
                    let event = try decoder.decode(sse)
                    emit(.event(event))
                } catch {
                    // A single bad payload must not kill the stream.
                    emit(.degraded(reason: .malformedEvent))
                }
            }
            // Stream ended without throwing → server closed the connection.
            return .clean
        } catch {
            if Task.isCancelled { return .clean }
            return .transportError
        }
    }

    /// Trip once no stream activity has arrived for `windowSeconds`.
    private func runWatchdog(windowSeconds: Double) async -> ConnectionOutcome {
        while !Task.isCancelled {
            let elapsed = clock.nowSeconds() - lastEventAt
            let remaining = windowSeconds - elapsed
            if remaining <= 0 { return .heartbeatTimeout }
            do {
                try await clock.sleep(for: .seconds(remaining))
            } catch {
                return .clean
            }
        }
        return .clean
    }

    /// Timestamp (seconds) of the last observed stream activity (a decoded
    /// event *or* a bare comment keep-alive), shared between pump and watchdog.
    private var lastEventAt: Double = 0

    /// Record stream activity now. Called from the SSE layer's `onActivity`
    /// hook for every completed line — comment keep-alives included — so the
    /// watchdog treats an idle-but-heartbeating connection as alive.
    private func markActivity() {
        lastEventAt = clock.nowSeconds()
    }

    /// Sleep the full-jitter backoff for `attempt`. Returns `false` if the
    /// sleep was cancelled (shutdown), so the loop can exit.
    private func backoff(attempt: Int) async -> Bool {
        let delay = policy.delay(forAttempt: attempt, using: &rng)
        guard delay > .zero else { return !Task.isCancelled }
        do {
            try await clock.sleep(for: delay)
            return true
        } catch {
            return false
        }
    }

    private func emit(_ state: StreamState) {
        continuation?.yield(state)
    }
}
