import Foundation

/// A gateway agent event, decoded from the SSE stream at `/api/chat/events`.
///
/// Variants mirror the gateway's `SseEvent` enum
/// (`crates/thinclaw-gateway/src/web/types.rs`, `#[serde(tag = "type")]`).
/// Only the event types the mobile client consumes in R0 are modeled as
/// first-class cases; every other `type` discriminator decodes to
/// ``AgentEvent/unknown(type:)`` so new gateway events can never crash or
/// stall an older client.
///
/// This type lives in ThinClawCore (the dependency-free leaf module) so that
/// ThinClawTransport, feature modules, and persistence can all share it
/// without cycles: Transport depends on Core, never the reverse.
public enum AgentEvent: Hashable, Sendable {
    /// Incremental token chunk of the in-progress assistant reply.
    /// Gateway shape: `{"type":"stream_chunk","content":"…","thread_id":"…"}`.
    case streamChunk(content: String, threadID: ThreadID?)

    /// The complete assistant reply for the turn.
    /// Gateway shape: `{"type":"response","content":"…","thread_id":"…"}`.
    case response(content: String, threadID: ThreadID?)

    /// Short human-readable progress line ("Thinking…", "Reading file…").
    /// Gateway shape: `{"type":"thinking","message":"…","thread_id":"…"}`.
    case thinking(message: String, threadID: ThreadID?)

    /// A tool invocation started.
    /// Gateway shape: `{"type":"tool_started","name":"…","thread_id":"…"}`.
    case toolStarted(name: String, threadID: ThreadID?)

    /// A tool invocation finished.
    /// Gateway shape: `{"type":"tool_completed","name":"…","success":true,…}`.
    case toolCompleted(name: String, success: Bool, threadID: ThreadID?)

    /// The agent is blocked waiting for the operator to approve a tool call.
    /// Gateway shape: `{"type":"approval_needed","request_id":"…","tool_name":"…",
    /// "description":"…","parameters":"…","thread_id":"…"}`.
    case approvalNeeded(ApprovalRequest)

    /// Token/cost accounting for the turn.
    /// Gateway shape: `{"type":"usage_update","input_tokens":…,"output_tokens":…,
    /// "cost_usd":…,"model":"…","thread_id":"…"}`.
    case usageUpdate(UsageUpdate)

    /// Keep-alive; carries no payload. Gateway shape: `{"type":"heartbeat"}`.
    /// Feed this to the reconnect watchdog, never to the UI timeline.
    case heartbeat

    /// The turn failed. Gateway shape: `{"type":"error","message":"…",…}`.
    case error(message: String, threadID: ThreadID?)

    /// Any event type this client version does not understand
    /// (e.g. `plan_update`, `subagent_spawned`, future additions).
    /// Consumers MUST tolerate and skip these.
    case unknown(type: String)
}

extension AgentEvent {
    /// The thread this event belongs to, when the gateway attached one.
    public var threadID: ThreadID? {
        switch self {
        case .streamChunk(_, let id), .response(_, let id), .thinking(_, let id),
            .toolStarted(_, let id), .toolCompleted(_, _, let id), .error(_, let id):
            return id
        case .approvalNeeded(let request):
            return request.threadID
        case .usageUpdate(let usage):
            return usage.threadID
        case .heartbeat, .unknown:
            return nil
        }
    }
}

/// Token/cost usage snapshot for one agent turn (`usage_update`).
public struct UsageUpdate: Hashable, Sendable, Codable {
    public var inputTokens: UInt32
    public var outputTokens: UInt32
    public var costUSD: Double?
    public var model: String?
    public var threadID: ThreadID?

    public init(
        inputTokens: UInt32,
        outputTokens: UInt32,
        costUSD: Double? = nil,
        model: String? = nil,
        threadID: ThreadID? = nil
    ) {
        self.inputTokens = inputTokens
        self.outputTokens = outputTokens
        self.costUSD = costUSD
        self.model = model
        self.threadID = threadID
    }
}
