import Foundation

/// Privacy-preserving aggregate of one actor's short-lived sessions.
/// Individual session and principal identifiers are intentionally absent.
public struct PresenceAggregate: Hashable, Sendable {
    public var actorID: String
    public var scope: PresenceScope
    public var state: PresenceState
    public var surfaces: [PresenceSurface]
    public var sessionCount: Int
    public var updatedAt: String
    public var expiresAt: String

    public init(
        actorID: String,
        scope: PresenceScope,
        state: PresenceState,
        surfaces: [PresenceSurface],
        sessionCount: Int,
        updatedAt: String,
        expiresAt: String
    ) {
        self.actorID = actorID
        self.scope = scope
        self.state = state
        self.surfaces = surfaces
        self.sessionCount = sessionCount
        self.updatedAt = updatedAt
        self.expiresAt = expiresAt
    }
}

public enum PresenceState: String, Hashable, Sendable, Codable {
    case online
    case away
    case busy
    case typing
}

public enum PresenceSurface: String, Hashable, Sendable, Codable {
    case desktop
    case web
    case ios
    case watchos
    case cli
    case channel
}

public enum PresenceScope: Hashable, Sendable {
    case principal
    case thread(ThreadID)

    public var threadID: ThreadID? {
        guard case .thread(let id) = self else { return nil }
        return id
    }
}

public enum PresenceEventKind: String, Hashable, Sendable, Codable {
    case joined
    case updated
    case expired
}

public enum PresenceEventCause: String, Hashable, Sendable, Codable {
    case publish
    case ttl
    case clear
    case disconnect
    case scopeChanged = "scope_changed"
}

public struct PresenceEvent: Hashable, Sendable {
    public var event: PresenceEventKind
    public var cause: PresenceEventCause
    public var presence: PresenceAggregate

    public init(
        event: PresenceEventKind,
        cause: PresenceEventCause,
        presence: PresenceAggregate
    ) {
        self.event = event
        self.cause = cause
        self.presence = presence
    }
}

/// Authoritative replacement for one principal/thread scope. Clients receive
/// this after initial connect and every reconnect because SSE has no replay.
public struct PresenceSnapshot: Hashable, Sendable {
    public var scope: PresenceScope
    public var presences: [PresenceAggregate]
    public var serverTime: String

    public init(scope: PresenceScope, presences: [PresenceAggregate], serverTime: String) {
        self.scope = scope
        self.presences = presences
        self.serverTime = serverTime
    }
}

/// A presence consumer first replaces a scope from `snapshot`, then applies
/// ordered incremental events. This makes reconnect repair explicit instead
/// of pretending the non-replayed SSE stream is an authoritative snapshot.
public enum PresenceUpdate: Hashable, Sendable {
    case snapshot(PresenceSnapshot)
    case event(PresenceEvent)
}
