import Foundation
import SwiftUI
import ThinClawAPI
import ThinClawCore
import ThinClawDesign
import ThinClawPersistence
import ThinClawTransport

/// The chat surface's view model for one thread: it hydrates from the local
/// ``TranscriptStoring`` cache for instant first paint, folds the live
/// ``GatewaySession`` event stream through the pure ``ChatTimelineReducer``,
/// sends messages (optimistically, with an offline outbox and a 429 cooldown),
/// pages history, and reconciles the transcript after a reconnect.
///
/// The reducer, the cooldown model, and the reconcile diff are all pure types
/// in ThinClawCore, unit-tested on macOS. This store is the thin iOS-only shell
/// that wires them to the session, the store, and SwiftUI observation.
@MainActor
@Observable
public final class ChatStore {
    /// The rows currently rendered, oldest-first.
    public private(set) var timeline: [TimelineItem] = []
    /// Connection pill state, mirrored from the session's ``ConnectionState``.
    public private(set) var connection: StatusPill.Status = .offline
    /// True while the connection is not live, so the UI can show the offline /
    /// degraded banner with a manual retry.
    public private(set) var isOffline: Bool = true
    /// True while a history page is loading (drives the scroll-top spinner).
    public private(set) var isLoadingHistory: Bool = false
    /// Whether older history remains to page in.
    public private(set) var hasMoreHistory: Bool = true
    /// Seconds remaining on a 429 composer cooldown (0 when the composer is
    /// free). Recomputed on demand from ``cooldown``.
    public private(set) var cooldownRemaining: TimeInterval = 0
    /// The composer draft text.
    public var draft: String = ""

    /// Whether the composer's send button should be disabled.
    public var isSendDisabled: Bool {
        draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            || cooldownRemaining > 0
    }

    // MARK: - Dependencies

    private let threadID: ThreadID
    private let session: GatewaySession
    private let store: any TranscriptStoring
    private let now: @Sendable () -> Date

    // MARK: - Pure state models

    private var reducer: ChatTimelineReducer
    private var cooldown = ComposerCooldown()

    // MARK: - Live tasks

    private var eventTask: Task<Void, Never>?
    private var connectionTask: Task<Void, Never>?
    /// Oldest timestamp currently held, used as the `before:` history cursor.
    private var oldestTimestamp: Date?
    /// Original text for each failed row, so ``retry(rowID:)`` can resend
    /// without the user retyping.
    private var retryTexts: [MessageID: String] = [:]
    /// Whether we have ever been live, so the first `connected` is a cold start
    /// and later ones are reconnects (which trigger reconcile + outbox flush).
    private var hasConnectedBefore = false
    /// Guards against concurrent ``flushOutbox`` runs when connection flapping
    /// fires two `.connected` transitions in quick succession.
    private var isFlushing = false

    public init(
        threadID: ThreadID,
        session: GatewaySession,
        store: any TranscriptStoring,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.threadID = threadID
        self.session = session
        self.store = store
        self.now = now
        self.reducer = ChatTimelineReducer(threadID: threadID, now: now)
    }

    // MARK: - Lifecycle

    /// Open the thread: hydrate the cache, then subscribe to live events and
    /// connection state. Idempotent.
    public func open() async {
        await hydrateFromCache()
        subscribeToEvents()
        subscribeToConnection()
    }

    /// Stop observing (e.g. when the view disappears).
    public func close() {
        eventTask?.cancel()
        eventTask = nil
        connectionTask?.cancel()
        connectionTask = nil
    }

    private func hydrateFromCache() async {
        let cached = (try? await store.timeline(for: threadID)) ?? []
        reducer = ChatTimelineReducer(threadID: threadID, items: cached, now: now)
        oldestTimestamp = cached.map(\.timestamp).min()
        publish()
    }

