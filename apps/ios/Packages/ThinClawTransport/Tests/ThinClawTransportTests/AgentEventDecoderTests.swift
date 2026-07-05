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

    @Test("approval_needed with risk:high decodes into a high-risk ApprovalRequest")
    func approvalNeededHigh() throws {
        let event = try decode(
            #"""
            {"type":"approval_needed","request_id":"appr_7f3a","tool_name":"shell_command","description":"Run a shell command","parameters":"{\"command\":\"ls\"}","risk":"high","thread_id":"web-1"}
            """#)
        #expect(
            event
                == .approvalNeeded(
                    ApprovalRequest(
                        requestID: "appr_7f3a",
                        toolName: "shell_command",
                        description: "Run a shell command",
                        parameters: #"{"command":"ls"}"#,
                        risk: .high,
                        threadID: ThreadID("web-1"))))
    }

    @Test("approval_needed with risk:low decodes into a low-risk ApprovalRequest")
    func approvalNeededLow() throws {
        let event = try decode(
            #"""
            {"type":"approval_needed","request_id":"appr_7f3a","tool_name":"read_file","description":"Read a file","parameters":"{\"path\":\"README.md\"}","risk":"low","thread_id":"web-1"}
            """#)
        guard case let .approvalNeeded(request) = event else {
            Issue.record("expected .approvalNeeded, got \(event)")
            return
        }
        #expect(request.risk == .low)
    }

    @Test("approval_needed missing risk defaults to high (safe default, D-K3)")
    func approvalNeededMissingRiskDefaultsHigh() throws {
        // A payload with no `risk` key must never silently downgrade: an
        // unknown/absent tier decodes to `.high` so the biometric gate holds.
        let event = try decode(
            #"""
            {"type":"approval_needed","request_id":"appr_7f3a","tool_name":"shell_command","description":"Run a shell command","parameters":"{\"command\":\"ls\"}","thread_id":"web-1"}
            """#)
        guard case let .approvalNeeded(request) = event else {
            Issue.record("expected .approvalNeeded, got \(event)")
            return
        }
        #expect(request.risk == .high)
    }

    @Test("approval_needed with an unknown risk string defaults to high")
    func approvalNeededUnknownRiskDefaultsHigh() throws {
        let event = try decode(
            #"""
            {"type":"approval_needed","request_id":"appr_7f3a","tool_name":"shell_command","description":"Run a shell command","parameters":"{}","risk":"nuclear","thread_id":"web-1"}
            """#)
        guard case let .approvalNeeded(request) = event else {
            Issue.record("expected .approvalNeeded, got \(event)")
            return
        }
        #expect(request.risk == .high)
    }

    @Test("auth_required decodes into an AuthPrompt with a parsed URL")
    func authRequired() throws {
        let event = try decode(
            #"""
            {"type":"auth_required","extension_name":"gmail","instructions":"Authorize access","auth_url":"https://accounts.example.com/oauth","auth_mode":"oauth","auth_status":"pending","thread_id":"web-1"}
            """#)
        guard case let .authRequired(prompt) = event else {
            Issue.record("expected .authRequired, got \(event)")
            return
        }
        #expect(prompt.extensionName == "gmail")
        #expect(prompt.instructions == "Authorize access")
        #expect(prompt.authURL == URL(string: "https://accounts.example.com/oauth"))
        #expect(prompt.threadID == ThreadID("web-1"))
    }

    @Test("auth_required tolerates a missing auth_url (text-only card)")
    func authRequiredWithoutURL() throws {
        let event = try decode(
            #"""
            {"type":"auth_required","extension_name":"gmail","auth_mode":"device_flow","auth_status":"pending"}
            """#)
        guard case let .authRequired(prompt) = event else {
            Issue.record("expected .authRequired, got \(event)")
            return
        }
        #expect(prompt.authURL == nil)
        #expect(prompt.instructions == nil)
    }

    @Test("credential_prompt decodes into a CredentialPrompt (no secret carried)")
    func credentialPrompt() throws {
        let event = try decode(
            #"""
            {"type":"credential_prompt","prompt_id":"cp_1","secret_name":"GITHUB_TOKEN","provider":"github","reason":"clone a private repo","thread_id":"web-1"}
            """#)
        guard case let .credentialPrompt(prompt) = event else {
            Issue.record("expected .credentialPrompt, got \(event)")
            return
        }
        #expect(prompt.promptID == "cp_1")
        #expect(prompt.secretName == "GITHUB_TOKEN")
        #expect(prompt.provider == "github")
        #expect(prompt.reason == "clone a private repo")
        #expect(prompt.threadID == ThreadID("web-1"))
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
        for type in ["plan_update", "subagent_spawned", "job_started", "totally_new_thing"] {
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
