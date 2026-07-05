import Foundation

/// Pure reducer that folds bursts of `stream_chunk` events into a single
/// growing text value, so the UI can redraw once per display tick instead of
/// once per token.
///
/// Usage pattern:
/// - call ``reduce(_:)`` for every incoming ``AgentEvent``;
/// - on your own cadence (e.g. a timer or display link) call ``drain()`` to
///   get the latest accumulated text, if it changed;
/// - a terminal event (`response` or `error`) is returned immediately by
///   ``reduce(_:)`` as a final update and resets the coalescer.
///
/// The coalescer is a value type with no clocks, timers, or I/O — callers own
/// the cadence — which is what keeps it trivially unit-testable.
public struct StreamChunkCoalescer: Hashable, Sendable {
    /// A visible change to the streaming message.
    public struct Update: Hashable, Sendable {
        /// Full accumulated text so far (not a delta).
        public var text: String
        public var threadID: ThreadID?
        /// True when the message is complete and streaming ended.
        public var isFinal: Bool

        public init(text: String, threadID: ThreadID? = nil, isFinal: Bool) {
            self.text = text
            self.threadID = threadID
            self.isFinal = isFinal
        }
    }

    private var buffer = ""
    private var threadID: ThreadID?
    private var dirty = false

    public init() {}

    /// Text accumulated for the in-flight message (empty when idle).
    public var pendingText: String { buffer }

    /// Whether ``drain()`` would currently return an update.
    public var hasPendingUpdate: Bool { dirty }

    /// Feed one event. Non-chunk, non-terminal events pass through untouched
    /// (returns `nil`). Terminal events return a final ``Update`` immediately.
    @discardableResult
    public mutating func reduce(_ event: AgentEvent) -> Update? {
        switch event {
        case .streamChunk(let content, let id):
            if threadID == nil { threadID = id }
            buffer += content
            dirty = true
            return nil

        case .response(let content, let id):
            // The gateway's `response` carries the full reply; prefer it, but
            // fall back to the accumulated chunks if it arrives empty.
            let text = content.isEmpty ? buffer : content
            let update = Update(text: text, threadID: id ?? threadID, isFinal: true)
            reset()
            return update

        case .error:
            // Surface whatever streamed in before the failure so partial
            // output is not silently dropped; the caller renders the error
            // itself from the event.
            guard !buffer.isEmpty else { return nil }
            let update = Update(text: buffer, threadID: threadID, isFinal: true)
            reset()
            return update

        case .thinking, .toolStarted, .toolCompleted, .approvalNeeded,
            .authRequired, .credentialPrompt, .usageUpdate, .heartbeat, .unknown:
            return nil
        }
    }

    /// Returns the accumulated in-progress text if it changed since the last
    /// drain, otherwise `nil`. Never returns a final update.
    public mutating func drain() -> Update? {
        guard dirty else { return nil }
        dirty = false
        return Update(text: buffer, threadID: threadID, isFinal: false)
    }

    private mutating func reset() {
        buffer = ""
        threadID = nil
        dirty = false
    }
}