    private func subscribeToEvents() {
        guard eventTask == nil else { return }
        eventTask = Task { [weak self, session, threadID] in
            let events = await session.events(in: threadID)
            for await event in events {
                guard let self else { break }
                await self.handle(event)
            }
        }
    }

    private func subscribeToConnection() {
        guard connectionTask == nil else { return }
        connectionTask = Task { [weak self, session] in
            let states = await session.connectionState
            for await state in states {
                guard let self else { break }
                await self.handle(connection: state)
            }
        }
    }

    // MARK: - Event handling

    private func handle(_ event: AgentEvent) async {
        reducer.apply(event)
        publish()
        // Persist finalized rows so the cache is warm next launch. Streaming
        // partials are intentionally not persisted — only the terminal states.
        switch event {
        case .response, .toolCompleted, .error, .approvalNeeded:
            await persistTimeline()
        default:
            break
        }
    }

    private func handle(connection state: ConnectionState) async {
        connection = Self.pillStatus(for: state)
        isOffline = !state.isLive

        if state.isLive {
            if hasConnectedBefore {
                // A reconnect: the SSE stream has no replay, so reconcile the
                // local transcript against the history head, then flush any
                // messages queued while offline.
                await reconcileAfterReconnect()
                await flushOutbox()
            } else {
                hasConnectedBefore = true
                // Cold connect: still flush anything queued before this launch.
                await flushOutbox()
            }
        }
    }

    /// Map the transport's ``ConnectionState`` onto the design ``StatusPill``.
    static func pillStatus(for state: ConnectionState) -> StatusPill.Status {
        switch state {
        case .idle: .offline
        case .connecting: .connecting
        case .connected: .connected
        case .reconnecting: .connecting
        case .failed: .offline
        }
    }

    // MARK: - Sending

    /// Send the current draft. Optimistically appends a user row, then either
    /// hands off to the gateway or, when offline, enqueues to the outbox.
    public func send() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        guard !cooldown.isCoolingDown(now: now()) else { return }
        draft = ""

        let rowID = reducer.appendOptimisticUserMessage(text)
        publish()

        if isOffline {
            await enqueue(text, replacing: rowID)
            return
        }

