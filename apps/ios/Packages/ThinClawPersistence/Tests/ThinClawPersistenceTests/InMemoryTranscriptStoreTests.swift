import Foundation
import Testing
import ThinClawCore

@testable import ThinClawPersistence

@Suite("InMemoryTranscriptStore")
struct InMemoryTranscriptStoreTests {
    private func makeThread(_ id: String, updatedAt: Date = .now) -> ChatThread {
        ChatThread(
            id: ThreadID(id),
            title: "Thread \(id)",
            createdAt: updatedAt.addingTimeInterval(-60),
            updatedAt: updatedAt
        )
    }

    @Test("threads sort most-recently-updated first")
    func threadOrdering() async throws {
        let store = InMemoryTranscriptStore()
        let older = makeThread("a", updatedAt: Date(timeIntervalSince1970: 100))
        let newer = makeThread("b", updatedAt: Date(timeIntervalSince1970: 200))
        try await store.upsert(thread: older)
        try await store.upsert(thread: newer)
        let listed = try await store.threads()
        #expect(listed.map(\.id.rawValue) == ["b", "a"])
    }

    @Test("deleting a thread drops its timeline")
    func deleteCascades() async throws {
        let store = InMemoryTranscriptStore()
        let thread = makeThread("t")
        try await store.upsert(thread: thread)
        try await store.append(
            TimelineItem(threadID: thread.id, timestamp: .now, kind: .userMessage(text: "hi")),
            to: thread.id
        )
        try await store.deleteThread(thread.id)
        #expect(try await store.threads().isEmpty)
        #expect(try await store.timeline(for: thread.id).isEmpty)
    }

    @Test("outbox preserves queue order and removal")
    func outboxOrdering() async throws {
        let store = InMemoryTranscriptStore()
        let first = OutboxMessage(
            threadID: nil, content: "first", queuedAt: Date(timeIntervalSince1970: 1))
        let second = OutboxMessage(
            threadID: nil, content: "second", queuedAt: Date(timeIntervalSince1970: 2))
        try await store.enqueueOutbox(second)
        try await store.enqueueOutbox(first)
        #expect(try await store.outbox().map(\.content) == ["first", "second"])
        try await store.removeFromOutbox(first.id)
        #expect(try await store.outbox().map(\.content) == ["second"])
    }
}
