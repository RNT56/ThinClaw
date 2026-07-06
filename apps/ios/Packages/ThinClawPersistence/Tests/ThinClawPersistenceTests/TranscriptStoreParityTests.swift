import Foundation
import Testing
import ThinClawCore

@testable import ThinClawPersistence

/// The two store implementations under test. Each parameterized case builds a
/// fresh store so the suites are independent and order-free.
enum StoreKind: CaseIterable, CustomStringConvertible {
    case inMemory
    case grdb

    var description: String {
        switch self {
        case .inMemory: "InMemoryTranscriptStore"
        case .grdb: "GRDBTranscriptStore"
        }
    }
}

/// Build a fresh store of the requested kind. The GRDB store gets a unique
/// temp-file database so the WAL/on-disk path is exercised (not an in-memory
/// SQLite connection), matching production.
///
/// - Returns: the store and, for GRDB, the directory to clean up.
func makeStore(_ kind: StoreKind) throws -> (store: any TranscriptStoring, cleanup: () -> Void) {
    switch kind {
    case .inMemory:
        return (InMemoryTranscriptStore(), {})
    case .grdb:
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("thinclaw-tests-\(UUID().uuidString)", isDirectory: true)
        let dbURL = dir.appendingPathComponent("transcripts.sqlite", isDirectory: false)
        let store = try GRDBTranscriptStore(path: dbURL)
        return (store, { try? FileManager.default.removeItem(at: dir) })
    }
}

private func makeThread(_ id: String, updatedAt: Date = .now) -> ChatThread {
    ChatThread(
        id: ThreadID(id),
        title: "Thread \(id)",
        channel: "ios",
        createdAt: updatedAt.addingTimeInterval(-60),
        updatedAt: updatedAt,
        lastMessagePreview: "preview \(id)")
}

/// Behavioral contract every `TranscriptStoring` implementation must satisfy,
/// run against BOTH the in-memory and GRDB stores so they can never drift.
@Suite("TranscriptStoring parity")
struct TranscriptStoreParityTests {
    @Test("threads sort most-recently-updated first", arguments: StoreKind.allCases)
    func threadOrdering(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        try await store.upsert(thread: makeThread("a", updatedAt: Date(timeIntervalSince1970: 100)))
        try await store.upsert(thread: makeThread("b", updatedAt: Date(timeIntervalSince1970: 200)))
        let listed = try await store.threads()
        #expect(listed.map(\.id.rawValue) == ["b", "a"])
    }

    @Test("upsert replaces an existing thread in place", arguments: StoreKind.allCases)
    func upsertReplaces(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        try await store.upsert(thread: makeThread("t", updatedAt: Date(timeIntervalSince1970: 1)))
        var updated = makeThread("t", updatedAt: Date(timeIntervalSince1970: 2))
        updated.title = "Renamed"
        try await store.upsert(thread: updated)
        let listed = try await store.threads()
        #expect(listed.count == 1)
        #expect(listed.first?.title == "Renamed")
    }

    @Test("deleting a thread drops its timeline", arguments: StoreKind.allCases)
    func deleteCascades(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        let thread = makeThread("t")
        try await store.upsert(thread: thread)
        try await store.append(
            TimelineItem(threadID: thread.id, timestamp: .now, kind: .userMessage(text: "hi")),
            to: thread.id)
        try await store.deleteThread(thread.id)
        #expect(try await store.threads().isEmpty)
        #expect(try await store.timeline(for: thread.id).isEmpty)
    }

    @Test("timeline returns items in timestamp order", arguments: StoreKind.allCases)
    func timelineOrdering(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        let thread = ThreadID("t")
        let later = TimelineItem(
            id: MessageID("b"), threadID: thread,
            timestamp: Date(timeIntervalSince1970: 200), kind: .agentMessage(text: "second"))
        let earlier = TimelineItem(
            id: MessageID("a"), threadID: thread,
            timestamp: Date(timeIntervalSince1970: 100), kind: .userMessage(text: "first"))
        // Append out of order; the store sorts by timestamp on read.
        try await store.append(later, to: thread)
        try await store.append(earlier, to: thread)
        let timeline = try await store.timeline(for: thread)
        #expect(timeline.map(\.id.rawValue) == ["a", "b"])
    }

    @Test("append upserts by (thread, item) id", arguments: StoreKind.allCases)
    func appendUpserts(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        let thread = ThreadID("t")
        let id = MessageID("row-1")
        try await store.append(
            TimelineItem(
                id: id, threadID: thread, timestamp: Date(timeIntervalSince1970: 1),
                kind: .streamingAgentMessage(text: "partial")),
            to: thread)
        try await store.append(
            TimelineItem(
                id: id, threadID: thread, timestamp: Date(timeIntervalSince1970: 2),
                kind: .agentMessage(text: "final")),
            to: thread)
        let timeline = try await store.timeline(for: thread)
        #expect(timeline.count == 1)
        #expect(timeline.first?.kind == .agentMessage(text: "final"))
    }

