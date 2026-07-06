import Foundation
import Testing
import ThinClawSnapshotKit

@testable import ThinClawCore

/// Millisecond-precision date helper matching SnapshotKit's ISO-8601(ms)
/// encoding, so any snapshot that round-trips through the store compares
/// exactly.
private func date(_ secondsSince1970: Double) -> Date {
    Date(timeIntervalSince1970: (secondsSince1970 * 1000).rounded() / 1000)
}

@Suite("SnapshotInputs projection + privacy")
struct SnapshotProjectionTests {
    @Test("Projects agent status fields verbatim, stamping generatedAt")
    func projectsAgentStatus() {
        let inputs = SnapshotInputs(
            phase: .runningTool,
            activeToolName: "shell_command",
            activeThreadID: ThreadID("web-42"),
            activeThreadTitle: "Fix the flaky test",
            unreadCount: 2)

        let out = inputs.project(at: date(1_750_000_000), privacy: .default)

        #expect(out.status.phase == .runningTool)
        #expect(out.status.activeToolName == "shell_command")
        #expect(out.status.activeThreadID == "web-42")
        #expect(out.status.activeThreadTitle == "Fix the flaky test")
        #expect(out.status.unreadCount == 2)
        #expect(out.status.generatedAt == date(1_750_000_000))
    }

    @Test("Clamps a negative unread count to zero")
    func clampsUnread() {
        let inputs = SnapshotInputs(unreadCount: -5)
        let out = inputs.project(at: date(1), privacy: .default)
        #expect(out.status.unreadCount == 0)
    }

    @Test("Maps approvals including risk tier and thread id")
    func mapsApprovals() {
        let inputs = SnapshotInputs(pendingApprovals: [
            ApprovalRequest(
                requestID: "r1", toolName: "write_file", description: "Write /etc/hosts",
                parameters: "{}", risk: .high, threadID: ThreadID("t1")),
            ApprovalRequest(
                requestID: "r2", toolName: "read_file", description: "Read README",
                parameters: "{}", risk: .low),
        ])

        let out = inputs.project(at: date(10), privacy: .default)

        #expect(out.approvals.approvals.count == 2)
        let first = out.approvals.approvals[0]
        #expect(first.id == "r1")
        #expect(first.toolName == "write_file")
        #expect(first.description == "Write /etc/hosts")
        #expect(first.threadID == "t1")
        #expect(first.risk == .high)
        #expect(first.requestedAt == date(10))
        #expect(out.approvals.approvals[1].risk == .low)
        #expect(out.approvals.approvals[1].threadID == nil)
    }

    @Test("Maps jobs with phase, title, and startedAt")
    func mapsJobs() {
        let inputs = SnapshotInputs(jobs: [
            .init(id: "j1", title: "Index repo", phase: .running, startedAt: date(5))
        ])
        let out = inputs.project(at: date(9), privacy: .default)
        #expect(out.jobs.jobs.count == 1)
        #expect(out.jobs.jobs[0].id == "j1")
        #expect(out.jobs.jobs[0].title == "Index repo")
        #expect(out.jobs.jobs[0].phase == .running)
        #expect(out.jobs.jobs[0].startedAt == date(5))
    }

    // MARK: - Privacy

    @Test("Truncates an over-limit preview with an ellipsis, never exceeding the limit")
    func truncatesPreview() {
        let policy = SnapshotPrivacyPolicy(previewsEnabled: true, previewCharacterLimit: 10)
        let inputs = SnapshotInputs(activeThreadTitle: "0123456789ABCDEF")
        let out = inputs.project(at: date(1), privacy: policy)
        let title = try! #require(out.status.activeThreadTitle)
        #expect(title.count == 10)
        #expect(title == "012345678\u{2026}")
    }

    @Test("Leaves an at-limit preview intact")
    func keepsShortPreview() {
        let policy = SnapshotPrivacyPolicy(previewsEnabled: true, previewCharacterLimit: 5)
        let inputs = SnapshotInputs(activeThreadTitle: "12345")
        let out = inputs.project(at: date(1), privacy: policy)
        #expect(out.status.activeThreadTitle == "12345")
    }

    @Test("Redacted policy drops titles and descriptions but keeps enums/ids/risk")
    func redactedDropsProse() {
        let inputs = SnapshotInputs(
            phase: .waitingForApproval,
            activeThreadTitle: "Secret thread title",
            pendingApprovals: [
                ApprovalRequest(
                    requestID: "r1", toolName: "run_shell",
                    description: "rm -rf things", parameters: "{}", risk: .high,
                    threadID: ThreadID("t1"))
            ],
            jobs: [.init(id: "j1", title: "Private job name", phase: .running, startedAt: date(1))])

        let out = inputs.project(at: date(1), privacy: .redacted)

        // Prose gone…
        #expect(out.status.activeThreadTitle == nil)
        #expect(out.approvals.approvals[0].description == "")
        // …but structural fields survive.
        #expect(out.status.phase == .waitingForApproval)
        #expect(out.approvals.approvals[0].toolName == "run_shell")
        #expect(out.approvals.approvals[0].risk == .high)
        #expect(out.approvals.approvals[0].threadID == "t1")
        #expect(out.jobs.jobs[0].id == "j1")
        #expect(out.jobs.jobs[0].phase == .running)
        // A dropped job title falls back to a generic placeholder, never empty.
        #expect(out.jobs.jobs[0].title == "Job")
    }

    @Test("Blank/whitespace previews resolve to nil")
    func blankPreviewIsNil() {
        let inputs = SnapshotInputs(activeThreadTitle: "   \n ")
        let out = inputs.project(at: date(1), privacy: .default)
        #expect(out.status.activeThreadTitle == nil)
    }
}
