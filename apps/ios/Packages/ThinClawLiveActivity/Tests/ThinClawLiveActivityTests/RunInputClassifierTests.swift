import Foundation
import Testing
import ThinClawCore

@testable import ThinClawLiveActivity

/// Which raw `AgentEvent`s drive the agent-run Live Activity for the active
/// thread. Pure mapping; no ActivityKit.
@Suite("RunInputClassifier")
struct RunInputClassifierTests {
    private let active = ThreadID("web-1")

    private func classify(_ event: AgentEvent) -> RunInput? {
        RunInputClassifier.input(from: event, activeThread: active, threadTitle: "Chat")
    }

    @Test("thinking → .thinking")
    func thinking() {
        #expect(classify(.thinking(message: "…", threadID: active)) == .thinking(threadID: active))
    }

    @Test("tool_started carries the (local-only) tool name")
    func toolStarted() {
        #expect(
            classify(.toolStarted(name: "read_file", threadID: active))
                == .toolStarted(threadID: active, toolName: "read_file"))
    }

    @Test("tool_completed returns to thinking (drops the tool name)")
    func toolCompleted() {
        #expect(
            classify(.toolCompleted(name: "read_file", success: true, threadID: active))
                == .thinking(threadID: active))
    }

    @Test("approval_needed → .awaitingApproval with the request id")
    func approvalNeeded() {
        let request = ApprovalRequest(
            requestID: "req-9", toolName: "shell", description: "run", parameters: "{}",
            risk: .high, threadID: active)
        #expect(
            classify(.approvalNeeded(request))
                == .awaitingApproval(threadID: active, requestID: "req-9"))
    }

    @Test("response → .completed")
    func response() {
        #expect(
            classify(.response(content: "done", threadID: active)) == .completed(threadID: active))
    }

    @Test("error → .failed")
    func error() {
        #expect(classify(.error(message: "boom", threadID: active)) == .failed(threadID: active))
    }

    @Test("content/accounting/heartbeat events do not drive the activity")
    func nonDriving() {
        #expect(classify(.streamChunk(content: "tok", threadID: active)) == nil)
        #expect(
            classify(.usageUpdate(UsageUpdate(inputTokens: 1, outputTokens: 2, threadID: active)))
                == nil)
        #expect(classify(.heartbeat) == nil)
        #expect(classify(.unknown(type: "plan_update")) == nil)
    }

    @Test("events for another thread are ignored")
    func otherThreadIgnored() {
        let other = ThreadID("web-2")
        #expect(classify(.thinking(message: "…", threadID: other)) == nil)
        #expect(classify(.response(content: "x", threadID: other)) == nil)
    }

    @Test("thread-less events are ignored")
    func threadlessIgnored() {
        #expect(classify(.thinking(message: "…", threadID: nil)) == nil)
    }

    // MARK: - Run-start classification

    @Test("thinking/tool_started/approval start a run; response/error do not")
    func runStart() {
        #expect(RunInputClassifier.isRunStart(.thinking(message: "", threadID: active), activeThread: active))
        #expect(
            RunInputClassifier.isRunStart(.toolStarted(name: "x", threadID: active), activeThread: active))
        let req = ApprovalRequest(
            requestID: "r", toolName: "t", description: "", parameters: "{}", risk: .low,
            threadID: active)
        #expect(RunInputClassifier.isRunStart(.approvalNeeded(req), activeThread: active))
        #expect(
            !RunInputClassifier.isRunStart(.response(content: "", threadID: active), activeThread: active))
        #expect(!RunInputClassifier.isRunStart(.error(message: "", threadID: active), activeThread: active))
    }

    @Test("a run-start signal for another thread is not a start")
    func runStartOtherThread() {
        let other = ThreadID("web-2")
        #expect(!RunInputClassifier.isRunStart(.thinking(message: "", threadID: other), activeThread: active))
    }
}
