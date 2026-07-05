import Foundation

/// A pending tool-call approval surfaced by the gateway (`approval_needed`).
///
/// `parameters` is a JSON-encoded string exactly as the gateway sends it
/// (the gateway serializes tool parameters to a string before embedding them
/// in the event); the UI pretty-prints it but the client never needs to
/// interpret it structurally.
public struct ApprovalRequest: Hashable, Sendable, Codable, Identifiable {
    /// Gateway-issued request id; echo it back on `/api/chat/approval`.
    public var requestID: String
    public var toolName: String
    public var description: String
    /// JSON-encoded tool parameters, verbatim from the gateway.
    public var parameters: String
    public var threadID: ThreadID?

    public var id: String { requestID }

    public init(
        requestID: String,
        toolName: String,
        description: String,
        parameters: String,
        threadID: ThreadID? = nil
    ) {
        self.requestID = requestID
        self.toolName = toolName
        self.description = description
        self.parameters = parameters
        self.threadID = threadID
    }
}
