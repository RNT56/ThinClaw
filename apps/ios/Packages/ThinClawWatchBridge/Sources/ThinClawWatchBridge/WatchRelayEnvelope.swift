import Foundation

/// RPCs the watch sends through the phone (or directly, when reachable).
///
/// Payloads are small `Codable` envelopes. The **watch's own reduced-scope
/// token rides inside** a relayed request (``WatchRelayEnvelope/watchToken``)
/// so the phone can forward it opaquely to the gateway without ever
/// substituting its own credential — the gateway then attributes and revokes
/// the watch independently (docs/MOBILE_SECURITY.md, D-K4).
public enum WatchRelayRequest: Codable, Sendable, Equatable {
    /// Approve or deny a pending tool call. `action` is the raw gateway verb
    /// (`"approve"` / `"deny"`; `"always"` is never sent from the watch).
    /// High-risk approvals are refused before they reach here (D-K3/D-K4) —
    /// the watch offers the approve action for low-risk entries only.
    case approve(requestID: String, threadID: String?, action: String)
    /// A dictated quick prompt (`POST /api/chat/send`).
    case quickAsk(prompt: String, threadID: String?)
    /// Ask for the freshest agent-status + pending-approvals snapshot.
    case snapshotRefresh
}

/// The phone's reply to a relayed ``WatchRelayRequest``.
///
/// A relayed request carries the watch token; the phone forwards it opaquely
/// and reports the gateway's outcome back verbatim (never its own decision).
public enum WatchRelayResponse: Codable, Sendable, Equatable {
    /// The gateway accepted the request. For a ``WatchRelayRequest/quickAsk``
    /// the gateway `message_id` is echoed back for the receipt UI.
    case accepted(messageID: String?)
    /// The request was rejected/failed. `reason` is a short, non-secret label
    /// (e.g. a status code family) safe to render on the wrist.
    case failed(reason: String)
    /// The watch's companion credential is missing or was rejected by the
    /// gateway (e.g. revoked). The watch surfaces "re-provisioning" and the
    /// phone re-mints on next reachability (D-K4 re-provision path).
    case reprovisionRequired

    public static let accepted = WatchRelayResponse.accepted(messageID: nil)

    // MARK: - Wire coding

    /// The reply is JSON under a single key, symmetric with
    /// ``WatchRelayEnvelope/messagePayload()`` on the request path, so the
    /// `WCSession` reply channel stays schema-agnostic and a concrete relay
    /// transport can decode the outcome the phone forwarded back.
    public static let messageKey = "response"

    /// Encode to a `[String: Any]` reply for `WCSession`'s `replyHandler`.
    public func messagePayload() throws -> [String: Any] {
        [Self.messageKey: try JSONEncoder().encode(self)]
    }

    /// Decode a reply produced by ``messagePayload()``.
    public static func fromMessage(_ message: [String: Any]) throws -> WatchRelayResponse {
        guard let data = message[messageKey] as? Data else {
            throw WatchRelayError.malformedMessage
        }
        return try JSONDecoder().decode(WatchRelayResponse.self, from: data)
    }
}

/// The wire envelope carried over `WCSession.sendMessage` (relay) or the direct
/// URLSession body when the watch talks to the gateway itself.
///
/// The token is **only** present on the relay path: the phone reads it out and
/// forwards it as the `Authorization: Bearer` header, never using its own. On
/// the direct path the watch signs the request itself, so the envelope omits
/// the token (it never needs to leave the watch keychain over the wire).
public struct WatchRelayEnvelope: Codable, Sendable, Equatable {
    /// Envelope schema version; unknown future versions are rejected by the
    /// host rather than silently mis-decoded.
    public var version: Int
    /// The watch's reduced-scope companion token (`tcd_…`). Present on relayed
    /// requests so the phone forwards it opaquely; `nil` on the direct path.
    public var watchToken: String?
    public var request: WatchRelayRequest

    public static let currentVersion = 1

    public init(
        version: Int = WatchRelayEnvelope.currentVersion,
        watchToken: String?,
        request: WatchRelayRequest
    ) {
        self.version = version
        self.watchToken = watchToken
        self.request = request
    }

    // MARK: - Wire coding

    /// Encode to `Data` for `WCSession` transfer or a direct body.
    public func encoded() throws -> Data {
        try JSONEncoder().encode(self)
    }

    /// Decode from wire `Data`, rejecting an unknown envelope version so a
    /// forward-incompatible peer fails closed rather than acting on a
    /// half-understood request.
    public static func decode(_ data: Data) throws -> WatchRelayEnvelope {
        let envelope = try JSONDecoder().decode(WatchRelayEnvelope.self, from: data)
        guard envelope.version == currentVersion else {
            throw WatchRelayError.unsupportedVersion(envelope.version)
        }
        return envelope
    }

    /// A `[String: Any]` form for `WCSession.sendMessage` interop, which only
    /// carries property-list values. The whole envelope is JSON under one key
    /// so the transport stays schema-agnostic.
    public static let messageKey = "envelope"

    public func messagePayload() throws -> [String: Any] {
        [Self.messageKey: try encoded()]
    }

    public static func fromMessage(_ message: [String: Any]) throws -> WatchRelayEnvelope {
        guard let data = message[messageKey] as? Data else {
            throw WatchRelayError.malformedMessage
        }
        return try decode(data)
    }
}

/// Errors from envelope/provisioning coding and relay routing.
public enum WatchRelayError: Error, Equatable, Sendable {
    /// The envelope declared a version this build does not understand.
    case unsupportedVersion(Int)
    /// A `WCSession` message did not carry a decodable envelope.
    case malformedMessage
    /// No route could carry the request (relay down, no direct reachability).
    case noRouteAvailable
    /// The relay/direct call did not complete within the deadline.
    case timedOut
}
