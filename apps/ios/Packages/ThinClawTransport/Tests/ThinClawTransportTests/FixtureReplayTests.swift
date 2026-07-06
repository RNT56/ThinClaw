import Foundation
import Testing
import ThinClawCore

@testable import ThinClawTransport

/// Replays hand-written fixtures of realistic gateway event streams through
/// the parser at adversarial chunkings, and decodes them into `AgentEvent`s.
@Suite("Fixture replay")
struct FixtureReplayTests {
    let decoder = AgentEventDecoder()

    private func decodeAll(_ events: [ServerSentEvent]) throws -> [AgentEvent] {
        try events.map { try decoder.decode($0) }
    }

    @Test("every fixture parses identically at every adversarial chunking", arguments: Fixture.allCases)
    func chunkingInvariance(fixture: Fixture) throws {
        let bytes = try fixture.bytes()
        let reference = parseChunked(bytes, size: .max)
        #expect(!reference.isEmpty, "fixture \(fixture.rawValue) produced no events")
        for size in adversarialChunkSizes {
            #expect(
                parseChunked(bytes, size: size) == reference,
                "chunk size \(size) diverged for \(fixture.rawValue)")
        }
    }

    @Test("every fixture parses identically with CRLF line endings", arguments: Fixture.allCases)
    func crlfInvariance(fixture: Fixture) throws {
        let reference = parseChunked(try fixture.bytes(), size: .max)
        let crlf = try fixture.crlfBytes()
        for size in [1, 7, .max] {
            #expect(
                parseChunked(crlf, size: size) == reference,
                "CRLF at chunk size \(size) diverged for \(fixture.rawValue)")
        }
    }

    @Test("basic stream decodes to the expected agent events")
    func basicStreamDecodes() throws {
        let thread = ThreadID("web-1720000000")
        let events = try decodeAll(parseChunked(try Fixture.basic.bytes(), size: 3))
        #expect(
            events == [
                .streamChunk(content: "Hel", threadID: thread),
                .streamChunk(content: "lo! I ca", threadID: thread),
                .streamChunk(content: "n help with that.", threadID: thread),
                .usageUpdate(
                    UsageUpdate(
                        inputTokens: 812, outputTokens: 64, costUSD: 0.0042,
                        model: "claude-sonnet-4-5", threadID: thread)),
                .response(content: "Hello! I can help with that.", threadID: thread),
                .heartbeat,
            ])
    }

    @Test("tool/approval stream decodes lifecycle, approval, and error")
    func toolsApprovalStreamDecodes() throws {
        let thread = ThreadID("web-1720000001")
        let events = try decodeAll(
            parseChunked(try Fixture.toolsApproval.bytes(), size: 5))
        #expect(
            events == [
                .thinking(message: "Checking the workspace...", threadID: thread),
                .toolStarted(name: "read_file", threadID: thread),
                .toolCompleted(name: "read_file", success: true, threadID: thread),
                .approvalNeeded(
                    ApprovalRequest(
                        requestID: "appr_7f3a",
                        toolName: "shell_command",
                        description: "Run a shell command",
                        parameters: #"{"command":"rm -rf /tmp/scratch"}"#,
                        risk: .high,
                        threadID: thread)),
                .heartbeat,
                .toolStarted(name: "shell_command", threadID: thread),
                .toolCompleted(name: "shell_command", success: false, threadID: thread),
                .error(message: "shell_command failed: exit status 1", threadID: thread),
            ])
    }

    @Test("unknown event types pass through as .unknown without derailing the stream")
    func unknownEventsPassThrough() throws {
        let events = try decodeAll(
            parseChunked(try Fixture.unknownAndComments.bytes(), size: 1))
        #expect(
            events == [
                .unknown(type: "plan_update"),
                .unknown(type: "subagent_spawned"),
                .streamChunk(
                    content: "still works after unknowns",
                    threadID: ThreadID("web-1720000002")),
                .response(
                    content: "still works after unknowns",
                    threadID: ThreadID("web-1720000002")),
            ])
    }

    @Test("retry and id fields in the fixture reach parser state")
    func retryAndIDState() throws {
        var parser = SSEParser()
        _ = parser.feed(try Fixture.unknownAndComments.bytes())
        #expect(parser.reconnectionTime == .milliseconds(5000))
        #expect(parser.lastEventID == "evt-002")
    }

    @Test("multibyte UTF-8 fixture survives byte-by-byte replay")
    func utf8FixtureByteByByte() throws {
        let events = try decodeAll(parseChunked(try Fixture.utf8.bytes(), size: 1))
        guard case .response(let content, _) = events.last else {
            Issue.record("expected trailing response event")
            return
        }
        #expect(content == "héllo → 🦀🧵 日本語テキスト and naïve étude")

        let chunks: [String] = events.compactMap {
            if case .streamChunk(let content, _) = $0 { return content }
            return nil
        }
        #expect(chunks.joined() == content)
    }
}
