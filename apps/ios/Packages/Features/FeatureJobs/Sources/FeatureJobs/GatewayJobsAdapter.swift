import Foundation
import ThinClawAPI
import ThinClawCore

/// Production ``JobsGateway`` for the read-only Jobs glance.
///
/// Two networking seams, both over the **same pinned session policy** the rest
/// of the app uses (D-X2, `docs/MOBILE_SECURITY.md`):
///
///   - list / summary / detail go through the generated REST ``Client``
///     (`GET /api/jobs`, `/api/jobs/summary`, `/api/jobs/{id}`) — typed,
///     contract-checked surface;
///   - the event tail hits `GET /api/jobs/{id}/events`, which is a **JSON
///     snapshot** endpoint (not SSE) and is deliberately **not** part of the
///     mobile OpenAPI surface (its `data` is a free-form value; there is no
///     job event *stream* on the gateway). It is fetched with a hand-rolled
///     pinned `URLSession` GET and decoded into ``JobEventsWire`` — the same
///     "hand-roll the non-generated path" split ThinClawTransport uses for SSE.
///
/// Every method is read-only: the phone token holds `jobs:read` only, so this
/// adapter exposes no mutation. The events endpoint is reachable by that scope
/// (any `GET /api/jobs/…` maps to `jobs:read`, see the gateway's `scopes.rs`).
public struct GatewayJobsAdapter: JobsGateway {
    private let client: any APIProtocol
    private let baseURL: URL
    private let token: @Sendable () -> String?
    private let session: URLSession

    /// - Parameters:
    ///   - client: The generated gateway REST client (over the pinned session).
    ///   - baseURL: The gateway base URL the pinned client targets — the same
    ///     URL is used to build the events-tail request so both share the pin.
    ///   - token: Supplies the current device bearer token.
    ///   - session: The **pinned** `URLSession` used for the raw events GET.
    public init(
        client: any APIProtocol,
        baseURL: URL,
        token: @escaping @Sendable () -> String?,
        session: URLSession
    ) {
        self.client = client
        self.baseURL = baseURL
        self.token = token
        self.session = session
    }

    public func listJobs() async throws -> [Job] {
        let response = try await client.jobsListHandler(.init()).ok.body.json
        return response.jobs.map(Self.job(from:))
    }

    public func jobsSummary() async throws -> JobsSummary {
        let response = try await client.jobsSummaryHandler(.init()).ok.body.json
        return JobsSummary(
            total: response.total,
            pending: response.pending,
            inProgress: response.inProgress,
            completed: response.completed,
            failed: response.failed,
            cancelled: response.cancelled,
            interrupted: response.interrupted,
            stuck: response.stuck)
    }

    public func jobDetail(id: String) async throws -> JobDetail {
        let response = try await client.jobsDetailHandler(.init(path: .init(id: id)))
            .ok.body.json
        return JobDetail(
            id: response.id,
            title: response.title,
            description: response.description,
            state: response.state,
            phase: JobPhase.from(state: response.state),
            createdAt: JobDateParsing.parse(response.createdAt),
            startedAt: response.startedAt.flatMap(JobDateParsing.parse),
            completedAt: response.completedAt.flatMap(JobDateParsing.parse),
            elapsedSeconds: response.elapsedSecs.map(Int.init),
            transitions: response.transitions.map(Self.transition(from:)))
    }

    public func jobEvents(id: String) async throws -> [JobEvent] {
        // `/api/jobs/{id}/events` is not in the generated surface — build the
        // request by hand over the pinned session with the device bearer token.
        var url = baseURL
        url.append(path: "api/jobs/\(id)/events")
        var request = URLRequest(url: url)
        request.httpMethod = "GET"
        if let token = token() {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw JobsAdapterError.badResponse
        }
        guard (200..<300).contains(http.statusCode) else {
            throw JobsAdapterError.status(http.statusCode)
        }

        let wire = try JSONDecoder().decode(JobEventsWire.self, from: data)
        return wire.events.map(JobEventProjector.project)
    }

    // MARK: - Mapping

    private static func job(from info: Components.Schemas.JobInfo) -> Job {
        Job(
            id: info.id,
            title: info.title,
            state: info.state,
            phase: JobPhase.from(state: info.state),
            createdAt: JobDateParsing.parse(info.createdAt),
            startedAt: info.startedAt.flatMap(JobDateParsing.parse))
    }

    private static func transition(from info: Components.Schemas.TransitionInfo) -> JobTransition {
        JobTransition(
            from: info.from,
            to: info.to,
            reason: info.reason,
            timestamp: JobDateParsing.parse(info.timestamp))
    }
}

/// Errors from the hand-rolled events fetch.
public enum JobsAdapterError: LocalizedError {
    case badResponse
    case status(Int)

    public var errorDescription: String? {
        switch self {
        case .badResponse:
            return "The gateway returned an unexpected response."
        case .status(let code):
            return "The gateway returned HTTP \(code)."
        }
    }
}