        do {
            _ = try await session.send(text, in: threadID)
            await persistTimeline()
        } catch let error as APIError {
            await handleSendFailure(error, text: text, rowID: rowID)
        } catch {
            retryTexts[rowID] = text
            reducer.markFailure(rowID: rowID, message: "Send failed. Tap to retry.")
            publish()
        }
    }

    private func handleSendFailure(_ error: APIError, text: String, rowID: MessageID) async {
        switch error {
        case .rateLimited(let retryAfter):
            cooldown.begin(retryAfter: retryAfter, now: now())
            refreshCooldown()
            // The message itself was rejected; mark it retryable.
            retryTexts[rowID] = text
            reducer.markFailure(rowID: rowID, message: "Rate limited. Tap to retry.")
        case .transport, .server, .notPaired:
            // Likely transient / connectivity: queue it so it flushes on
            // reconnect instead of forcing the user to retype.
            await enqueue(text, replacing: rowID)
            return
        case .unauthorized, .forbidden, .pinMismatch, .unexpected:
            retryTexts[rowID] = text
            reducer.markFailure(rowID: rowID, message: "Send failed. Tap to retry.")
        }
        publish()
    }

    /// Replace an optimistic row with a queued placeholder and persist the
    /// message to the outbox for ordered flush on reconnect.
    private func enqueue(_ text: String, replacing rowID: MessageID) async {
        reducer.removeRow(rowID)
        _ = reducer.appendQueuedNote(text)
        publish()
        let message = OutboxMessage(threadID: threadID, content: text, queuedAt: now())
        try? await store.enqueueOutbox(message)
    }

    /// Flush queued messages in enqueue order once the gateway is reachable.
    /// Stops on the first failure so ordering is preserved for the next attempt.
    ///
    /// Reentrancy-guarded: connection flapping can fire two `.connected`
    /// transitions in quick succession, each suspending at `await
    /// session.send`; without the guard both would iterate the same outbox
    /// snapshot and double-send.
    private func flushOutbox() async {
        guard !isFlushing else { return }
        isFlushing = true
        defer { isFlushing = false }
        let queued = (try? await store.outbox()) ?? []
        for message in queued where message.threadID == threadID {
            do {
                _ = try await session.send(message.content, in: threadID)
                try? await store.removeFromOutbox(message.id)
            } catch {
                // Leave this and everything after it queued; retry next reconnect.
                break
            }
        }
    }

    /// Retry a failed row: drop the failure row, then resend its original text
    /// through the normal send path (which re-applies offline/429/optimistic
    /// handling). No-op if the row is not a failure we captured text for.
    public func retry(rowID: MessageID) async {
        guard let item = timeline.first(where: { $0.id == rowID }),
            case .failure = item.kind,
            let text = retryTexts[rowID]
        else { return }
        retryTexts.removeValue(forKey: rowID)
        reducer.removeRow(rowID)
        publish()
        draft = text
        await send()
    }

    // MARK: - History paging

    /// Load the next older page when the user scrolls to the top.
    public func loadOlderHistory() async {
        guard !isLoadingHistory, hasMoreHistory else { return }
        isLoadingHistory = true
        defer { isLoadingHistory = false }
        do {
            let page = try await session.history(thread: threadID, before: oldestTimestamp)
            mergeHistory(page)
        } catch {
            // Leave the existing timeline; the banner reflects connectivity.
        }
    }

    private func mergeHistory(_ page: HistoryPage) {
        hasMoreHistory = page.hasMore
        if let oldest = page.oldestTimestamp {
            oldestTimestamp = oldest
        } else if let earliest = page.items.map(\.timestamp).min() {
            oldestTimestamp = earliest
        }
        // Merge older rows in front of what we hold, de-duplicated by id.
        let existingIDs = Set(reducer.items.map(\.id))
        let older = page.items.filter { !existingIDs.contains($0.id) }
        let merged = (older + reducer.items).sorted { $0.timestamp < $1.timestamp }
        reducer = ChatTimelineReducer(threadID: threadID, items: merged, now: now)
        publish()
    }

    // MARK: - Reconcile

    private func reconcileAfterReconnect() async {
        do {
            let result = try await session.reconcile(thread: threadID, against: reducer.items)
            guard !result.isEmpty else { return }
            apply(result)
            await persistTimeline()
        } catch {
            // Reconcile is best-effort; a failure leaves the local view intact.
        }
    }

    /// Apply a ``ReconcileResult`` to the live timeline: drop removed ids,
    /// upsert changed/added rows, then re-sort by timestamp.
    private func apply(_ result: ReconcileResult) {
        var byID: [MessageID: TimelineItem] = [:]
        for item in reducer.items { byID[item.id] = item }
        for id in result.removed { byID.removeValue(forKey: id) }
        for item in result.upserted { byID[item.id] = item }
        let merged = byID.values.sorted { $0.timestamp < $1.timestamp }
        reducer = ChatTimelineReducer(threadID: threadID, items: merged, now: now)
        publish()
    }

    // MARK: - Manual retry (banner)

    /// Manual reconnect trigger from the offline/degraded banner. The session
    /// self-reconnects, but this lets an impatient user nudge a fresh cold open.
    public func retryConnection() async {
        await session.start()
    }

    // MARK: - Publishing

    /// Push the reducer's rows and derived cooldown state into the observable
    /// surface. Single funnel so every mutation redraws consistently.
    private func publish() {
        timeline = reducer.items
        refreshCooldown()
    }

    private func refreshCooldown() {
        cooldownRemaining = cooldown.remaining(now: now())
    }

    private func persistTimeline() async {
        try? await store.replaceTimeline(reducer.items, for: threadID)
    }
}
