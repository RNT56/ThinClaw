import Foundation

/// One dispatched Server-Sent Event, after field accumulation.
///
/// Mirrors the WHATWG `MessageEvent` surface the gateway relies on:
/// `event` (type, defaulting to `"message"`), `data` (multi-line `data:`
/// fields joined with `\n`), and the last seen `id:` value at dispatch time.
public struct ServerSentEvent: Hashable, Sendable {
    /// Event type from the `event:` field; `"message"` when absent.
    public var event: String
    /// Payload from one or more `data:` fields, joined with `\n`.
    public var data: String
    /// The stream's "last event ID" at the moment this event dispatched
    /// (from the most recent `id:` field), or `nil` if none was ever set.
    public var lastEventID: String?

    public init(event: String = "message", data: String, lastEventID: String? = nil) {
        self.event = event
        self.data = data
        self.lastEventID = lastEventID
    }
}
