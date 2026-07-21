import Foundation
import Testing

@testable import ThinClawSnapshotKit

/// Millisecond-precision date helper: the store encodes ISO-8601 with
/// fractional (ms) precision, so fixtures use ms-representable dates for
/// exact Equatable round-trips.
private func date(_ secondsSince1970: Double) -> Date {
    Date(timeIntervalSince1970: (secondsSince1970 * 1000).rounded() / 1000)
}

private func makeTempStore() throws -> SnapshotStore {
    let dir = FileManager.default.temporaryDirectory
        .appendingPathComponent("thinclaw-snapshot-tests-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return SnapshotStore(baseURL: dir)
}

@Suite("SnapshotStore round-trips")
struct SnapshotRoundTripTests {
    @Test("AgentStatusSnapshot round-trips exactly")
    func agentStatusRoundTrip() throws {
        let store = try makeTempStore()
        let snapshot = AgentStatusSnapshot(
            generatedAt: date(1_750_000_000.125),
            phase: .runningTool,
            activeToolName: "shell_command",
            activeThreadID: "web-1720000000",
            activeThreadTitle: "Fix the flaky test",
            unreadCount: 3)

        try store.save(snapshot)
        #expect(try store.load(AgentStatusSnapshot.self) == snapshot)
    }

    @Test("gateway-scoped stale metadata round-trips")
    func gatewayMetadataRoundTrip() throws {
        let store = try makeTempStore()
        let snapshot = AgentStatusSnapshot(
            gatewayInstanceID: "gateway-a",
            stale: true,
            generatedAt: date(1_750_000_000),
            phase: .idle)

        try store.save(snapshot)
        let loaded = try store.load(AgentStatusSnapshot.self)

        #expect(loaded?.gatewayInstanceID == "gateway-a")
        #expect(loaded?.isKnownStale == true)
    }

    @Test("PendingApprovalsSnapshot round-trips exactly")
    func pendingApprovalsRoundTrip() throws {
        let store = try makeTempStore()
        let snapshot = PendingApprovalsSnapshot(
            generatedAt: date(1_750_000_001),
            approvals: [
                .init(
                    id: "appr_1", toolName: "shell_command",
                    description: "Run rm -rf /tmp/scratch",
                    threadID: "web-1", requestedAt: date(1_750_000_000)),
                .init(
                    id: "appr_2", toolName: "http_request",
                    description: "POST to production", requestedAt: date(1_750_000_000.5)),
            ])

        try store.save(snapshot)
        #expect(try store.load(PendingApprovalsSnapshot.self) == snapshot)
    }

    @Test("JobsSnapshot round-trips exactly")
    func jobsRoundTrip() throws {
        let store = try makeTempStore()
        let snapshot = JobsSnapshot(
            generatedAt: date(1_750_000_002),
            jobs: [
                .init(
                    id: "job_1", title: "Nightly refactor", phase: .running,
                    startedAt: date(1_749_999_000))
            ])

        try store.save(snapshot)
        #expect(try store.load(JobsSnapshot.self) == snapshot)
    }

    @Test("QuickAskReceipt round-trips exactly")
    func quickAskRoundTrip() throws {
        let store = try makeTempStore()
        let receipt = QuickAskReceipt(
            generatedAt: date(1_750_000_003),
            text: "What's on my calendar today?",
            threadID: nil,
            deliveryState: .queued)

        try store.save(receipt)
        #expect(try store.load(QuickAskReceipt.self) == receipt)
    }

    @Test("snapshots of different types live in separate files")
    func distinctFiles() throws {
        let store = try makeTempStore()
        try store.save(AgentStatusSnapshot(generatedAt: date(1), phase: .idle))
        try store.save(JobsSnapshot(generatedAt: date(2), jobs: []))

        #expect(
            store.fileURL(for: AgentStatusSnapshot.self)
                != store.fileURL(for: JobsSnapshot.self))
        #expect(try store.load(AgentStatusSnapshot.self)?.phase == .idle)
        #expect(try store.load(JobsSnapshot.self)?.jobs.isEmpty == true)
    }

    @Test("save overwrites the previous snapshot")
    func saveOverwrites() throws {
        let store = try makeTempStore()
        try store.save(AgentStatusSnapshot(generatedAt: date(1), phase: .idle))
        try store.save(AgentStatusSnapshot(generatedAt: date(2), phase: .streaming))

        let loaded = try store.load(AgentStatusSnapshot.self)
        #expect(loaded?.phase == .streaming)
        #expect(loaded?.generatedAt == date(2))
    }
}

