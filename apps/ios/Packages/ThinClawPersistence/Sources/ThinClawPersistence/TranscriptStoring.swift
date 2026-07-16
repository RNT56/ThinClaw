import Foundation
import ThinClawCore

/// Local cache of threads and timeline items plus the offline send-outbox.
///
/// The gateway owns history; implementations are caches that may be reset
/// and re-synced at any time. The production GRDB-backed store arrives at
/// milestone M1 (app-process-only — extensions read snapshot files instead).
public protocol TranscriptStoring: Sendable {
    func threads() async throws -> [ChatThread]
    func upsert(thread: ChatThread) async throws
    func deleteThread(_ id: ThreadID) async throws

    func timeline(for thread: ThreadID) async throws -> [TimelineItem]
    func replaceTimeline(_ items: [TimelineItem], for thread: ThreadID) async throws
    func append(_ item: TimelineItem, to thread: ThreadID) async throws

    /// Messages composed while offline, flushed in order on reconnect.
    func enqueueOutbox(_ message: OutboxMessage) async throws
    /// Persist the optimistic timeline row and its outbox envelope as one
    /// transaction. Implementations must not leave only one half behind.
    func enqueueOutbox(
        _ message: OutboxMessage,
        timelineItem: TimelineItem,
        in thread: ThreadID
    ) async throws
    func outbox() async throws -> [OutboxMessage]
    func removeFromOutbox(_ id: UUID) async throws

    /// Erase every cached thread, timeline row, and queued send in this
    /// namespace. Used by the deterministic unpair lifecycle.
    func clearAll() async throws
}

/// A send queued while the gateway was unreachable.
public struct OutboxMessage: Hashable, Sendable, Codable, Identifiable {
    public let id: UUID
    public var threadID: ThreadID?
    public var content: String
    public var queuedAt: Date
    public var timelineItemID: MessageID?

    public init(
        id: UUID = UUID(),
        threadID: ThreadID?,
        content: String,
        queuedAt: Date,
        timelineItemID: MessageID? = nil
    ) {
        self.id = id
        self.threadID = threadID
        self.content = content
        self.queuedAt = queuedAt
        self.timelineItemID = timelineItemID
    }
}
