import Foundation

/// Pure, platform-neutral reducer that folds a single thread's live
/// ``AgentEvent`` sequence into the flat ``TimelineItem`` list the chat UI
/// renders.
///
/// It lives in ThinClawCore (the dependency-free leaf) rather than in the
/// iOS-only `FeatureChat.ChatStore` so the reducer logic — stream→final swap,
/// tool-call lifecycle rows, optimistic sends, error rows, out-of-order and
/// wrong-thread tolerance — is exercised by plain `swift test` on macOS, with
/// the `@MainActor @Observable` store reduced to a thin wiring shell.
///
/// ## Contract
/// - The reducer is scoped to one ``threadID``. Events for a *different* thread
///   are ignored (defense in depth; the `GatewaySession` already routes
///   per-thread).
/// - ``AgentEvent/streamChunk(content:threadID:)`` values arrive **already
///   coalesced** by ``GatewaySession`` (full accumulated text, not per-token
///   deltas). Each one updates the single in-flight streaming row.
/// - ``AgentEvent/response(content:threadID:)`` finalizes the streaming row:
///   the same row id flips from ``TimelineItem/Kind/streamingAgentMessage`` to
///   ``TimelineItem/Kind/agentMessage`` (a stable id so persistence and
///   reconcile match it), so the UI performs an in-place swap, not an
///   insert+delete flicker.
/// - Tool lifecycle: `tool_started` inserts a `.running` row keyed by tool
///   name; `tool_completed` flips the newest matching running row to
///   `.succeeded`/`.failed`.
///
/// Rows are kept in insertion order; the store sorts for display if it needs
/// strict timestamp ordering. The reducer never sorts, so an out-of-order burst
/// cannot reorder already-shown rows under the user.
public struct ChatTimelineReducer: Sendable {
    /// The thread this reducer is scoped to.
    public let threadID: ThreadID

    /// The current rows, in insertion order.
    public private(set) var items: [TimelineItem]

    /// Id of the in-flight streaming agent row, if one is open. Nil between
    /// turns.
    private var streamingRowID: MessageID?

    /// Monotonic clock seam so the reducer stays pure and deterministic in
    /// tests; the store passes a real clock in production.
    private let now: @Sendable () -> Date

    public init(
        threadID: ThreadID,
        items: [TimelineItem] = [],
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.threadID = threadID
        self.items = items
        self.now = now
    }

    // MARK: - Local (optimistic) mutations

    /// Insert an optimistic user row for a just-composed message and return its
    /// id, so the store can key an eventual failure/retry to it.
    @discardableResult
    public mutating func appendOptimisticUserMessage(_ text: String) -> MessageID {
        let item = TimelineItem(
            threadID: threadID, timestamp: now(), kind: .userMessage(text: text))
        items.append(item)
        return item.id
    }

    /// Insert a "queued (offline)" placeholder as a status note tied to an
    /// outbox message, so the composer shows the send was accepted locally.
    @discardableResult
    public mutating func appendQueuedNote(_ text: String) -> MessageID {
        let item = TimelineItem(
            threadID: threadID, timestamp: now(),
            kind: .statusNote(text: "Queued: \(text)"))
        items.append(item)
        return item.id
    }

    /// Replace the row `id` with a failure row carrying `message` (used when an
    /// optimistic send fails and the UI offers retry). No-op if the id is gone.
    public mutating func markFailure(rowID id: MessageID, message: String) {
        guard let index = items.firstIndex(where: { $0.id == id }) else { return }
        items[index] = TimelineItem(
            id: id, threadID: threadID, timestamp: items[index].timestamp,
            kind: .failure(message: message))
    }

    /// Remove a row by id (e.g. dropping a "queued" note once the send lands).
    public mutating func removeRow(_ id: MessageID) {
        items.removeAll { $0.id == id }
    }

    // MARK: - Live event folding

