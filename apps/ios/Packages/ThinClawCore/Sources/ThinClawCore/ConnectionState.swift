import Foundation

/// Client-side view of the SSE event-stream connection to the gateway.
public enum ConnectionState: Hashable, Sendable {
    /// No connection requested (e.g. not yet paired).
    case idle
    /// First connection attempt in flight.
    case connecting
    /// Stream established and heartbeats arriving.
    case connected
    /// Stream dropped; a retry is scheduled. `attempt` counts consecutive
    /// failures since the last successful connection (0-based).
    case reconnecting(attempt: Int)
    /// Gave up or hit a non-retryable error (e.g. auth revoked).
    case failed(message: String)
}

extension ConnectionState {
    /// Whether the UI should render live-updating affordances.
    public var isLive: Bool {
        if case .connected = self { return true }
        return false
    }
}
