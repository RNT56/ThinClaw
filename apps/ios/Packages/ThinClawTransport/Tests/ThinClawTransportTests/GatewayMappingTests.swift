import Foundation
import Testing
import ThinClawAPI
import ThinClawCore

@testable import ThinClawTransport

@Suite("GatewayMapping wire → domain")
struct GatewayMappingTests {
    // MARK: - Threads

    @Test("thread listing maps ids, title, channel, and timestamps")
    func mapsThreadListing() {
        let response = Components.Schemas.ThreadListResponse(
            activeThread: "t1",
            threads: [
                .init(
                    createdAt: "2026-07-04T10:00:00Z",
                    id: "t1",
                    state: "idle",
                    threadType: "ios",
                    title: "Trip planning",
                    turnCount: 3,
                    updatedAt: "2026-07-04T10:05:30Z"),
                .init(
                    createdAt: "2026-07-04T09:00:00Z",
                    id: "t2",
                    state: "running",
                    threadType: nil,
                    title: nil,
                    turnCount: 0,
                    updatedAt: "2026-07-04T09:00:00Z"),
            ])

        let threads = GatewayMapping.chatThreads(from: response)
        #expect(threads.count == 2)
        #expect(threads[0].id == ThreadID("t1"))
        #expect(threads[0].title == "Trip planning")
        #expect(threads[0].channel == "ios")
        #expect(threads[0].createdAt == GatewayMapping.date("2026-07-04T10:00:00Z"))
        #expect(threads[0].updatedAt == GatewayMapping.date("2026-07-04T10:05:30Z"))
        // Missing title/channel degrade to empty/nil, not a crash.
        #expect(threads[1].title == "")
        #expect(threads[1].channel == nil)
        #expect(threads[1].lastMessagePreview == nil)
    }

    // MARK: - History turns

    @Test("a turn maps to user message, tool calls, then agent response in order")
    func mapsTurnWithToolCalls() {
        let turn = Components.Schemas.TurnInfo(
            completedAt: "2026-07-04T10:00:05Z",
            hideUserInput: false,
            response: "Here is the weather.",
            startedAt: "2026-07-04T10:00:00Z",
            state: "completed",
            toolCalls: [
                .init(hasError: false, hasResult: true, name: "get_weather"),
                .init(hasError: true, hasResult: false, name: "flaky_tool"),
                .init(hasError: false, hasResult: false, name: "pending_tool"),
            ],
            turnNumber: 1,
            userInput: "What's the weather?")

        let items = GatewayMapping.timelineItems(from: turn, threadID: ThreadID("t1"))
        #expect(items.count == 5)

        guard case .userMessage(let text) = items[0].kind else {
            Issue.record("row 0 should be the user message")
            return
        }
        #expect(text == "What's the weather?")

        #expect(items[1].kind == .toolCall(name: "get_weather", status: .succeeded))
        #expect(items[2].kind == .toolCall(name: "flaky_tool", status: .failed))
        #expect(items[3].kind == .toolCall(name: "pending_tool", status: .running))

        #expect(items[4].kind == .agentMessage(text: "Here is the weather."))
        // Agent response uses the completion time; the user row uses start.
        #expect(items[0].timestamp == GatewayMapping.date("2026-07-04T10:00:00Z"))
        #expect(items[4].timestamp == GatewayMapping.date("2026-07-04T10:00:05Z"))
    }

    @Test("hidden user input suppresses the user row")
    func hidesUserInput() {
        let turn = Components.Schemas.TurnInfo(
            hideUserInput: true,
            response: "background result",
            startedAt: "2026-07-04T10:00:00Z",
            state: "completed",
            toolCalls: [],
            turnNumber: 2,
            userInput: "internal prompt")
        let items = GatewayMapping.timelineItems(from: turn, threadID: ThreadID("t1"))
        #expect(items.count == 1)
        #expect(items[0].kind == .agentMessage(text: "background result"))
    }

    @Test("a turn with no response omits the agent row")
    func omitsEmptyResponse() {
        let turn = Components.Schemas.TurnInfo(
            response: nil,
            startedAt: "2026-07-04T10:00:00Z",
            state: "running",
            toolCalls: [],
            turnNumber: 3,
            userInput: "hi")
        let items = GatewayMapping.timelineItems(from: turn, threadID: ThreadID("t1"))
        #expect(items.count == 1)
        #expect(items[0].kind == .userMessage(text: "hi"))
    }

