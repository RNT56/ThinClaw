import Foundation

/// The native client's deliberate event-transport contract.
///
/// ThinClaw iOS standardizes on server-sent events. It does not negotiate or
/// silently fall back to WebSocket: both transports carry the same gateway
/// schema, while a single supervised SSE path keeps TLS pinning, cancellation,
/// watchdog, and foreground reconciliation behavior identical on every device.
public enum GatewayEventTransportPolicy: String, CaseIterable, Sendable {
    /// `GET /api/chat/events` with `Accept: text/event-stream`.
    case serverSentEvents = "sse"

    /// The only supported native-client policy.
    public static let mobileDefault = GatewayEventTransportPolicy.serverSentEvents

    /// Gateway route used for a fresh event connection.
    public var endpointPath: String {
        switch self {
        case .serverSentEvents: "api/chat/events"
        }
    }

    /// Media type required by the selected route.
    public var acceptHeader: String {
        switch self {
        case .serverSentEvents: "text/event-stream"
        }
    }

    /// The gateway event stream is live-only. After reconnect, the session
    /// reconciles authoritative REST snapshots instead of pretending an SSE id
    /// or a second transport can replay missed events.
    public var missedEventRecovery: MissedEventRecovery {
        .reconcileRESTSnapshots
    }

    /// Stream failures reconnect through the same supervised SSE state machine;
    /// there is no hidden transport downgrade with different security behavior.
    public var failurePolicy: EventTransportFailurePolicy {
        .reconnectSameTransport
    }

    func request(baseURL: URL, token: String) -> URLRequest {
        var request = URLRequest(url: baseURL.appending(path: endpointPath))
        request.httpMethod = "GET"
        request.setValue(acceptHeader, forHTTPHeaderField: "Accept")
        // Header-only auth (MOBILE_SECURITY D-T4/T14) — never `?token=`.
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        return request
    }
}

public enum MissedEventRecovery: String, Sendable {
    case reconcileRESTSnapshots = "reconcile_rest_snapshots"
}

public enum EventTransportFailurePolicy: String, Sendable {
    case reconnectSameTransport = "reconnect_same_transport"
}
