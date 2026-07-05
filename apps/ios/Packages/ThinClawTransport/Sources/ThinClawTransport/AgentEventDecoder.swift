import Foundation
import ThinClawCore

/// Failure decoding an SSE `data` payload into an ``AgentEvent``.
public enum AgentEventDecodingError: Error, Hashable, Sendable {
    /// The `data` payload was not a JSON object.
    case malformedJSON
    /// The JSON object had no string `type` discriminator.
    case missingTypeDiscriminator
    /// The `type` was recognized but its payload did not match the contract.
    case invalidPayload(type: String)
}

/// Decodes gateway SSE `data` JSON into ``ThinClawCore/AgentEvent`` by the
/// `type` discriminator (the gateway serializes its `SseEvent` enum with
/// `#[serde(tag = "type")]`).
///
/// Forward-compatibility contract: an unrecognized `type` decodes to
/// `.unknown(type:)` — never an error — so new gateway event kinds degrade
/// gracefully on old clients. Only structurally broken payloads throw.
public struct AgentEventDecoder: Sendable {
    public init() {}

    /// Decode a parsed SSE event's `data` payload.
    public func decode(_ event: ServerSentEvent) throws -> AgentEvent {
        try decode(json: Data(event.data.utf8))
    }

    /// Decode a raw JSON payload.
    public func decode(json data: Data) throws -> AgentEvent {
        // JSONDecoder is not Sendable; a per-call instance keeps this type
        // trivially Sendable and is cheap relative to the JSON parse itself.
        let decoder = JSONDecoder()

        let envelope: TypeEnvelope
        do {
            envelope = try decoder.decode(TypeEnvelope.self, from: data)
        } catch let error as DecodingError {
            if case .keyNotFound = error { throw AgentEventDecodingError.missingTypeDiscriminator }
            if case .typeMismatch = error { throw AgentEventDecodingError.missingTypeDiscriminator }
            throw AgentEventDecodingError.malformedJSON
        } catch {
            throw AgentEventDecodingError.malformedJSON
        }

        func payload<P: Decodable>(_ type: P.Type) throws -> P {
            do {
                return try decoder.decode(P.self, from: data)
            } catch {
                throw AgentEventDecodingError.invalidPayload(type: envelope.type)
            }
        }

        switch envelope.type {
        case "stream_chunk":
            let p = try payload(StreamChunkPayload.self)
            return .streamChunk(content: p.content, threadID: p.threadId.map(ThreadID.init))
        case "response":
            let p = try payload(ResponsePayload.self)
            return .response(content: p.content, threadID: p.threadId.map(ThreadID.init))
        case "thinking":
            let p = try payload(ThinkingPayload.self)
            return .thinking(message: p.message, threadID: p.threadId.map(ThreadID.init))
        case "tool_started":
            let p = try payload(ToolStartedPayload.self)
            return .toolStarted(name: p.name, threadID: p.threadId.map(ThreadID.init))
        case "tool_completed":
            let p = try payload(ToolCompletedPayload.self)
            return .toolCompleted(
                name: p.name, success: p.success, threadID: p.threadId.map(ThreadID.init))
        case "approval_needed":
            let p = try payload(ApprovalNeededPayload.self)
            return .approvalNeeded(
                ApprovalRequest(
                    requestID: p.requestId,
                    toolName: p.toolName,
                    description: p.description,
                    parameters: p.parameters,
                    threadID: p.threadId.map(ThreadID.init)))
        case "usage_update":
            let p = try payload(UsageUpdatePayload.self)
            return .usageUpdate(
                UsageUpdate(
                    inputTokens: p.inputTokens,
                    outputTokens: p.outputTokens,
                    costUSD: p.costUsd,
                    model: p.model,
                    threadID: p.threadId.map(ThreadID.init)))
        case "heartbeat":
            return .heartbeat
        case "error":
            let p = try payload(ErrorPayload.self)
            return .error(message: p.message, threadID: p.threadId.map(ThreadID.init))
        default:
            return .unknown(type: envelope.type)
        }
    }
}

// MARK: - Wire payloads (snake_case, matching the gateway's serde output)

private struct TypeEnvelope: Decodable {
    let type: String
}

private struct StreamChunkPayload: Decodable {
    let content: String
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case content
        case threadId = "thread_id"
    }
}

private struct ResponsePayload: Decodable {
    let content: String
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case content
        case threadId = "thread_id"
    }
}

private struct ThinkingPayload: Decodable {
    let message: String
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case message
        case threadId = "thread_id"
    }
}

private struct ToolStartedPayload: Decodable {
    let name: String
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case name
        case threadId = "thread_id"
    }
}

private struct ToolCompletedPayload: Decodable {
    let name: String
    let success: Bool
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case name
        case success
        case threadId = "thread_id"
    }
}

private struct ApprovalNeededPayload: Decodable {
    let requestId: String
    let toolName: String
    let description: String
    let parameters: String
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case toolName = "tool_name"
        case description
        case parameters
        case threadId = "thread_id"
    }
}

private struct UsageUpdatePayload: Decodable {
    let inputTokens: UInt32
    let outputTokens: UInt32
    let costUsd: Double?
    let model: String?
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case inputTokens = "input_tokens"
        case outputTokens = "output_tokens"
        case costUsd = "cost_usd"
        case model
        case threadId = "thread_id"
    }
}

private struct ErrorPayload: Decodable {
    let message: String
    let threadId: String?

    enum CodingKeys: String, CodingKey {
        case message
        case threadId = "thread_id"
    }
}