    @Test("replaceTimeline swaps the whole thread transcript", arguments: StoreKind.allCases)
    func replaceTimeline(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        let thread = ThreadID("t")
        try await store.append(
            TimelineItem(
                id: MessageID("old"), threadID: thread, timestamp: Date(timeIntervalSince1970: 1),
                kind: .userMessage(text: "old")),
            to: thread)
        let fresh = [
            TimelineItem(
                id: MessageID("new-1"), threadID: thread,
                timestamp: Date(timeIntervalSince1970: 10), kind: .userMessage(text: "q")),
            TimelineItem(
                id: MessageID("new-2"), threadID: thread,
                timestamp: Date(timeIntervalSince1970: 11), kind: .agentMessage(text: "a")),
        ]
        try await store.replaceTimeline(fresh, for: thread)
        let timeline = try await store.timeline(for: thread)
        #expect(timeline.map(\.id.rawValue) == ["new-1", "new-2"])
    }

    @Test("timeline of an unknown thread is empty", arguments: StoreKind.allCases)
    func unknownThreadEmpty(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        #expect(try await store.timeline(for: ThreadID("nope")).isEmpty)
    }

    @Test("outbox preserves queue order and removal", arguments: StoreKind.allCases)
    func outboxOrdering(_ kind: StoreKind) async throws {
        let (store, cleanup) = try makeStore(kind)
        defer { cleanup() }
        let first = OutboxMessage(
            threadID: nil, content: "first", queuedAt: Date(timeIntervalSince1970: 1))
        let second = OutboxMessage(
            threadID: ThreadID("t"), content: "second", queuedAt: Date(timeIntervalSince1970: 2))
        // Enqueue out of order; store sorts by queuedAt.
        try await store.enqueueOutbox(second)
        try await store.enqueueOutbox(first)
        #expect(try await store.outbox().map(\.content) == ["first", "second"])
        try await store.removeFromOutbox(first.id)
        let remaining = try await store.outbox()
        #expect(remaining.map(\.content) == ["second"])
        #expect(remaining.first?.threadID == ThreadID("t"))
    }
}

/// GRDB-specific coverage: on-disk round-trip across reopen, migration head,
/// and all timeline kinds surviving JSON payload storage.
@Suite("GRDBTranscriptStore")
struct GRDBTranscriptStoreTests {
    private func withTempDBURL<T>(_ body: (URL) async throws -> T) async throws -> T {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("thinclaw-grdb-\(UUID().uuidString)", isDirectory: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let dbURL = dir.appendingPathComponent("transcripts.sqlite", isDirectory: false)
        return try await body(dbURL)
    }

    @Test("data persists across store reopen (durable on disk)")
    func roundTripAcrossReopen() async throws {
        try await withTempDBURL { dbURL in
            let thread = ThreadID("t")
            do {
                let store = try GRDBTranscriptStore(path: dbURL)
                try await store.upsert(
                    thread: ChatThread(
                        id: thread, title: "Persisted",
                        createdAt: Date(timeIntervalSince1970: 1),
                        updatedAt: Date(timeIntervalSince1970: 2)))
                try await store.append(
                    TimelineItem(
                        id: MessageID("m1"), threadID: thread,
                        timestamp: Date(timeIntervalSince1970: 3),
                        kind: .agentMessage(text: "hello")),
                    to: thread)
                try await store.enqueueOutbox(
                    OutboxMessage(
                        threadID: thread, content: "queued",
                        queuedAt: Date(timeIntervalSince1970: 4)))
            }
            // Reopen a brand new pool over the same file: everything is still
            // there, proving it hit the disk, not just an in-memory cache.
            let reopened = try GRDBTranscriptStore(path: dbURL)
            #expect(try await reopened.threads().map(\.title) == ["Persisted"])
            #expect(try await reopened.timeline(for: thread).count == 1)
            #expect(try await reopened.outbox().map(\.content) == ["queued"])
        }
    }

    @Test("migration brings a fresh database to the v1 head")
    func migrationHead() async throws {
        try await withTempDBURL { dbURL in
            let store = try GRDBTranscriptStore(path: dbURL)
            let (threads, items, outbox) = try await store.appliedMigrationsAndTables()
            #expect(threads)
            #expect(items)
            #expect(outbox)
        }
    }

    @Test("every timeline kind survives the JSON payload round-trip")
    func allKindsRoundTrip() async throws {
        try await withTempDBURL { dbURL in
            let store = try GRDBTranscriptStore(path: dbURL)
            let thread = ThreadID("t")
            let approval = ApprovalRequest(
                requestID: "r1", toolName: "shell", description: "run", parameters: "{}",
                risk: .high, threadID: thread)
            let kinds: [TimelineItem.Kind] = [
                .userMessage(text: "u"),
                .agentMessage(text: "a"),
                .streamingAgentMessage(text: "s"),
                .statusNote(text: "n"),
                .toolCall(name: "grep", status: .running),
                .toolCall(name: "grep", status: .succeeded),
                .toolCall(name: "grep", status: .failed),
                .approval(approval),
                .failure(message: "boom"),
            ]
            let items = kinds.enumerated().map { index, kind in
                TimelineItem(
                    id: MessageID("k\(index)"), threadID: thread,
                    timestamp: Date(timeIntervalSince1970: Double(index)), kind: kind)
            }
            try await store.replaceTimeline(items, for: thread)
            let loaded = try await store.timeline(for: thread)
            #expect(loaded == items)
        }
    }
}
