import Foundation
import Testing
import ThinClawCore

@testable import ThinClawTransport

@Suite("AgentEventDecoder")
struct AgentEventDecoderTests {
    let decoder = AgentEventDecoder()

    private func decode(_ json: String) throws -> AgentEvent {
        try decoder.decode(json: Data(json.utf8))
    }

    @Test("stream_chunk decodes with thread id")
    func streamChunk() throws {
        let event = try decode(
            #"{"type":"stream_chunk","content":"Hel","thread_id":"web-1"}"#)
        #expect(event == .streamChunk(content: "Hel", threadID: ThreadID("web-1")))
    }

    @Test("stream_chunk tolerates a missing optional thread_id")
    func streamChunkWithoutThread() throws {
        let event = try decode(#"{"type":"stream_chunk","content":"x"}"#)
        #expect(event == .streamChunk(content: "x", threadID: nil))
    }

    @Test("response decodes")
    func response() throws {
        let event = try decode(
            #"{"type":"response","content":"Done.","thread_id":"web-1"}"#)
        #expect(event == .response(content: "Done.", threadID: ThreadID("web-1")))
    }

    @Test("thinking decodes")
    func thinking() throws {
        let event = try decode(
            #"{"type":"thinking","message":"Reading files...","thread_id":"web-1"}"#)
        #expect(event == .thinking(message: "Reading files...", threadID: ThreadID("web-1")))
    }

    @Test("tool_started and tool_completed decode")
    func toolLifecycle() throws {
        let started = try decode(
            #"{"type":"tool_started","name":"shell_command","thread_id":"web-1"}"#)
        #expect(started == .toolStarted(name: "shell_command", threadID: ThreadID("web-1")))

        let completed = try decode(
            #"{"type":"tool_completed","name":"shell_command","success":false,"thread_id":"web-1"}"#
        )
        #expect(
            completed
                == .toolCompleted(name: "shell_command", success: false, threadID: ThreadID("web-1")))
    }

    @Test("approval_needed decodes into an ApprovalRequest")
    func approvalNeeded() throws {
        let event = try decode(
            #"""
            {"type":"approval_needed","request_id":"appr_7f3a","tool_name":"shell_command","description":"Run a shell command","parameters":"{\"command\":\"ls\"}","thread_id":"web-1"}
            """#)
        #expect(
            event
                == .approvalNeeded(
                    ApprovalRequest(
                        requestID: "appr_7f3a",
                        toolName: "shell_command",
                        description: "Run a shell command",
                        parameters: #"{"command":"ls"}"#,
                        threadID: ThreadID("web-1"))))
    }

    @Test("usage_update decodes with optional cost and model")
    func usageUpdate() throws {
        let full = try decode(
            #"{"type":"usage_update","input_tokens":812,"output_tokens":64,"cost_usd":0.0042,"model":"claude-sonnet-4-5","thread_id":"web-1"}"#
        )
        #expect(
            full
                == .usageUpdate(
                    UsageUpdate(
                        inputTokens: 812, outputTokens: 64, costUSD: 0.0042,
                        model: "claude-sonnet-4-5", threadID: ThreadID("web-1"))))

        let minimal = try decode(
            #"{"type":"usage_update","input_tokens":1,"output_tokens":2}"#)
        #expect(minimal == .usageUpdate(UsageUpdate(inputTokens: 1, outputTokens: 2)))
    }

    @Test("heartbeat decodes from its bare envelope")
    func heartbeat() throws {
        #expect(try decode(#"{"type":"heartbeat"}"#) == .heartbeat)
    }

    @Test("error decodes")
    func errorEvent() throws {
        let event = try decode(
            #"{"type":"error","message":"provider timeout","thread_id":"web-1"}"#)
        #expect(event == .error(message: "provider timeout", threadID: ThreadID("web-1")))
    }

    @Test("unknown event types decode to .unknown, never throw")
    func unknownTypes() throws {
        for type in ["plan_update", "subagent_spawned", "auth_required", "totally_new_thing"] {
            let event = try decode(#"{"type":"\#(type)","anything":123}"#)
            #expect(event == .unknown(type: type))
        }
    }

    @Test("malformed JSON throws malformedJSON")
    func malformedJSON() {
        #expect(throws: AgentEventDecodingError.malformedJSON) {
            _ = try decode("not json at all")
        }
    }

    @Test("missing type discriminator throws")
    func missingType() {
        #expect(throws: AgentEventDecodingError.missingTypeDiscriminator) {
            _ = try decode(#"{"content":"orphan"}"#)
        }
    }

    @Test("known type with broken payload throws invalidPayload")
    func invalidPayload() {
        #expect(throws: AgentEventDecodingError.invalidPayload(type: "stream_chunk")) {
            _ = try decode(#"{"type":"stream_chunk","content":42}"#)
        }
    }

    @Test("decodes straight from a ServerSentEvent")
    func decodeFromServerSentEvent() throws {
        let sse = ServerSentEvent(data: #"{"type":"heartbeat"}"#)
        #expect(try decoder.decode(sse) == .heartbeat)
    }
}
