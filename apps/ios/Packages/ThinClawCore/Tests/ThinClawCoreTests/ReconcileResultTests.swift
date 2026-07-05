import Foundation
import Testing

@testable import ThinClawCore

@Suite("ReconcileResult.diff")
struct ReconcileResultTests {
    private let thread = ThreadID("t1")

    private func item(
        _ id: String,
        _ secondsFromEpoch: TimeInterval,
        _ kind: TimelineItem.Kind,
        thread: ThreadID? = nil
    ) -> TimelineItem {
        TimelineItem(
            id: MessageID(id),
            threadID: thread ?? self.thread,
            timestamp: Date(timeIntervalSince1970: secondsFromEpoch),
            kind: kind)
    }

    @Test("identical local and server produce an empty repair set")
    func noDivergence() {
        let items = [
            item("a", 10, .userMessage(text: "hi")),
            item("b", 11, .agentMessage(text: "hello")),
        ]
        let result = ReconcileResult.diff(threadID: thread, local: items, server: items)
        #expect(result.isEmpty)
        #expect(result.threadID == thread)
    }

    @Test("server items missing locally are upserted")
    func serverAheadUpserts() {
        let local = [item("a", 10, .userMessage(text: "hi"))]
        let server = [
            item("a", 10, .userMessage(text: "hi")),
            item("b", 11, .agentMessage(text: "hello")),
        ]
        let result = ReconcileResult.diff(threadID: thread, local: local, server: server)
        #expect(result.upserted.map(\.id) == [MessageID("b")])
        #expect(result.removed.isEmpty)
    }

    @Test("an item that changed value is re-upserted with the server version")
    func changedItemUpserts() {
        let local = [item("t", 10, .toolCall(name: "fs", status: .running))]
        let server = [item("t", 10, .toolCall(name: "fs", status: .succeeded))]
        let result = ReconcileResult.diff(threadID: thread, local: local, server: server)
        #expect(result.upserted.count == 1)
        #expect(result.upserted.first?.kind == .toolCall(name: "fs", status: .succeeded))
        #expect(result.removed.isEmpty)
    }

    @Test("local-only items within the server window are removed")
    func localOnlyInWindowRemoved() {
        // Server window is [10, 12]; the stray local "x" at t=11 is inside it
        // and absent from the server → it must be dropped.
        let local = [
            item("a", 10, .userMessage(text: "hi")),
            item("x", 11, .agentMessage(text: "optimistic that never landed")),
            item("b", 12, .agentMessage(text: "hello")),
        ]
        let server = [
            item("a", 10, .userMessage(text: "hi")),
            item("b", 12, .agentMessage(text: "hello")),
        ]
        let result = ReconcileResult.diff(threadID: thread, local: local, server: server)
        #expect(result.removed == [MessageID("x")])
        #expect(result.upserted.isEmpty)
    }

    @Test("local items older than the server window are preserved, not removed")
    func localBeforeWindowPreserved() {
        // Server head only covers [50, 51]; the older local "old" at t=10 is
        // simply not in this page and must be left alone.
        let local = [
            item("old", 10, .userMessage(text: "ancient")),
            item("a", 50, .userMessage(text: "hi")),
        ]
        let server = [
            item("a", 50, .userMessage(text: "hi")),
            item("b", 51, .agentMessage(text: "hello")),
        ]
        let result = ReconcileResult.diff(threadID: thread, local: local, server: server)
        #expect(result.removed.isEmpty)
        #expect(result.upserted.map(\.id) == [MessageID("b")])
    }

    @Test("items belonging to other threads are ignored on both sides")
    func otherThreadsIgnored() {
        let other = ThreadID("t2")
        let local = [
            item("a", 10, .userMessage(text: "hi")),
            item("z", 11, .userMessage(text: "other thread"), thread: other),
        ]
        let server = [item("a", 10, .userMessage(text: "hi"))]
        let result = ReconcileResult.diff(threadID: thread, local: local, server: server)
        #expect(result.isEmpty)
    }

    @Test("empty server head against a non-empty local view removes nothing")
    func emptyServerRemovesNothing() {
        // With no server window, there is nothing to reconcile against, so the
        // local view is trusted wholesale (no false removals).
        let local = [item("a", 10, .userMessage(text: "hi"))]
        let result = ReconcileResult.diff(threadID: thread, local: local, server: [])
        #expect(result.isEmpty)
    }

    @Test("a wrong-thread server response never deletes local items of this thread")
    func misroutedServerResponseRemovesNothing() {
        // Defense-in-depth: the server answered a history request for `thread`
        // with items that all belong to a *different* thread (a misrouted or
        // buggy response). The window must be scoped to this thread, so it is
        // undefined here — and the recent, valid local items for `thread` must
        // survive untouched rather than being deleted as "missing".
        let other = ThreadID("t2")
        let local = [
            item("a", 50, .userMessage(text: "hi")),
            item("b", 51, .agentMessage(text: "hello")),
        ]
        // Cross-thread items with OLDER timestamps: if the window were computed
        // over all server items, it would start at t=10 and both local items
        // (t=50, t=51) would fall inside it and be wrongly removed.
        let server = [
            item("x", 10, .userMessage(text: "other thread"), thread: other),
            item("y", 11, .agentMessage(text: "other thread"), thread: other),
        ]
        let result = ReconcileResult.diff(threadID: thread, local: local, server: server)
        #expect(result.removed.isEmpty)
        #expect(result.upserted.isEmpty)
    }
}
