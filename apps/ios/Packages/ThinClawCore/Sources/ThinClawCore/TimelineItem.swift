import Foundation

/// One renderable row in a chat transcript.
///
/// The chat UI is a flat list of timeline items; `kind` carries the
/// per-row payload. Codable so ThinClawPersistence can store transcripts
/// without a parallel record type (GRDB rows will wrap this at M1).
public struct TimelineItem: Hashable, Sendable, Codable, Identifiable {
    public enum DeliveryState: String, Hashable, Sendable, Codable {
        case sending
        case queued
        case failed
    }

    public enum Kind: Hashable, Sendable, Codable {
        /// A message typed by the operator on this device.
        case userMessage(text: String)
        /// A completed assistant reply.
        case agentMessage(text: String)
        /// An assistant reply still streaming in; `text` is the partial body.
        case streamingAgentMessage(text: String)
        /// Progress/status line (from `thinking` events).
        case statusNote(text: String)
        /// A tool invocation and its lifecycle.
        case toolCall(name: String, status: ToolCallStatus)
        /// An inline approval prompt.
        case approval(ApprovalRequest)
        /// An extension-authorization (OAuth) prompt (`auth_required`).
        case authPrompt(AuthPrompt)
        /// A credential request the phone cannot answer (`credential_prompt`);
        /// rendered as a "handle on desktop" card (D-T4).
        case credentialPrompt(CredentialPrompt)
        /// A turn-level failure.
        case failure(message: String)
    }

    public enum ToolCallStatus: Hashable, Sendable, Codable {
        case running
        case succeeded
        case failed
    }

    public var id: MessageID
    public var threadID: ThreadID
    public var timestamp: Date
    public var kind: Kind
    /// Local delivery state for an operator-authored row. Nil once the gateway
    /// has accepted the message or for rows originating from gateway history.
    public var deliveryState: DeliveryState?

    public init(
        id: MessageID = MessageID(),
        threadID: ThreadID,
        timestamp: Date,
        kind: Kind,
        deliveryState: DeliveryState? = nil
    ) {
        self.id = id
        self.threadID = threadID
        self.timestamp = timestamp
        self.kind = kind
        self.deliveryState = deliveryState
    }
}
