import Foundation
import ThinClawAPI
import ThinClawCore

/// Production ``LiveActivityRegistrar``: registers Live Activity push tokens
/// with the gateway over the generated, pinned client. All calls are
/// best-effort — a transient gateway outage must not crash the app or the
/// activity; the token is re-sent on the next `pushTokenUpdates` emission.
///
/// Agent-run activities always register with `kind: .agentRun` and the owning
/// `thread_id` so the gateway's first-party notifier can route run-progress
/// events to this activity's per-activity update token (docs/MOBILE_SECURITY.md
/// D-N2). Job activities are not driven by this manager.
public struct GatewayLiveActivityRegistrar: LiveActivityRegistrar {
    private let client: any APIProtocol

    public init(client: any APIProtocol) {
        self.client = client
    }

    public func registerActivityToken(
        activityID: String,
        pushToken: String,
        threadID: ThreadID
    ) async {
        _ = try? await client.devicesMeLiveActivityRegisterHandler(
            path: .init(activityId: activityID),
            body: .json(
                .init(
                    jobId: nil,
                    kind: .agentRun,
                    pushToken: pushToken,
                    threadId: threadID.rawValue)))
    }

    public func removeActivityToken(activityID: String) async {
        _ = try? await client.devicesMeLiveActivityRemoveHandler(
            path: .init(activityId: activityID))
    }

    public func registerStartToken(pushToken: String) async {
        _ = try? await client.devicesMeLiveActivityStartTokenRegisterHandler(
            body: .json(.init(pushToken: pushToken)))
    }
}
