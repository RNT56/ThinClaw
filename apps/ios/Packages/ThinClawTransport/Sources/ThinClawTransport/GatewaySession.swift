import Foundation
import ThinClawAPI
import ThinClawCore

/// The mobile client's session over a paired gateway: it owns the live event
/// ``GatewayStream`` and the generated REST client, and exposes the operations
/// the chat surface needs.
///
/// Responsibilities:
/// - lifecycle: ``start()`` opens the event stream, ``shutdown()`` tears it
///   down;
/// - actions: ``send(_:in:)``, ``abort(thread:)`` over REST;
/// - reads: ``threads()``, ``history(thread:before:limit:)`` over REST;
/// - live events: routed per-thread through ``events(in:)``, with
///   `stream_chunk` bursts folded through ``StreamChunkCoalescer`` and flushed
///   on a ~10 Hz cadence so the UI redraws once per tick, not once per token;
/// - connection status: ``connectionState`` mirrors the underlying stream's
///   lifecycle as a domain ``ConnectionState``;
/// - reconcile: after a reconnect the SSE stream has no replay, so
///   ``reconcile(thread:against:)`` refetches the history head and diffs it
///   against the caller's local items into a ``ReconcileResult``.
///
/// The session is a single logical connection; construct one per paired
/// gateway.
public actor GatewaySession {
    /// Coalescer flush cadence: ~10 Hz, matching a comfortable UI redraw rate.
    public static let coalesceInterval: Duration = .milliseconds(100)

    private let client: any APIProtocol
    private let stream: GatewayStream
    private let coalesceInterval: Duration

    /// Per-thread subscribers for routed live events.
    private var eventSubscribers: [ThreadID: [UUID: AsyncStream<AgentEvent>.Continuation]] = [:]
    /// Session-wide subscribers for `approval_needed` events, independent of
    /// which thread (if any) is open. The approvals surface is global — a
    /// pending tool call must reach it even when no chat screen is mounted, and
    /// even when the event carries no `thread_id` — so approvals fan out here
    /// rather than through the per-thread routing that drops thread-less events.
    private var approvalSubscribers: [UUID: AsyncStream<ApprovalRequest>.Continuation] = [:]
    /// Per-thread coalescers for in-flight streaming text.
    private var coalescers: [ThreadID: StreamChunkCoalescer] = [:]

    /// Connection-state fan-out.
    private var connectionContinuations: [UUID: AsyncStream<ConnectionState>.Continuation] = [:]
    private var currentConnectionState: ConnectionState = .idle

    private var pumpTask: Task<Void, Never>?
    private var flushTask: Task<Void, Never>?
    private let clock: any StreamClock

    /// - Parameters:
    ///   - client: The generated gateway REST client (`ThinClawAPI.Client` in
    ///     production; a mock in tests).
    ///   - stream: The live event stream to own.
    ///   - clock: Time seam for the coalescer flush cadence. Defaults to the
    ///     system clock.
    ///   - coalesceInterval: Flush cadence override (tests use a fast one).
    public init(
        client: any APIProtocol,
        stream: GatewayStream,
        clock: any StreamClock = SystemStreamClock(),
        coalesceInterval: Duration = GatewaySession.coalesceInterval
    ) {
        self.client = client
        self.stream = stream
        self.clock = clock
        self.coalesceInterval = coalesceInterval
    }

    // MARK: - Lifecycle

    /// Open the event stream and begin routing events. Idempotent while running.
    public func start() {
        guard pumpTask == nil else { return }
        updateConnectionState(.connecting)
        pumpTask = Task {
            let states = await self.stream.start()
            await self.consume(states)
        }
        flushTask = Task { await self.flushLoop() }
    }

    /// Tear down the stream, flush cadence, and all subscribers.
    public func shutdown() async {
        pumpTask?.cancel()
        pumpTask = nil
        flushTask?.cancel()
        flushTask = nil
        await stream.shutdown()

        for continuations in eventSubscribers.values {
            for continuation in continuations.values { continuation.finish() }
        }
        eventSubscribers.removeAll()
        for continuation in approvalSubscribers.values { continuation.finish() }
        approvalSubscribers.removeAll()
        coalescers.removeAll()

        updateConnectionState(.idle)
        for continuation in connectionContinuations.values { continuation.finish() }
        connectionContinuations.removeAll()
    }

    // MARK: - Connection state

    /// A live stream of ``ConnectionState`` transitions. Replays the current
    /// state immediately on subscribe.
    public var connectionState: AsyncStream<ConnectionState> {
        AsyncStream { continuation in
            let id = UUID()
            continuation.yield(currentConnectionState)
            connectionContinuations[id] = continuation
            continuation.onTermination = { [weak self] _ in
                Task { await self?.dropConnectionSubscriber(id) }
            }
        }
    }

    private func dropConnectionSubscriber(_ id: UUID) {
        connectionContinuations[id] = nil
    }

    // MARK: - Per-thread event routing

    /// Live events for one thread. `stream_chunk` bursts are delivered as
    /// coalesced ``AgentEvent/streamChunk(content:threadID:)`` values carrying
    /// the full accumulated text (not per-token deltas), flushed at the
    /// session's cadence; every other event is forwarded as it arrives.
    public func events(in thread: ThreadID) -> AsyncStream<AgentEvent> {
        AsyncStream { continuation in
            let id = UUID()
            eventSubscribers[thread, default: [:]][id] = continuation
            continuation.onTermination = { [weak self] _ in
                Task { await self?.dropEventSubscriber(thread: thread, id: id) }
            }
        }
    }

    private func dropEventSubscriber(thread: ThreadID, id: UUID) {
        eventSubscribers[thread]?[id] = nil
        if eventSubscribers[thread]?.isEmpty == true { eventSubscribers[thread] = nil }
    }

    // MARK: - Approval event routing

    /// A session-wide stream of live ``ApprovalRequest`` values, one per
    /// `approval_needed` event, regardless of the event's thread. The approvals
    /// surface subscribes here so a pending tool call reaches it whether or not
    /// the owning thread is currently open (or attached at all).
    public func approvalEvents() -> AsyncStream<ApprovalRequest> {
        AsyncStream { continuation in
            let id = UUID()
            approvalSubscribers[id] = continuation
            continuation.onTermination = { [weak self] _ in
                Task { await self?.dropApprovalSubscriber(id) }
            }
        }
    }

    private func dropApprovalSubscriber(_ id: UUID) {
        approvalSubscribers[id] = nil
    }

    // MARK: - Actions

    /// Send a message. Returns the gateway-issued message id.
    public func send(_ text: String, in thread: ThreadID?) async throws -> MessageID {
        do {
            let output = try await client.chatSendHandler(
                .init(body: .json(.init(content: text, threadId: thread?.rawValue))))
            let response = try output.accepted.body.json
            return GatewayMapping.messageID(from: response)
        } catch {
            throw APIError.from(error)
        }
    }

    /// Abort the in-flight turn for a thread.
    public func abort(thread: ThreadID?) async throws {
        do {
            _ = try await client.chatAbortHandler(
                .init(body: .json(.init(threadId: thread?.rawValue))))
        } catch {
            throw APIError.from(error)
        }
    }

    /// Submit a decision for a pending tool approval
    /// (`POST /api/chat/approval`). The caller is responsible for any biometric
    /// gate (D-K3) *before* invoking this — the session performs no gating.
    ///
    /// - Parameters:
    ///   - requestID: The gateway-issued approval request id to decide.
    ///   - decision: Approve, approve-always, or deny.
    ///   - thread: The thread that owns the pending approval, so the agent loop
    ///     resumes the right session. Pass the request's `threadID` when known.
    public func respondToApproval(
        _ requestID: String,
        decision: ApprovalDecision,
        thread: ThreadID? = nil
    ) async throws {
        do {
            _ = try await client.chatApprovalHandler(
                .init(
                    body: .json(
                        .init(
                            action: decision.wire,
                            requestId: requestID,
                            threadId: thread?.rawValue))))
        } catch {
            throw APIError.from(error)
        }
    }

    // MARK: - Reads

    /// Cold-load the currently-pending tool approvals
    /// (`GET /api/chat/approvals`), oldest-first. Best-effort and lossy by
    /// contract (the gateway cache is in-memory), so treat an empty result as
    /// "none known", not "definitely none".
    public func pendingApprovals() async throws -> [ApprovalRequest] {
        do {
            let output = try await client.chatApprovalsHandler(.init())
            let response = try output.ok.body.json
            return GatewayMapping.approvalRequests(from: response)
        } catch {
            throw APIError.from(error)
        }
    }

    /// List the conversation threads visible to this device.
    public func threads() async throws -> [ChatThread] {
        do {
            let output = try await client.chatThreadsHandler(.init())
            let response = try output.ok.body.json
            return GatewayMapping.chatThreads(from: response)
        } catch {
            throw APIError.from(error)
        }
    }

    /// Fetch a page of history for a thread, oldest-first.
    ///
    /// - Parameters:
    ///   - thread: The thread to page.
    ///   - before: Cursor (oldest timestamp already held) for the next older
    ///     page, or `nil` for the head.
    ///   - limit: Max turns to request.
    public func history(
        thread: ThreadID,
        before: Date? = nil,
        limit: Int = 50
    ) async throws -> HistoryPage {
        do {
            let output = try await client.chatHistoryHandler(
                .init(
                    query: .init(
                        threadId: thread.rawValue,
                        limit: limit,
                        before: before?.iso8601)))
            let response = try output.ok.body.json
            return GatewayMapping.historyPage(from: response)
        } catch {
            throw APIError.from(error)
        }
    }

    /// Refetch the history head for a thread and diff it against the caller's
    /// `local` items, returning the repair set needed after a reconnect (the
    /// SSE stream has no replay).
    public func reconcile(
        thread: ThreadID,
        against local: [TimelineItem]
    ) async throws -> ReconcileResult {
        let page = try await history(thread: thread)
        return ReconcileResult.diff(threadID: thread, local: local, server: page.items)
    }

    // MARK: - Stream consumption

    private func consume(_ states: AsyncStream<StreamState>) async {
        for await state in states {
            if Task.isCancelled { break }
            switch state {
            case .connected:
                updateConnectionState(.connected)
            case .reconnecting(let attempt):
                updateConnectionState(.reconnecting(attempt: attempt))
            case .degraded:
                // A degrade does not by itself change the coarse domain state;
                // a following `.reconnecting` will if the connection drops.
                break
            case .event(let event):
                route(event)
            }
        }
    }

    /// Route one decoded event to its thread's subscribers, folding
    /// `stream_chunk` through the per-thread coalescer.
    private func route(_ event: AgentEvent) {
        // Approvals fan out session-wide first (independent of thread routing),
        // so the global approvals surface sees every pending tool call — even
        // one whose event omitted `thread_id`.
        if case .approvalNeeded(let request) = event {
            for continuation in approvalSubscribers.values { continuation.yield(request) }
        }

        guard let thread = event.threadID else {
            // Thread-less events (e.g. heartbeat) are not routed to any UI.
            return
        }
        switch event {
        case .streamChunk:
            var coalescer = coalescers[thread] ?? StreamChunkCoalescer()
            coalescer.reduce(event)
            coalescers[thread] = coalescer
        // The flush loop drains this on the ~10 Hz cadence.
        case .response, .error:
            // Terminal for streaming: flush whatever accumulated as a final
            // coalesced chunk first, then forward the terminal event itself.
            if var coalescer = coalescers[thread] {
                if let update = coalescer.reduce(event) {
                    deliver(.streamChunk(content: update.text, threadID: thread), to: thread)
                }
                coalescers[thread] = nil
            }
            deliver(event, to: thread)
        default:
            deliver(event, to: thread)
        }
    }

    /// Flush every thread's coalescer on the cadence, delivering the latest
    /// accumulated text as a single coalesced chunk.
    private func flushLoop() async {
        while !Task.isCancelled {
            do {
                try await clock.sleep(for: coalesceInterval)
            } catch {
                return
            }
            for thread in coalescers.keys {
                guard var coalescer = coalescers[thread] else { continue }
                if let update = coalescer.drain() {
                    coalescers[thread] = coalescer
                    deliver(.streamChunk(content: update.text, threadID: thread), to: thread)
                }
            }
        }
    }

    private func deliver(_ event: AgentEvent, to thread: ThreadID) {
        guard let subscribers = eventSubscribers[thread] else { return }
        for continuation in subscribers.values { continuation.yield(event) }
    }

    private func updateConnectionState(_ state: ConnectionState) {
        guard state != currentConnectionState else { return }
        currentConnectionState = state
        for continuation in connectionContinuations.values { continuation.yield(state) }
    }
}

extension Date {
    /// RFC 3339 / ISO 8601 rendering with fractional seconds, for history
    /// cursors sent back to the gateway.
    fileprivate var iso8601: String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.string(from: self)
    }
}
