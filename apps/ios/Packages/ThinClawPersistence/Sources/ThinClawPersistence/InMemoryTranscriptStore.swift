import Foundation
import ThinClawCore

/// Actor-backed in-memory `TranscriptStoring` — the R0 default and the test
/// double for feature code until the GRDB store lands (M1).
public actor InMemoryTranscriptStore: TranscriptStoring {
    private var threadsByID: [ThreadID: ChatThread] = [:]
    private var timelines: [ThreadID: [TimelineItem]] = [:]
    private var outboxMessages: [OutboxMessage] = []

    public init() {}

    public func threads() async throws -> [ChatThread] {
        threadsByID.values.sorted { $0.updatedAt > $1.updatedAt }
    }

    public func upsert(thread: ChatThread) async throws {
        threadsByID[thread.id] = thread
    }

    public func deleteThread(_ id: ThreadID) async throws {
        threadsByID.removeValue(forKey: id)
        timelines.removeValue(forKey: id)
    }

    public func timeline(for thread: ThreadID) async throws -> [TimelineItem] {
        timelines[thread] ?? []
    }

    public func replaceTimeline(_ items: [TimelineItem], for thread: ThreadID) async throws {
        timelines[thread] = items
    }

    public func append(_ item: TimelineItem, to thread: ThreadID) async throws {
        timelines[thread, default: []].append(item)
    }

    public func enqueueOutbox(_ message: OutboxMessage) async throws {
        outboxMessages.append(message)
    }

    public func outbox() async throws -> [OutboxMessage] {
        outboxMessages.sorted { $0.queuedAt < $1.queuedAt }
    }

    public func removeFromOutbox(_ id: UUID) async throws {
        outboxMessages.removeAll { $0.id == id }
    }
}