    @Test("turn item ids are stable across identical remappings (for reconcile)")
    func stableTurnIDs() {
        let turn = Components.Schemas.TurnInfo(
            response: "r",
            startedAt: "2026-07-04T10:00:00Z",
            state: "completed",
            toolCalls: [.init(hasError: false, hasResult: true, name: "t")],
            turnNumber: 7,
            userInput: "q")
        let first = GatewayMapping.timelineItems(from: turn, threadID: ThreadID("t1"))
        let second = GatewayMapping.timelineItems(from: turn, threadID: ThreadID("t1"))
        #expect(first.map(\.id) == second.map(\.id))
        // And distinct across roles within the turn.
        #expect(Set(first.map(\.id)).count == first.count)
    }

    @Test("history response maps pagination cursor and hasMore")
    func mapsHistoryPage() {
        let response = Components.Schemas.HistoryResponse(
            hasMore: true,
            oldestTimestamp: "2026-07-04T09:00:00Z",
            threadId: "t1",
            turns: [
                .init(
                    response: "hello",
                    startedAt: "2026-07-04T10:00:00Z",
                    state: "completed",
                    toolCalls: [],
                    turnNumber: 1,
                    userInput: "hi")
            ])
        let page = GatewayMapping.historyPage(from: response)
        #expect(page.threadID == ThreadID("t1"))
        #expect(page.hasMore == true)
        #expect(page.oldestTimestamp == GatewayMapping.date("2026-07-04T09:00:00Z"))
        #expect(page.items.count == 2)  // user + agent
    }

    @Test("history hasMore defaults to false when absent")
    func historyHasMoreDefault() {
        let response = Components.Schemas.HistoryResponse(threadId: "t1", turns: [])
        let page = GatewayMapping.historyPage(from: response)
        #expect(page.hasMore == false)
        #expect(page.oldestTimestamp == nil)
        #expect(page.items.isEmpty)
    }

    // MARK: - Approvals

    @Test("a pending approval entry maps to an ApprovalRequest")
    func mapsApproval() {
        let entry = Components.Schemas.PendingApprovalEntry(
            createdAt: "2026-07-04T10:00:00Z",
            description: "Write to disk",
            parameters: #"{"path":"/tmp/x"}"#,
            requestId: "req-1",
            risk: .high,
            threadId: "t1",
            toolName: "fs_write")
        let request = GatewayMapping.approvalRequest(from: entry)
        #expect(request.requestID == "req-1")
        #expect(request.toolName == "fs_write")
        #expect(request.description == "Write to disk")
        #expect(request.parameters == #"{"path":"/tmp/x"}"#)
        #expect(request.risk == .high)
        #expect(request.threadID == ThreadID("t1"))
    }

    @Test("approvals response preserves oldest-first ordering")
    func mapsApprovalsList() {
        let response = Components.Schemas.PendingApprovalsResponse(approvals: [
            .init(
                createdAt: "2026-07-04T10:00:00Z", description: "a", parameters: "{}",
                requestId: "r1", risk: .low, threadId: "t1", toolName: "tool_a"),
            .init(
                createdAt: "2026-07-04T10:01:00Z", description: "b", parameters: "{}",
                requestId: "r2", risk: .high, threadId: "t1", toolName: "tool_b"),
        ])
        let requests = GatewayMapping.approvalRequests(from: response)
        #expect(requests.map(\.requestID) == ["r1", "r2"])
        #expect(requests.map(\.risk) == [.low, .high])
    }

    @Test("an approval maps to an inline timeline row keyed by request id")
    func mapsApprovalTimelineItem() {
        let entry = Components.Schemas.PendingApprovalEntry(
            createdAt: "2026-07-04T10:00:00Z",
            description: "Write",
            parameters: "{}",
            requestId: "req-9",
            risk: .high,
            threadId: "t1",
            toolName: "fs_write")
        let item = GatewayMapping.timelineItem(from: entry)
        #expect(item.id == MessageID("approval-req-9"))
        #expect(item.threadID == ThreadID("t1"))
        guard case .approval(let request) = item.kind else {
            Issue.record("expected an approval row")
            return
        }
        #expect(request.requestID == "req-9")
        #expect(request.risk == .high)
    }

    // MARK: - Send + timestamps

    @Test("send response maps to a MessageID")
    func mapsSendResponse() {
        let response = Components.Schemas.SendMessageResponse(messageId: "m-42", status: "accepted")
        #expect(GatewayMapping.messageID(from: response) == MessageID("m-42"))
    }

    @Test("timestamps parse with and without fractional seconds")
    func parsesTimestamps() {
        let withFraction = GatewayMapping.date("2026-07-04T10:00:00.123Z")
        let plain = GatewayMapping.date("2026-07-04T10:00:00Z")
        #expect(withFraction.timeIntervalSince1970 > plain.timeIntervalSince1970)
        #expect(abs(withFraction.timeIntervalSince1970 - plain.timeIntervalSince1970 - 0.123) < 0.001)
        // An unparseable value degrades to the epoch, never a crash.
        #expect(GatewayMapping.date("not-a-date") == Date(timeIntervalSince1970: 0))
    }
}
