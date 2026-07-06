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

/// The gateway's thread listing: the regular conversation threads plus the
/// pinned assistant thread, which is distinct from `threads` on the wire.
///
/// The assistant thread is the always-present, pinned home for the agent's
/// default conversation. It is surfaced separately so callers can prefer it as
/// the default landing thread without depending on it also appearing in
/// `threads`.
public struct ThreadListing: Hashable, Sendable, Codable {
    /// Regular conversation threads, ordered as the gateway returned them.
    public var threads: [ChatThread]
    /// The pinned assistant thread, when present.
    public var assistantThread: ChatThread?

    public init(threads: [ChatThread], assistantThread: ChatThread? = nil) {
        self.threads = threads
        self.assistantThread = assistantThread
    }
}
