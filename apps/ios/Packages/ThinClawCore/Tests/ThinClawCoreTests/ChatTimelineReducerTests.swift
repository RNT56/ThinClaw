import Foundation
import Testing

@testable import ThinClawCore

@Suite("ChatTimelineReducer")
struct ChatTimelineReducerTests {
    let thread = ThreadID("th_1")
    let other = ThreadID("th_2")

    /// A fixed clock so synthesized ids differ but timestamps are deterministic.
    private func makeReducer() -> ChatTimelineReducer {
        ChatTimelineReducer(threadID: thread, now: { Date(timeIntervalSince1970: 0) })
    }

    @Test("streaming chunks fold into one row, then the response finalizes it in place")
    func streamThenFinalSwap() {
        var reducer = makeReducer()
        reducer.apply(.streamChunk(content: "Hel", threadID: thread))
        reducer.apply(.streamChunk(content: "Hello", threadID: thread))
        #expect(reducer.items.count == 1)
        guard case .streamingAgentMessage(let partial) = reducer.items[0].kind else {
            Issue.record("expected streaming row")
            return
        }
        #expect(partial == "Hello")
        let streamingID = reducer.items[0].id

        reducer.apply(.response(content: "Hello, world.", threadID: thread))
        #expect(reducer.items.count == 1, "finalize must swap in place, not append")
        #expect(reducer.items[0].id == streamingID, "id is stable across the swap")
        #expect(reducer.items[0].kind == .agentMessage(text: "Hello, world."))
    }

    @Test("a response with no preceding stream is inserted as a final message")
    func responseWithoutStream() {
        var reducer = makeReducer()
        reducer.apply(.response(content: "Direct.", threadID: thread))
        #expect(reducer.items.map(\.kind) == [.agentMessage(text: "Direct.")])
    }

    @Test("tool_started then tool_completed flips the running row to succeeded")
    func toolLifecycleSucceeds() {
        var reducer = makeReducer()
        reducer.apply(.toolStarted(name: "grep", threadID: thread))
        #expect(reducer.items.map(\.kind) == [.toolCall(name: "grep", status: .running)])
        reducer.apply(.toolCompleted(name: "grep", success: true, threadID: thread))
        #expect(reducer.items.map(\.kind) == [.toolCall(name: "grep", status: .succeeded)])
    }

    @Test("a failed tool completion flips to failed")
    func toolLifecycleFails() {
        var reducer = makeReducer()
        reducer.apply(.toolStarted(name: "shell", threadID: thread))
        reducer.apply(.toolCompleted(name: "shell", success: false, threadID: thread))
        #expect(reducer.items.map(\.kind) == [.toolCall(name: "shell", status: .failed)])
    }

    @Test("interleaved tool call around a streaming reply keeps both rows")
    func toolAndStreamInterleave() {
        var reducer = makeReducer()
        reducer.apply(.toolStarted(name: "grep", threadID: thread))
        reducer.apply(.streamChunk(content: "Result: ", threadID: thread))
        reducer.apply(.toolCompleted(name: "grep", success: true, threadID: thread))
        reducer.apply(.streamChunk(content: "Result: 42", threadID: thread))
        reducer.apply(.response(content: "Result: 42", threadID: thread))
        #expect(reducer.items.count == 2)
        #expect(reducer.items[0].kind == .toolCall(name: "grep", status: .succeeded))
        #expect(reducer.items[1].kind == .agentMessage(text: "Result: 42"))
    }

