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
        // Sorted-by-timestamp on read, matching the GRDB store's contract so the
        // two implementations are interchangeable (see the parity test suite).
        (timelines[thread] ?? []).sorted { $0.timestamp < $1.timestamp }
    }

    public func replaceTimeline(_ items: [TimelineItem], for thread: ThreadID) async throws {
        timelines[thread] = items
    }

    public func append(_ item: TimelineItem, to thread: ThreadID) async throws {
        // Upsert by id: appending an item whose id already exists (e.g. a
        // streaming row finalizing to its server id) replaces it, matching the
        // GRDB store's `(thread_id, item_id)` upsert.
        var items = timelines[thread] ?? []
        if let index = items.firstIndex(where: { $0.id == item.id }) {
            items[index] = item
        } else {
            items.append(item)
        }
        timelines[thread] = items
    }

    public func enqueueOutbox(_ message: OutboxMessage) async throws {
        outboxMessages.append(message)
    }

    public func enqueueOutbox(
        _ message: OutboxMessage,
        timelineItem: TimelineItem,
        in thread: ThreadID
    ) async throws {
        var items = timelines[thread] ?? []
        if let index = items.firstIndex(where: { $0.id == timelineItem.id }) {
            items[index] = timelineItem
        } else {
            items.append(timelineItem)
        }
        timelines[thread] = items
        outboxMessages.removeAll { $0.id == message.id }
        outboxMessages.append(message)
    }

    public func outbox() async throws -> [OutboxMessage] {
        // Match the GRDB store: order by queued_at, then id as a stable
        // tie-breaker for same-instant enqueues (parity contract).
        outboxMessages.sorted {
            ($0.queuedAt, $0.id.uuidString) < ($1.queuedAt, $1.id.uuidString)
        }
    }

    public func removeFromOutbox(_ id: UUID) async throws {
        outboxMessages.removeAll { $0.id == id }
    }

    public func clearAll() async throws {
        threadsByID.removeAll()
        timelines.removeAll()
        outboxMessages.removeAll()
    }
}
