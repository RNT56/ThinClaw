import Foundation
import GRDB
import ThinClawCore

/// Production ``TranscriptStoring`` backed by a WAL-mode GRDB `DatabasePool`.
///
/// This is the on-device cache the chat and sessions surfaces hydrate from for
/// instant first paint, plus the durable offline send-outbox. It is a **cache**,
/// not a source of truth: the gateway owns history, so the schema may be reset
/// and re-synced at any time without data loss.
///
/// ## App-process-only (docs/MOBILE_SECURITY.md, data at rest)
/// This database is opened **only inside the main app process**. Widgets, the
/// watch, and the Notification Service Extension never open it — they read the
/// far smaller, redacted `ThinClawSnapshotKit` App-Group snapshot files
/// instead. Keeping the database single-writer/single-process avoids WAL
/// cross-process coordination hazards and keeps the full transcript out of
/// extension sandboxes.
///
/// ## File protection
/// The database directory is created with
/// `NSFileProtectionCompleteUntilFirstUserAuthentication` on iOS
/// (docs/MOBILE_SECURITY.md, "Transcript cache") so the transcript is encrypted
/// at rest yet still readable for a background refresh after first unlock. The
/// API is iOS-only, so it is conditionalized out on macOS (where these tests
/// run).
///
/// ## Storage shape
/// `TimelineItem` and `OutboxMessage` are `Codable`, so rows store the JSON of
/// the domain value in a `payload` blob alongside the few columns needed for
/// keying and ordering. This deliberately avoids a parallel GRDB record type
/// (see the note on ``ThinClawCore/TimelineItem``): a new timeline kind never
/// requires a migration.
public final class GRDBTranscriptStore: TranscriptStoring {
    private let dbPool: DatabasePool

    /// Open (creating if needed) the transcript database at `url`.
    ///
    /// - Parameter url: The database file path. The containing directory is
    ///   created if missing and, on iOS, tagged
    ///   `CompleteUntilFirstUserAuthentication`.
    public init(path url: URL) throws {
        try Self.prepareDirectory(for: url)

        var configuration = Configuration()
        // WAL is the default for `DatabasePool`, but pin it explicitly: it is
        // load-bearing for concurrent reads during a write and is documented
        // behavior we rely on.
        configuration.prepareDatabase { db in
            try db.execute(sql: "PRAGMA journal_mode = WAL")
        }
        self.dbPool = try DatabasePool(path: url.path, configuration: configuration)
        try Self.migrator.migrate(dbPool)
    }