@Suite("SnapshotStore edge cases")
struct SnapshotStoreEdgeCaseTests {
    @Test("loading a never-written snapshot returns nil")
    func missingFileReturnsNil() throws {
        let store = try makeTempStore()
        #expect(try store.load(AgentStatusSnapshot.self) == nil)
    }

    @Test("remove deletes the file; removing twice is fine")
    func removeIsIdempotent() throws {
        let store = try makeTempStore()
        try store.save(AgentStatusSnapshot(generatedAt: date(1), phase: .idle))
        try store.remove(AgentStatusSnapshot.self)
        #expect(try store.load(AgentStatusSnapshot.self) == nil)
        try store.remove(AgentStatusSnapshot.self)
    }

    @Test("a newer schema version on disk is rejected, not mis-decoded")
    func newerSchemaRejected() throws {
        let store = try makeTempStore()
        var snapshot = AgentStatusSnapshot(generatedAt: date(1), phase: .idle)
        snapshot.schemaVersion = AgentStatusSnapshot.currentSchemaVersion + 5
        try store.save(snapshot)

        #expect(
            throws: SnapshotStoreError.unsupportedSchemaVersion(
                found: AgentStatusSnapshot.currentSchemaVersion + 5,
                supported: AgentStatusSnapshot.currentSchemaVersion)
        ) {
            _ = try store.load(AgentStatusSnapshot.self)
        }
    }

    @Test("garbage on disk surfaces as corruptSnapshot")
    func corruptFile() throws {
        let store = try makeTempStore()
        try FileManager.default.createDirectory(
            at: store.baseURL, withIntermediateDirectories: true)
        try Data("{ not json".utf8).write(
            to: store.fileURL(for: AgentStatusSnapshot.self))

        #expect(
            throws: SnapshotStoreError.corruptSnapshot(
                fileName: AgentStatusSnapshot.fileName)
        ) {
            _ = try store.load(AgentStatusSnapshot.self)
        }
    }

    @Test("valid JSON missing snapshot fields is corrupt, not a crash")
    func wrongShapeFile() throws {
        let store = try makeTempStore()
        try FileManager.default.createDirectory(
            at: store.baseURL, withIntermediateDirectories: true)
        try Data(#"{"schemaVersion":1}"#.utf8).write(
            to: store.fileURL(for: AgentStatusSnapshot.self))

        #expect(
            throws: SnapshotStoreError.corruptSnapshot(
                fileName: AgentStatusSnapshot.fileName)
        ) {
            _ = try store.load(AgentStatusSnapshot.self)
        }
    }

    @Test("snapshot files are stable, sorted, human-inspectable JSON")
    func stableEncoding() throws {
        let store = try makeTempStore()
        let snapshot = AgentStatusSnapshot(generatedAt: date(1_750_000_000), phase: .thinking)
        try store.save(snapshot)
        let first = try Data(contentsOf: store.fileURL(for: AgentStatusSnapshot.self))
        try store.save(snapshot)
        let second = try Data(contentsOf: store.fileURL(for: AgentStatusSnapshot.self))

        #expect(first == second, "same snapshot must serialize byte-identically")
        let text = String(decoding: first, as: UTF8.self)
        #expect(text.contains("\"schemaVersion\""))
        #expect(text.contains("2025-06-15T"), "dates must be ISO-8601")
    }
}
