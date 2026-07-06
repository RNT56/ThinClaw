import Foundation

/// One entry from a job's event log (`GET /api/jobs/{id}/events`).
///
/// The gateway endpoint is a **JSON snapshot**, not an SSE stream: it returns
/// the full stored event list (`{ job_id, events: [{ id, event_type, data,
/// created_at }] }`, see `crates/thinclaw-gateway/src/web/jobs.rs`
/// `JobEventInfo`). The "live tail" is therefore a poll: the store re-fetches
/// and folds only rows past the highest ``id`` it has already seen. Because the
/// gateway assigns a **monotonically increasing** integer `id` per stored
/// event, that id is a reliable append-only cursor.
///
/// `data` is an opaque JSON object whose shape depends on ``kind``. Rather than
/// decode every backend payload variant into a typed enum (they evolve on the
/// gateway side), the store extracts a single human-readable ``summary`` line
/// for the tail; unknown event types still render with their raw type + summary
/// so the tail never silently drops rows.
public struct JobEvent: Identifiable, Hashable, Sendable {
    /// Monotonic per-job event id; also the tail cursor.
    public let id: Int64
    /// Raw event type string from the gateway (`message`, `tool_use`,
    /// `tool_result`, `result`, …).
    public let type: String
    /// Normalized kind for iconography; unknown types map to ``JobEvent/Kind/other``.
    public let kind: Kind
    /// A short, already-extracted human-readable line describing the event
    /// (e.g. a truncated message body, `tool_name`, or a result message).
    public let summary: String
    /// Event time, parsed from the RFC3339 `created_at`. Nil if unparseable.
    public let createdAt: Date?

    public init(id: Int64, type: String, kind: Kind, summary: String, createdAt: Date?) {
        self.id = id
        self.type = type
        self.kind = kind
        self.summary = summary
        self.createdAt = createdAt
    }

    /// Normalized event kind. Mirrors the `event_type` strings the job worker
    /// logs (`src/agent/worker.rs`): `message`, `tool_use`, `tool_result`,
    /// `result`. Anything else is ``other``.
    public enum Kind: String, Hashable, Sendable {
        case message
        case toolUse
        case toolResult
        case result
        case other

        public static func from(type: String) -> Kind {
            switch type {
            case "message": return .message
            case "tool_use": return .toolUse
            case "tool_result": return .toolResult
            case "result": return .result
            default: return .other
            }
        }
    }
}

/// The raw wire row from `GET /api/jobs/{id}/events` (`JobEventInfo`).
///
/// This is a hand-rolled decode target rather than a generated type: the events
/// endpoint is intentionally **not** part of the mobile OpenAPI surface (its
/// `data` is a free-form `serde_json::Value`, and it is a polling snapshot, not
/// one of the typed REST resources). The concrete gateway adapter decodes this,
/// then maps it into ``JobEvent`` via ``JobEventProjector``.
public struct JobEventWire: Decodable, Hashable, Sendable {
    public let id: Int64
    public let eventType: String
    public let data: JSONValue
    public let createdAt: String

    enum CodingKeys: String, CodingKey {
        case id
        case eventType = "event_type"
        case data
        case createdAt = "created_at"
    }

    public init(id: Int64, eventType: String, data: JSONValue, createdAt: String) {
        self.id = id
        self.eventType = eventType
        self.data = data
        self.createdAt = createdAt
    }
}

/// The full `GET /api/jobs/{id}/events` response body.
public struct JobEventsWire: Decodable, Hashable, Sendable {
    public let jobId: String
    public let events: [JobEventWire]

    enum CodingKeys: String, CodingKey {
        case jobId = "job_id"
        case events
    }

    public init(jobId: String, events: [JobEventWire]) {
        self.jobId = jobId
        self.events = events
    }
}

/// Pure mapping from a wire event row to a display ``JobEvent``. Kept separate
/// from the store so both the summary extraction and the truncation rules are
/// unit-testable without any networking.
public enum JobEventProjector {
    /// Max characters kept from a message/output body before eliding, so a
    /// verbose tool dump does not blow up the tail row.
    public static let summaryCharLimit = 200

    public static func project(_ wire: JobEventWire) -> JobEvent {
        let kind = JobEvent.Kind.from(type: wire.eventType)
        return JobEvent(
            id: wire.id,
            type: wire.eventType,
            kind: kind,
            summary: summary(kind: kind, type: wire.eventType, data: wire.data),
            createdAt: rfc3339(wire.createdAt))
    }

    /// Extract a one-line summary from the opaque `data` object based on the
    /// known worker event shapes:
    ///   - `message`        → `content`
    ///   - `tool_use`       → `tool_name`
    ///   - `tool_result`    → `tool_name` (+ ok/fail from `success`)
    ///   - `result`         → `message`
    /// Unknown shapes fall back to the event type.
    static func summary(kind: JobEvent.Kind, type: String, data: JSONValue) -> String {
        switch kind {
        case .message:
            return truncate(data.string(for: "content") ?? type)
        case .toolUse:
            return data.string(for: "tool_name") ?? type
        case .toolResult:
            let name = data.string(for: "tool_name") ?? type
            if let success = data.bool(for: "success") {
                return success ? "\(name) — ok" : "\(name) — failed"
            }
            return name
        case .result:
            return truncate(data.string(for: "message") ?? type)
        case .other:
            // Best-effort: surface a `message`/`content` if the unknown payload
            // happens to carry one, else just the type.
            return truncate(
                data.string(for: "message") ?? data.string(for: "content") ?? type)
        }
    }

    static func truncate(_ text: String) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.count > summaryCharLimit else { return trimmed }
        return String(trimmed.prefix(summaryCharLimit)) + "…"
    }

    static func rfc3339(_ raw: String) -> Date? {
        JobDateParsing.parse(raw)
    }
}

/// Shared RFC3339 parsing for job timestamps. The gateway emits fractional and
/// non-fractional second forms (`to_rfc3339`); try both.
///
/// Formatters are constructed per call: `ISO8601DateFormatter` is not
/// `Sendable`, parsing is not on a hot path, and this keeps the type free of
/// shared mutable state under Swift 6 strict concurrency (mirrors
/// `ThinClawTransport.GatewayMapping`).
public enum JobDateParsing {
    public static func parse(_ raw: String) -> Date? {
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let parsed = withFraction.date(from: raw) { return parsed }

        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: raw)
    }
}
