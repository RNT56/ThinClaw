import Foundation
import Testing

@testable import ThinClawCore

@Suite("TimelineAccessibility")
struct TimelineAccessibilityTests {
    private func a(_ kind: TimelineItem.Kind) -> TimelineAccessibility {
        kind.accessibility
    }

    @Test("user and agent messages are attributed by speaker")
    func attribution() {
        #expect(a(.userMessage(text: "hi there")).label == "You said hi there")
        #expect(a(.agentMessage(text: "done")).label == "Agent said done")
    }

    @Test("a streaming reply keeps a stable label and moves prose into value")
    func streamingUsesValueForPoliteUpdates() {
        let desc = a(.streamingAgentMessage(text: "partial answer"))
        #expect(desc.label == "Agent is responding")
        #expect(desc.value == "partial answer")
        // A final agent message, by contrast, carries no separate value.
        #expect(a(.agentMessage(text: "partial answer")).value == nil)
    }

    @Test("tool activity announces name and lifecycle phase")
    func toolLifecycle() {
        #expect(a(.toolCall(name: "shell", status: .running)).label == "Tool shell running")
        #expect(a(.toolCall(name: "shell", status: .succeeded)).label == "Tool shell succeeded")
        #expect(a(.toolCall(name: "shell", status: .failed)).label == "Tool shell failed")
    }

    @Test("approval cards announce tool and risk, and hint the action")
    func approvalRiskAndHint() {
        let high = ApprovalRequest(
            requestID: "1", toolName: "delete_repo", description: "d",
            parameters: "{}", risk: .high)
        let low = ApprovalRequest(
            requestID: "2", toolName: "read_file", description: "d",
            parameters: "{}", risk: .low)
        #expect(a(.approval(high)).label == "delete_repo approval, high risk")
        #expect(a(.approval(low)).label == "read_file approval, low risk")
        #expect(a(.approval(high)).hint != nil)
    }

    @Test("auth prompt hints opening only when an auth URL is present")
    func authPromptHint() {
        let withURL = AuthPrompt(extensionName: "gmail", authURL: URL(string: "https://x"))
        let withoutURL = AuthPrompt(extensionName: "gmail")
        #expect(a(.authPrompt(withURL)).label == "gmail needs authorization")
        #expect(a(.authPrompt(withURL)).hint != nil)
        #expect(a(.authPrompt(withoutURL)).hint == nil)
    }

    @Test("credential prompt tells the operator to handle on desktop (D-T4)")
    func credentialPromptHandoff() {
        let prompt = CredentialPrompt(
            promptID: "p", secretName: "TOKEN", provider: "github", reason: "r")
        #expect(a(.credentialPrompt(prompt)).label == "github needs a credential. Handle on desktop")
    }

    @Test("failure rows announce the error and hint retry")
    func failureRetryHint() {
        let desc = a(.failure(message: "network error"))
        #expect(desc.label == "Failed. network error")
        #expect(desc.hint == "Double tap to retry")
    }

    @Test("the descriptor is reachable from a whole TimelineItem")
    func reachableFromItem() {
        let item = TimelineItem(
            threadID: ThreadID("t"), timestamp: Date(),
            kind: .agentMessage(text: "ok"))
        #expect(item.accessibility.label == "Agent said ok")
    }
}
