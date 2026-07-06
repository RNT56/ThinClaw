import Foundation
import ThinClawCore
import ThinClawTransport

/// The event feed the ``LiveActivityManager`` observes for one thread. A thin
/// seam over ``GatewaySession/events(in:)`` so the manager can be driven by a
/// scripted stream under `swift test` on a Mac host, without constructing a
/// live session.
public protocol RunEventSource: Sendable {
    /// Live agent events for `thread`, matching ``GatewaySession/events(in:)``.
    func events(in thread: ThreadID) async -> AsyncStream<AgentEvent>
}

/// Production ``RunEventSource``: forwards to a ``GatewaySession``.
public struct GatewaySessionEventSource: RunEventSource {
    private let session: GatewaySession

    public init(session: GatewaySession) {
        self.session = session
    }

    public func events(in thread: ThreadID) async -> AsyncStream<AgentEvent> {
        await session.events(in: thread)
    }
}
