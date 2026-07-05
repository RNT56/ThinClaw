import Foundation
import ThinClawAPI
import ThinClawCore

/// Translates gateway wire DTOs (the `swift-openapi-generator` output under
/// `ThinClawAPI.Components.Schemas`) into ``ThinClawCore`` domain models.
///
/// This is the single seam where the generated, snake-cased, string-typed wire
/// shape becomes the app's typed domain. Keeping it in one place means a spec
/// regeneration only ripples through here, and the exhaustive mapping tests
/// pin every field.
///
/// Timestamps arrive as RFC 3339 / ISO 8601 strings; they are parsed with a
/// fractional-seconds-tolerant formatter and fall back to a fixed epoch only
/// when the gateway sends something unparseable (never expected, but the UI
/// must not crash on a malformed date).
public enum GatewayMapping {
    // MARK: - Threads

    /// Map a gateway ``ThreadInfo`` into a ``ChatThread``.
    ///
    /// `channel` is taken from `thread_type` (the gateway's origin marker),
    /// and there is no per-thread preview in the listing DTO so
    /// ``ChatThread/lastMessagePreview`` is left `nil`.
    public static func chatThread(from info: Components.Schemas.ThreadInfo) -> ChatThread {
        ChatThread(
            id: ThreadID(info.id),
            title: info.title ?? "",
            channel: info.threadType,
            createdAt: date(info.createdAt),
            updatedAt: date(info.updatedAt),
            lastMessagePreview: nil
        )
    }

    /// Map the `GET /api/chat/threads` response into domain threads, ordered
    /// as the gateway returned them.
    public static func chatThreads(
        from response: Components.Schemas.ThreadListResponse
    ) -> [ChatThread] {
        response.threads.map(chatThread(from:))
    }

    // MARK: - History

    /// Map the `GET /api/chat/history` response into a ``HistoryPage``.
    ///
    /// Turns are flattened into ``TimelineItem`` rows oldest-first; see
    /// ``timelineItems(from:threadID:)`` for the per-turn expansion.
    public static func historyPage(
        from response: Components.Schemas.HistoryResponse
    ) -> HistoryPage {
        let threadID = ThreadID(response.threadId)
        let items = response.turns.flatMap { timelineItems(from: $0, threadID: threadID) }
        return HistoryPage(
            threadID: threadID,
            items: items,
            hasMore: response.hasMore ?? false,
            oldestTimestamp: response.oldestTimestamp.map(date)
        )
    }

    /// Expand one ``TurnInfo`` into its ordered timeline rows:
    /// the user message (unless hidden), then each tool call, then the agent
    /// response (when the turn produced one).
    ///
    /// Ids are synthesized deterministically from the thread id, turn number,
    /// and row role/index so the same server turn maps to the same ids across
    /// refetches — which is what lets ``ReconcileResult/diff(threadID:local:server:)``
    /// match rows after a reconnect. Tool-call rows carry their lifecycle
    /// status derived from `has_error` / `has_result`.
    public static func timelineItems(
        from turn: Components.Schemas.TurnInfo,
        threadID: ThreadID
    ) -> [TimelineItem] {
        var items: [TimelineItem] = []
        let started = date(turn.startedAt)
        let completed = turn.completedAt.map(date) ?? started

        if turn.hideUserInput != true {
            items.append(
                TimelineItem(
                    id: turnItemID(threadID, turn.turnNumber, "user"),
                    threadID: threadID,
                    timestamp: started,
                    kind: .userMessage(text: turn.userInput)))
        }

        for (index, call) in turn.toolCalls.enumerated() {
            items.append(
                TimelineItem(
                    id: turnItemID(threadID, turn.turnNumber, "tool-\(index)"),
                    threadID: threadID,
                    timestamp: started,
                    kind: .toolCall(name: call.name, status: toolStatus(from: call))))
        }

        if let response = turn.response, !response.isEmpty {
            items.append(
                TimelineItem(
                    id: turnItemID(threadID, turn.turnNumber, "agent"),
                    threadID: threadID,
                    timestamp: completed,
                    kind: .agentMessage(text: response)))
        }

        return items
    }

    /// Derive a tool-call lifecycle status from the DTO's `has_error` /
    /// `has_result` flags. A completed-with-error call is `.failed`; a call
    /// with a result is `.succeeded`; anything else is still `.running`.
    public static func toolStatus(
        from call: Components.Schemas.ToolCallInfo
    ) -> TimelineItem.ToolCallStatus {
        if call.hasError { return .failed }
        if call.hasResult { return .succeeded }
        return .running
    }

    // MARK: - Approvals

    /// Map one pending-approval entry into a domain ``ApprovalRequest``.
    public static func approvalRequest(
        from entry: Components.Schemas.PendingApprovalEntry
    ) -> ApprovalRequest {
        ApprovalRequest(
            requestID: entry.requestId,
            toolName: entry.toolName,
            description: entry.description,
            parameters: entry.parameters,
            threadID: entry.threadId.map(ThreadID.init))
    }

    /// Map the `GET /api/chat/approvals` response into domain approvals,
    /// preserving the gateway's oldest-first ordering.
    public static func approvalRequests(
        from response: Components.Schemas.PendingApprovalsResponse
    ) -> [ApprovalRequest] {
        response.approvals.map(approvalRequest(from:))
    }

    /// Represent a pending approval as an inline timeline row, keyed by its
    /// request id so it reconciles against the live `approval_needed` event.
    public static func timelineItem(
        from entry: Components.Schemas.PendingApprovalEntry
    ) -> TimelineItem {
        let request = approvalRequest(from: entry)
        return TimelineItem(
            id: MessageID("approval-\(entry.requestId)"),
            threadID: request.threadID ?? ThreadID(""),
            timestamp: date(entry.createdAt),
            kind: .approval(request))
    }

    // MARK: - Send

    /// The message id echoed back by `POST /api/chat/send`.
    public static func messageID(
        from response: Components.Schemas.SendMessageResponse
    ) -> MessageID {
        MessageID(response.messageId)
    }

    // MARK: - Helpers

    /// Deterministic timeline-row id for a server turn's `role` slot.
    private static func turnItemID(_ thread: ThreadID, _ turn: Int, _ role: String) -> MessageID {
        MessageID("\(thread.rawValue)#turn\(turn)#\(role)")
    }

    /// Parse an RFC 3339 / ISO 8601 timestamp, tolerating the presence or
    /// absence of fractional seconds. Falls back to the Unix epoch if the
    /// gateway ever sends an unparseable value (defensive; not expected).
    ///
    /// Formatters are constructed per call: `ISO8601DateFormatter` is not
    /// `Sendable`, mapping is not on a hot path, and this keeps the type free
    /// of shared mutable state under Swift 6 strict concurrency.
    static func date(_ raw: String) -> Date {
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let parsed = withFraction.date(from: raw) { return parsed }

        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        if let parsed = plain.date(from: raw) { return parsed }

        return Date(timeIntervalSince1970: 0)
    }
}
