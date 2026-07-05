import Foundation

/// A conversation with the agent, as listed in the Sessions surface.
public struct ChatThread: Hashable, Sendable, Codable, Identifiable {
    public var id: ThreadID
    public var title: String
    /// Channel the thread originated on (e.g. "web", "ios"), if known.
    public var channel: String?
    public var createdAt: Date
    public var updatedAt: Date
    public var lastMessagePreview: String?

    public init(
        id: ThreadID,
        title: String,
        channel: String? = nil,
        createdAt: Date,
        updatedAt: Date,
        lastMessagePreview: String? = nil
    ) {
        self.id = id
        self.title = title
        self.channel = channel
        self.createdAt = createdAt
        self.updatedAt = updatedAt
        self.lastMessagePreview = lastMessagePreview
    }
}
