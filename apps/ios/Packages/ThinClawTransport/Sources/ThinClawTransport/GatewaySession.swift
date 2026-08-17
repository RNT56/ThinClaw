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
    public static let presenceTTLSeconds: Int64 = 45
    public static let presenceRefreshInterval: Duration = .seconds(20)

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
    /// Scope-specific, privacy-filtered aggregate presence updates.
    private struct PresenceSubscriber {
        let scope: PresenceScope
        let continuation: AsyncStream<PresenceUpdate>.Continuation
        var reconciliationID: UUID?
        var bufferedEvents: [PresenceEvent]
    }
    private var presenceSubscribers: [UUID: PresenceSubscriber] = [:]
    /// Per-thread coalescers for in-flight streaming text.
    private var coalescers: [ThreadID: StreamChunkCoalescer] = [:]

    /// Connection-state fan-out.
    private var connectionContinuations: [UUID: AsyncStream<ConnectionState>.Continuation] = [:]
    private var currentConnectionState: ConnectionState = .idle

    private var pumpTask: Task<Void, Never>?
    private var flushTask: Task<Void, Never>?
    private var presenceTask: Task<Void, Never>?
    private var presenceReconcileTasks: [UUID: Task<Void, Never>] = [:]
    private let presenceSessionID = UUID()
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
        presenceTask = Task { await self.presenceLoop() }
    }

    /// Pause network work while preserving subscribers for a later foreground
    /// `start()`. This is the reversible scene-lifecycle path.
    public func suspend() async {
        let pump = pumpTask
        pumpTask = nil
        let flush = flushTask
        flushTask = nil
        let presence = presenceTask
        presenceTask = nil
        let presenceReconciles = Array(presenceReconcileTasks.values)
        presenceReconcileTasks.removeAll()
        let wasRunning = pump != nil || flush != nil || presence != nil
        pump?.cancel()
        flush?.cancel()
        presence?.cancel()
        for task in presenceReconciles { task.cancel() }
        await stream.shutdown()
        // Wait for every task that can publish before the final DELETE. Task
        // cancellation alone does not guarantee an in-flight HTTP PUT ended.
        await pump?.value
        await flush?.value
        await presence?.value
        for task in presenceReconciles { await task.value }
        if wasRunning { try? await clearPresence() }

        resetPresenceReconciliation()
        coalescers.removeAll()
        updateConnectionState(.idle)
    }

    /// Permanently tear down the session and finish every subscriber. Use
    /// ``suspend()`` for a background/foreground transition.
    public func shutdown() async {
        await suspend()

        for continuations in eventSubscribers.values {
            for continuation in continuations.values { continuation.finish() }
        }
        eventSubscribers.removeAll()
        for continuation in approvalSubscribers.values { continuation.finish() }
        approvalSubscribers.removeAll()
        for subscriber in presenceSubscribers.values { subscriber.continuation.finish() }
        presenceSubscribers.removeAll()

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

    // MARK: - Presence event routing

    /// Principal/thread aggregates visible to this authenticated device. The
    /// wire contract never contains individual session or principal IDs.
    public func presenceEvents(thread: ThreadID? = nil) -> AsyncStream<PresenceUpdate> {
        AsyncStream { continuation in
            let id = UUID()
            let scope = thread.map(PresenceScope.thread) ?? .principal
            presenceSubscribers[id] = PresenceSubscriber(
                scope: scope,
                continuation: continuation,
                reconciliationID: nil,
                bufferedEvents: [])
            continuation.onTermination = { [weak self] _ in
                Task { await self?.dropPresenceSubscriber(id) }
            }
            // Before the stream is active, the next `.connected` transition
            // owns the initial snapshot. A late subscriber to an already-live
            // session still receives an immediate cold load.
            if currentConnectionState == .connected {
                Task { [weak self] in
                    await self?.reconcilePresenceScopes([scope])
                }
            }
        }
    }

    private func dropPresenceSubscriber(_ id: UUID) {
        presenceSubscribers[id] = nil
    }

    private func reconcilePresenceScopes(_ scopes: Set<PresenceScope>) async {
        let tickets = beginPresenceReconciliation(scopes)
        await completePresenceReconciliation(tickets)
    }

    private func beginPresenceReconciliation(
        _ scopes: Set<PresenceScope>
    ) -> [PresenceScope: UUID] {
        let tickets = Dictionary(uniqueKeysWithValues: scopes.map { ($0, UUID()) })
        let subscriberIDs = presenceSubscribers.compactMap { id, subscriber in
            tickets[subscriber.scope] == nil ? nil : id
        }
        for id in subscriberIDs {
            guard var subscriber = presenceSubscribers[id] else { continue }
            if subscriber.reconciliationID == nil {
                subscriber.bufferedEvents.removeAll(keepingCapacity: true)
            }
            subscriber.reconciliationID = tickets[subscriber.scope]
            presenceSubscribers[id] = subscriber
        }
        return tickets
    }

    private func completePresenceReconciliation(_ tickets: [PresenceScope: UUID]) async {
        for (scope, ticket) in tickets {
            let snapshot = try? await presenceSnapshot(thread: scope.threadID)
            let matchingIDs = presenceSubscribers.compactMap { id, subscriber in
                subscriber.scope == scope && subscriber.reconciliationID == ticket ? id : nil
            }
            for id in matchingIDs {
                guard var subscriber = presenceSubscribers[id] else { continue }
                let buffered = subscriber.bufferedEvents
                subscriber.reconciliationID = nil
                subscriber.bufferedEvents.removeAll(keepingCapacity: true)
                presenceSubscribers[id] = subscriber
                if let snapshot {
                    subscriber.continuation.yield(.snapshot(snapshot))
                }
                for event in buffered {
                    subscriber.continuation.yield(.event(event))
                }
            }
        }
    }

    private func presenceReconciliationFinished(_ id: UUID) {
        presenceReconcileTasks[id] = nil
    }

    /// A fresh transport connection gets an authoritative new snapshot. Any
    /// event buffered for the previous connection predates that snapshot and
    /// must not be replayed afterward as if it were newer.
    private func resetPresenceReconciliation() {
        for id in Array(presenceSubscribers.keys) {
            guard var subscriber = presenceSubscribers[id] else { continue }
            subscriber.reconciliationID = nil
            subscriber.bufferedEvents.removeAll(keepingCapacity: true)
            presenceSubscribers[id] = subscriber
        }
    }

    // MARK: - Actions

    /// Send a message. Returns the gateway-issued message id.
    public func send(
        _ text: String,
        in thread: ThreadID?,
        clientMessageID: UUID? = nil
    ) async throws -> MessageID {
        do {
            let output = try await client.chatSendHandler(
                .init(
                    body: .json(
                        .init(
                            clientMessageId: clientMessageID?.uuidString,
                            content: text,
                            threadId: thread?.rawValue))))
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

    /// Publish a bounded iOS presence state. `typing` must carry an owned
    /// thread; the gateway validates both the TTL and ownership again.
    public func publishPresence(
        _ state: PresenceState,
        thread: ThreadID? = nil,
        ttlSeconds: Int64 = GatewaySession.presenceTTLSeconds
    ) async throws {
        do {
            let output = try await client.presencePublishHandler(
                .init(
                    path: .init(sessionId: presenceSessionID.uuidString),
                    body: .json(
                        .init(
                            state: .init(rawValue: state.rawValue)!,
                            surface: .ios,
                            threadId: thread?.rawValue,
                            ttlSeconds: ttlSeconds))))
            _ = try output.ok
        } catch {
            throw APIError.from(error)
        }
    }

    /// Idempotently remove this session's presence lease.
    public func clearPresence() async throws {
        do {
            let output = try await client.presenceClearHandler(
                .init(path: .init(sessionId: presenceSessionID.uuidString)))
            _ = try output.ok
        } catch {
            throw APIError.from(error)
        }
    }

    /// Fetch an authoritative principal- or thread-scoped aggregate. This is
    /// the repair path for initial subscription and every SSE reconnect.
    public func presenceSnapshot(thread: ThreadID? = nil) async throws -> PresenceSnapshot {
        do {
            let output = try await client.presenceSnapshotHandler(
                .init(query: .init(threadId: thread?.rawValue)))
            let generated = try output.ok.body.json
            let encoded = try JSONEncoder().encode(generated)
            let wire = try JSONDecoder().decode(PresenceSnapshotWire.self, from: encoded)
            return try wire.domain(scope: thread.map(PresenceScope.thread) ?? .principal)
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
        try await threadListing().threads
    }

    /// List the conversation threads visible to this device, surfacing the
    /// pinned assistant thread (`assistant_thread`) separately from the regular
    /// `threads`. Prefer this over ``threads()`` when the caller needs the
    /// pinned assistant thread (e.g. to pick a default landing thread).
    public func threadListing() async throws -> ThreadListing {
        do {
            let output = try await client.chatThreadsHandler(.init())
            let response = try output.ok.body.json
            return GatewayMapping.threadListing(from: response)
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
                for task in presenceReconcileTasks.values { task.cancel() }
                resetPresenceReconciliation()
                let scopes = Set(presenceSubscribers.values.map(\.scope))
                let tickets = beginPresenceReconciliation(scopes)
                let reconciliationID = UUID()
                presenceReconcileTasks[reconciliationID] = Task { [weak self] in
                    guard let self else { return }
                    try? await self.publishPresence(.online)
                    if !Task.isCancelled {
                        await self.completePresenceReconciliation(tickets)
                    }
                    await self.presenceReconciliationFinished(reconciliationID)
                }
            case .reconnecting(let attempt):
                for task in presenceReconcileTasks.values { task.cancel() }
                resetPresenceReconciliation()
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
        if case .presence(let presence) = event {
            let matchingIDs = presenceSubscribers.compactMap { id, subscriber in
                subscriber.scope == presence.presence.scope ? id : nil
            }
            for id in matchingIDs {
                guard var subscriber = presenceSubscribers[id] else { continue }
                if subscriber.reconciliationID != nil {
                    subscriber.bufferedEvents.append(presence)
                    presenceSubscribers[id] = subscriber
                } else {
                    subscriber.continuation.yield(.event(presence))
                }
            }
            return
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

    private func presenceLoop() async {
        while !Task.isCancelled {
            do {
                try await clock.sleep(for: Self.presenceRefreshInterval)
            } catch {
                return
            }
            if currentConnectionState == .connected {
                try? await publishPresence(.online)
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

private struct PresenceSnapshotWire: Decodable {
    let presences: [Aggregate]
    let serverTime: String

    enum CodingKeys: String, CodingKey {
        case presences
        case serverTime = "server_time"
    }

    struct Aggregate: Decodable {
        let actorID: String
        let scope: Scope
        let state: PresenceState
        let surfaces: [PresenceSurface]
        let sessionCount: Int
        let updatedAt: String
        let expiresAt: String

        enum CodingKeys: String, CodingKey {
            case actorID = "actor_id"
            case scope, state, surfaces
            case sessionCount = "session_count"
            case updatedAt = "updated_at"
            case expiresAt = "expires_at"
        }
    }

    struct Scope: Decodable {
        let kind: String
        let threadID: String?

        enum CodingKeys: String, CodingKey {
            case kind
            case threadID = "thread_id"
        }
    }

    func domain(scope snapshotScope: PresenceScope) throws -> PresenceSnapshot {
        let mapped = try presences.map { wire -> PresenceAggregate in
            let scope: PresenceScope
            switch wire.scope.kind {
            case "principal":
                scope = .principal
            case "thread":
                guard let threadID = wire.scope.threadID else {
                    throw PresenceSnapshotMappingError.invalidThreadScope
                }
                scope = .thread(ThreadID(threadID))
            default:
                throw PresenceSnapshotMappingError.unknownScope
            }
            guard scope == snapshotScope else {
                throw PresenceSnapshotMappingError.scopeMismatch
            }
            return PresenceAggregate(
                actorID: wire.actorID,
                scope: scope,
                state: wire.state,
                surfaces: wire.surfaces,
                sessionCount: wire.sessionCount,
                updatedAt: wire.updatedAt,
                expiresAt: wire.expiresAt)
        }
        return PresenceSnapshot(scope: snapshotScope, presences: mapped, serverTime: serverTime)
    }
}

private enum PresenceSnapshotMappingError: Error {
    case invalidThreadScope
    case unknownScope
    case scopeMismatch
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