    /// Convenience: open the store at the conventional app-support location
    /// (`<AppSupport>/ThinClaw/transcripts.sqlite`). App-process-only.
    public static func atDefaultLocation(
        fileManager: FileManager = .default
    ) throws -> GRDBTranscriptStore {
        let base = try fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true)
        let dir = base.appendingPathComponent("ThinClaw", isDirectory: true)
        let dbURL = dir.appendingPathComponent("transcripts.sqlite", isDirectory: false)
        return try GRDBTranscriptStore(path: dbURL)
    }

    // MARK: - Directory + file protection

    private static func prepareDirectory(for url: URL) throws {
        let dir = url.deletingLastPathComponent()
        let fileManager = FileManager.default
        #if os(iOS)
            try fileManager.createDirectory(
                at: dir,
                withIntermediateDirectories: true,
                attributes: [
                    .protectionKey: FileProtectionType.completeUntilFirstUserAuthentication
                ])
        #else
            try fileManager.createDirectory(at: dir, withIntermediateDirectories: true)
        #endif
    }

    // MARK: - Migrations

    private static var migrator: DatabaseMigrator {
        var migrator = DatabaseMigrator()
        migrator.registerMigration("v1") { db in
            try db.create(table: "threads") { t in
                t.column("id", .text).primaryKey()
                // Sorted-by column, denormalized from the payload so the thread
                // list can order without decoding every row.
                t.column("updated_at", .double).notNull()
                t.column("payload", .blob).notNull()
            }
            try db.create(table: "timeline_items") { t in
                // Composite primary key (thread_id, item_id): items are unique
                // within a thread and upserted by id on reconcile.
                t.column("thread_id", .text).notNull()
                t.column("item_id", .text).notNull()
                t.column("timestamp", .double).notNull()
                t.column("payload", .blob).notNull()
                t.primaryKey(["thread_id", "item_id"])
            }
            // Order a thread's timeline by time without a full-table scan.
            try db.create(
                index: "idx_timeline_thread_time",
                on: "timeline_items",
                columns: ["thread_id", "timestamp"])
            try db.create(table: "outbox") { t in
                t.column("id", .text).primaryKey()
                t.column("queued_at", .double).notNull()
                t.column("payload", .blob).notNull()
            }
        }
        return migrator
    }

    // MARK: - Coding

    private static let encoder = JSONEncoder()
    private static let decoder = JSONDecoder()

    private static func encode<T: Encodable>(_ value: T) throws -> Data {
        try encoder.encode(value)
    }

    private static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        try decoder.decode(type, from: data)
    }

    // MARK: - Threads

    public func threads() async throws -> [ChatThread] {
        try await dbPool.read { db in
            let rows = try Row.fetchAll(
                db,
                sql: "SELECT payload FROM threads ORDER BY updated_at DESC")
            return try rows.map { row in
                try Self.decode(ChatThread.self, from: row["payload"])
            }
        }
    }

    public func upsert(thread: ChatThread) async throws {
        let payload = try Self.encode(thread)
        let updatedAt = thread.updatedAt.timeIntervalSince1970
        let id = thread.id.rawValue
        try await dbPool.write { db in
            try db.execute(
                sql: """
                    INSERT INTO threads (id, updated_at, payload)
                    VALUES (?, ?, ?)
                    ON CONFLICT(id) DO UPDATE SET
                        updated_at = excluded.updated_at,
                        payload = excluded.payload
                    """,
                arguments: [id, updatedAt, payload])
        }
    }

    public func deleteThread(_ id: ThreadID) async throws {
        let raw = id.rawValue
        try await dbPool.write { db in
            try db.execute(sql: "DELETE FROM threads WHERE id = ?", arguments: [raw])
            try db.execute(
                sql: "DELETE FROM timeline_items WHERE thread_id = ?", arguments: [raw])
        }
    }

    // MARK: - Timeline

    public func timeline(for thread: ThreadID) async throws -> [TimelineItem] {
        let raw = thread.rawValue
        return try await dbPool.read { db in
            let rows = try Row.fetchAll(
                db,
                sql: """
                    SELECT payload FROM timeline_items
                    WHERE thread_id = ?
                    ORDER BY timestamp ASC
                    """,
                arguments: [raw])
            return try rows.map { row in
                try Self.decode(TimelineItem.self, from: row["payload"])
            }
        }
    }

    public func replaceTimeline(_ items: [TimelineItem], for thread: ThreadID) async throws {
        let raw = thread.rawValue
        let encoded = try items.map {
            (id: $0.id.rawValue, timestamp: $0.timestamp.timeIntervalSince1970, payload: try Self.encode($0))
        }
        try await dbPool.write { db in
            try db.execute(
                sql: "DELETE FROM timeline_items WHERE thread_id = ?", arguments: [raw])
            for item in encoded {
                try db.execute(
                    sql: """
                        INSERT INTO timeline_items (thread_id, item_id, timestamp, payload)
                        VALUES (?, ?, ?, ?)
                        """,
                    arguments: [raw, item.id, item.timestamp, item.payload])
            }
        }
    }

    public func append(_ item: TimelineItem, to thread: ThreadID) async throws {
        let raw = thread.rawValue
        let id = item.id.rawValue
        let timestamp = item.timestamp.timeIntervalSince1970
        let payload = try Self.encode(item)
        try await dbPool.write { db in
            // Upsert on (thread_id, item_id): appending an item whose id already
            // exists (e.g. a streaming row finalizing to its server id) replaces
            // it rather than throwing on the composite primary key.
            try db.execute(
                sql: """
                    INSERT INTO timeline_items (thread_id, item_id, timestamp, payload)
                    VALUES (?, ?, ?, ?)
                    ON CONFLICT(thread_id, item_id) DO UPDATE SET
                        timestamp = excluded.timestamp,
                        payload = excluded.payload
                    """,
                arguments: [raw, id, timestamp, payload])
        }
    }

    // MARK: - Outbox

    public func enqueueOutbox(_ message: OutboxMessage) async throws {
        let id = message.id.uuidString
        let queuedAt = message.queuedAt.timeIntervalSince1970
        let payload = try Self.encode(message)
        try await dbPool.write { db in
            try db.execute(
                sql: """
                    INSERT INTO outbox (id, queued_at, payload)
                    VALUES (?, ?, ?)
                    ON CONFLICT(id) DO UPDATE SET
                        queued_at = excluded.queued_at,
                        payload = excluded.payload
                    """,
                arguments: [id, queuedAt, payload])
        }
    }

    public func outbox() async throws -> [OutboxMessage] {
        try await dbPool.read { db in
            let rows = try Row.fetchAll(
                db,
                // `id` is the tie-breaker so two messages enqueued in the same
                // millisecond flush in a stable, deterministic order.
                sql: "SELECT payload FROM outbox ORDER BY queued_at ASC, id ASC")
            return try rows.map { row in
                try Self.decode(OutboxMessage.self, from: row["payload"])
            }
        }
    }

    public func removeFromOutbox(_ id: UUID) async throws {
        let raw = id.uuidString
        try await dbPool.write { db in
            try db.execute(sql: "DELETE FROM outbox WHERE id = ?", arguments: [raw])
        }
    }

    // MARK: - Test support

    /// Whether each v1 table exists. Exposed for the migration-head test so it
    /// asserts the schema without reaching into GRDB internals from the test
    /// target.
    func appliedMigrationsAndTables() async throws -> (
        threads: Bool, timelineItems: Bool, outbox: Bool
    ) {
        try await dbPool.read { db in
            (
                threads: try db.tableExists("threads"),
                timelineItems: try db.tableExists("timeline_items"),
                outbox: try db.tableExists("outbox")
            )
        }
    }
}