    @Test("repeated calls to the same tool flip the newest running row")
    func repeatedToolName() {
        var reducer = makeReducer()
        reducer.apply(.toolStarted(name: "grep", threadID: thread))
        reducer.apply(.toolStarted(name: "grep", threadID: thread))
        reducer.apply(.toolCompleted(name: "grep", success: true, threadID: thread))
        // First stays running, second (newest) succeeds.
        #expect(
            reducer.items.map(\.kind) == [
                .toolCall(name: "grep", status: .running),
                .toolCall(name: "grep", status: .succeeded),
            ])
    }

    @Test("out-of-order tool completion with no running row synthesizes a terminal row")
    func outOfOrderToolCompletion() {
        var reducer = makeReducer()
        reducer.apply(.toolCompleted(name: "grep", success: true, threadID: thread))
        #expect(reducer.items.map(\.kind) == [.toolCall(name: "grep", status: .succeeded)])
    }

    @Test("events for another thread are ignored")
    func wrongThreadIgnored() {
        var reducer = makeReducer()
        reducer.apply(.streamChunk(content: "nope", threadID: other))
        reducer.apply(.response(content: "nope", threadID: other))
        reducer.apply(.toolStarted(name: "x", threadID: other))
        #expect(reducer.items.isEmpty)
    }

    @Test("nil-thread events (heartbeat) are ignored")
    func nilThreadIgnored() {
        var reducer = makeReducer()
        reducer.apply(.heartbeat)
        reducer.apply(.usageUpdate(UsageUpdate(inputTokens: 1, outputTokens: 2)))
        #expect(reducer.items.isEmpty)
    }

    @Test("error drops a half-streamed row and appends a failure")
    func errorDropsPartialStream() {
        var reducer = makeReducer()
        reducer.apply(.streamChunk(content: "half", threadID: thread))
        reducer.apply(.error(message: "boom", threadID: thread))
        #expect(reducer.items.map(\.kind) == [.failure(message: "boom")])
    }

    @Test("thinking becomes a status note")
    func thinkingNote() {
        var reducer = makeReducer()
        reducer.apply(.thinking(message: "Reading file…", threadID: thread))
        #expect(reducer.items.map(\.kind) == [.statusNote(text: "Reading file…")])
    }

    @Test("approval rows are keyed by request id and de-duplicated")
    func approvalDedup() {
        var reducer = makeReducer()
        let request = ApprovalRequest(
            requestID: "r1", toolName: "shell", description: "run", parameters: "{}",
            threadID: thread)
        reducer.apply(.approvalNeeded(request))
        reducer.apply(.approvalNeeded(request))  // e.g. redelivered after reconnect
        #expect(reducer.items.count == 1)
        #expect(reducer.items[0].id == MessageID("approval-r1"))
    }

    @Test("optimistic user message returns a stable id that failure can target")
    func optimisticThenFailure() {
        var reducer = makeReducer()
        let id = reducer.appendOptimisticUserMessage("hi")
        #expect(reducer.items.map(\.kind) == [.userMessage(text: "hi")])
        reducer.markFailure(rowID: id, message: "send failed")
        #expect(reducer.items.map(\.kind) == [.failure(message: "send failed")])
    }

    @Test("queued note can be appended then removed when the send lands")
    func queuedNoteLifecycle() {
        var reducer = makeReducer()
        let id = reducer.appendQueuedNote("later")
        #expect(reducer.items.count == 1)
        reducer.removeRow(id)
        #expect(reducer.items.isEmpty)
    }

    @Test("a full turn: user → thinking → tool → stream → response")
    func fullTurn() {
        var reducer = makeReducer()
        reducer.appendOptimisticUserMessage("What is 6*7?")
        reducer.apply(.thinking(message: "Calculating…", threadID: thread))
        reducer.apply(.toolStarted(name: "calc", threadID: thread))
        reducer.apply(.toolCompleted(name: "calc", success: true, threadID: thread))
        reducer.apply(.streamChunk(content: "The answer ", threadID: thread))
        reducer.apply(.streamChunk(content: "The answer is 42.", threadID: thread))
        reducer.apply(.response(content: "The answer is 42.", threadID: thread))
        #expect(
            reducer.items.map(\.kind) == [
                .userMessage(text: "What is 6*7?"),
                .statusNote(text: "Calculating…"),
                .toolCall(name: "calc", status: .succeeded),
                .agentMessage(text: "The answer is 42."),
            ])
    }
}
