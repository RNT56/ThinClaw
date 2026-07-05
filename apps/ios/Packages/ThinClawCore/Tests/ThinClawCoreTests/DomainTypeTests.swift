import Foundation
import Testing

@testable import ThinClawCore

@Suite("Identifiers")
struct IdentifierTests {
    @Test("ThreadID and MessageID encode as bare JSON strings")
    func identifiersEncodeAsStrings() throws {
        let encoded = try JSONEncoder().encode(ThreadID("th_42"))
        #expect(String(decoding: encoded, as: UTF8.self) == "\"th_42\"")

        let decoded = try JSONDecoder().decode(ThreadID.self, from: Data("\"th_42\"".utf8))
        #expect(decoded == ThreadID("th_42"))
    }

    @Test("MessageID() generates unique ids")
    func messageIDUniqueness() {
        #expect(MessageID() != MessageID())
    }
}

@Suite("AgentEvent")
struct AgentEventTests {
    @Test("threadID accessor covers every payload-bearing case")
    func threadIDAccessor() {
        let id = ThreadID("t")
        let approval = ApprovalRequest(
            requestID: "r", toolName: "shell", description: "d", parameters: "{}", threadID: id)

        #expect(AgentEvent.streamChunk(content: "", threadID: id).threadID == id)
        #expect(AgentEvent.response(content: "", threadID: id).threadID == id)
        #expect(AgentEvent.thinking(message: "", threadID: id).threadID == id)
        #expect(AgentEvent.toolStarted(name: "", threadID: id).threadID == id)
        #expect(AgentEvent.toolCompleted(name: "", success: true, threadID: id).threadID == id)
        #expect(AgentEvent.approvalNeeded(approval).threadID == id)
        #expect(
            AgentEvent.usageUpdate(UsageUpdate(inputTokens: 1, outputTokens: 1, threadID: id))
                .threadID == id)
        #expect(AgentEvent.error(message: "", threadID: id).threadID == id)
        #expect(AgentEvent.heartbeat.threadID == nil)
        #expect(AgentEvent.unknown(type: "plan_update").threadID == nil)
    }
}

@Suite("TimelineItem")
struct TimelineItemTests {
    @Test("timeline items round-trip through Codable")
    func timelineItemRoundTrip() throws {
        let items: [TimelineItem] = [
            TimelineItem(
                threadID: ThreadID("t"),
                timestamp: Date(timeIntervalSince1970: 1_750_000_000),
                kind: .userMessage(text: "hi")),
            TimelineItem(
                threadID: ThreadID("t"),
                timestamp: Date(timeIntervalSince1970: 1_750_000_001),
                kind: .toolCall(name: "shell_command", status: .succeeded)),
            TimelineItem(
                threadID: ThreadID("t"),
                timestamp: Date(timeIntervalSince1970: 1_750_000_002),
                kind: .approval(
                    ApprovalRequest(
                        requestID: "req_1", toolName: "shell_command",
                        description: "Run a command", parameters: "{\"command\":\"ls\"}"))),
        ]

        let data = try JSONEncoder().encode(items)
        let decoded = try JSONDecoder().decode([TimelineItem].self, from: data)
        #expect(decoded == items)
    }
}

@Suite("ConnectionState")
struct ConnectionStateTests {
    @Test("only connected is live")
    func isLive() {
        #expect(ConnectionState.connected.isLive)
        #expect(!ConnectionState.idle.isLive)
        #expect(!ConnectionState.connecting.isLive)
        #expect(!ConnectionState.reconnecting(attempt: 3).isLive)
        #expect(!ConnectionState.failed(message: "x").isLive)
    }
}
