#if os(iOS) && canImport(ActivityKit)
    import Foundation
    import Testing

    @testable import ThinClawSnapshotKit

    /// Verifies the Live Activity `ContentState` decodes the *exact* wire shape
    /// the gateway's push policy emits. `push_policy.rs` builds `content-state`
    /// as `{"phase":"<bare string>","revision":N[,"progress":P]}`, so `RunPhase`
    /// must have `String` raw values — a raw-value-less enum would expect
    /// `{"phase":{"runningTool":{}}}` and fail to decode the real payload.
    @Suite("AgentRunAttributes.ContentState wire decode")
    struct AgentRunAttributesTests {
        /// The precise JSON `push_policy::content_state` produces for a running
        /// tool update at revision 2 (no progress).
        @Test("decodes {\"phase\":\"runningTool\",\"revision\":2}")
        func decodesRunningToolPhase() throws {
            let json = Data(#"{"phase":"runningTool","revision":2}"#.utf8)
            let state = try JSONDecoder().decode(
                AgentRunAttributes.ContentState.self, from: json)
            #expect(state.phase == .runningTool)
            #expect(state.revision == 2)
            #expect(state.progress == nil)
            #expect(state.toolName == nil)
        }

        /// The same shape with a progress field, as emitted when a run reports
        /// progress.
        @Test("decodes phase + revision + progress")
        func decodesWithProgress() throws {
            let json = Data(#"{"phase":"thinking","revision":5,"progress":42}"#.utf8)
            let state = try JSONDecoder().decode(
                AgentRunAttributes.ContentState.self, from: json)
            #expect(state.phase == .thinking)
            #expect(state.revision == 5)
            #expect(state.progress == 42)
        }

        /// Every phase the backend can send round-trips through its bare-string
        /// raw value.
        @Test("every backend phase string decodes")
        func everyPhaseDecodes() throws {
            let cases: [(String, AgentRunAttributes.ContentState.RunPhase)] = [
                ("thinking", .thinking),
                ("runningTool", .runningTool),
                ("awaitingApproval", .awaitingApproval),
                ("done", .done),
                ("failed", .failed),
            ]
            for (raw, expected) in cases {
                let json = Data(#"{"phase":"\#(raw)","revision":1}"#.utf8)
                let state = try JSONDecoder().decode(
                    AgentRunAttributes.ContentState.self, from: json)
                #expect(state.phase == expected)
            }
        }
    }
#endif