    /// Fold one live event for this thread into the timeline.
    public mutating func apply(_ event: AgentEvent) {
        // Wrong-thread events are never this reducer's concern. A `nil` thread
        // (e.g. a heartbeat that slipped through) is likewise ignored.
        if let eventThread = event.threadID, eventThread != threadID { return }

        switch event {
        case .streamChunk(let content, _):
            applyStreamingText(content)

        case .response(let content, _):
            finalizeStreaming(with: content)

        case .thinking(let message, _):
            items.append(
                TimelineItem(
                    threadID: threadID, timestamp: now(), kind: .statusNote(text: message)))

        case .toolStarted(let name, _):
            items.append(
                TimelineItem(
                    threadID: threadID, timestamp: now(),
                    kind: .toolCall(name: name, status: .running)))

        case .toolCompleted(let name, let success, _):
            flipToolCall(name: name, to: success ? .succeeded : .failed)

        case .approvalNeeded(let request):
            applyApproval(request)

        case .error(let message, _):
            // Drop any half-streamed row, then append the failure so partial
            // output does not linger looking complete.
            dropOpenStreamingRow()
            items.append(
                TimelineItem(
                    threadID: threadID, timestamp: now(), kind: .failure(message: message)))

        case .usageUpdate, .heartbeat, .unknown:
            // Not rendered in the transcript.
            break
        }
    }

    // MARK: - Streaming helpers

    private mutating func applyStreamingText(_ text: String) {
        if let id = streamingRowID, let index = items.firstIndex(where: { $0.id == id }) {
            items[index] = TimelineItem(
                id: id, threadID: threadID, timestamp: items[index].timestamp,
                kind: .streamingAgentMessage(text: text))
        } else {
            let item = TimelineItem(
                threadID: threadID, timestamp: now(),
                kind: .streamingAgentMessage(text: text))
            items.append(item)
            streamingRowID = item.id
        }
    }

    private mutating func finalizeStreaming(with text: String) {
        if let id = streamingRowID, let index = items.firstIndex(where: { $0.id == id }) {
            // In-place swap: same id, streaming → final, so the UI diff is a
            // content change on one row.
            items[index] = TimelineItem(
                id: id, threadID: threadID, timestamp: items[index].timestamp,
                kind: .agentMessage(text: text))
        } else {
            // A response with no preceding stream (e.g. missed chunks after a
            // reconnect): just insert the final message.
            items.append(
                TimelineItem(
                    threadID: threadID, timestamp: now(), kind: .agentMessage(text: text)))
        }
        streamingRowID = nil
    }

    private mutating func dropOpenStreamingRow() {
        guard let id = streamingRowID else { return }
        items.removeAll { $0.id == id }
        streamingRowID = nil
    }

    // MARK: - Tool + approval helpers

    private mutating func flipToolCall(name: String, to status: TimelineItem.ToolCallStatus) {
        // Flip the most recent still-running row for this tool name. Searching
        // from the end handles repeated calls to the same tool in a turn.
        for index in items.indices.reversed() {
            if case .toolCall(let toolName, .running) = items[index].kind, toolName == name {
                items[index] = TimelineItem(
                    id: items[index].id, threadID: threadID, timestamp: items[index].timestamp,
                    kind: .toolCall(name: name, status: status))
                return
            }
        }
        // A completion with no matching running row (out-of-order or missed
        // start): synthesize the terminal row so the tool result is not lost.
        items.append(
            TimelineItem(
                threadID: threadID, timestamp: now(),
                kind: .toolCall(name: name, status: status)))
    }

    private mutating func applyApproval(_ request: ApprovalRequest) {
        // Approvals are keyed by request id so a duplicate `approval_needed`
        // (e.g. after a reconnect) updates the existing row rather than stacking.
        let rowID = MessageID("approval-\(request.requestID)")
        let item = TimelineItem(
            id: rowID, threadID: threadID, timestamp: now(), kind: .approval(request))
        if let index = items.firstIndex(where: { $0.id == rowID }) {
            items[index] = item
        } else {
            items.append(item)
        }
    }
}
