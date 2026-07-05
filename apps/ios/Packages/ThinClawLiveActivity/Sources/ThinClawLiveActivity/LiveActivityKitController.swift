#if os(iOS) && canImport(ActivityKit)
    import ActivityKit
    import Foundation
    import ThinClawCore
    import ThinClawSnapshotKit

    /// Production ``ActivityController``: wraps `ActivityKit.Activity` for
    /// ``AgentRunAttributes``. Owns the live `Activity` handles keyed by thread,
    /// observes each activity's `pushTokenUpdates`, and forwards new tokens to the
    /// ``LiveActivityManager`` so it can register them with the gateway.
    ///
    /// iOS-only (behind `canImport(ActivityKit)`); the pure reducer +
    /// ``LiveActivityManager`` are exercised on macOS with a fake controller.
    @MainActor
    public final class LiveActivityKitController: ActivityController {
        /// Set once, right after construction, so the controller can hand new
        /// push tokens back for gateway registration. Weak to avoid a retain
        /// cycle with the manager (which owns the controller).
        public weak var manager: LiveActivityManager?

        private var activities: [ThreadID: Activity<AgentRunAttributes>] = [:]
        private var tokenTasks: [ThreadID: Task<Void, Never>] = [:]

        public init() {}

        public var areActivitiesEnabled: Bool {
            ActivityAuthorizationInfo().areActivitiesEnabled
        }

        public func requestActivity(
            thread: ThreadID,
            title: String,
            state: RunState
        ) async -> String? {
            // Never run two activities for one thread.
            if activities[thread] != nil { return activities[thread]?.id }

            let attributes = AgentRunAttributes(threadID: thread.rawValue, threadTitle: title)
            let content = ActivityContent(
                state: Self.contentState(from: state), staleDate: nil)

            do {
                let activity = try Activity.request(
                    attributes: attributes,
                    content: content,
                    pushType: .token)
                activities[thread] = activity
                observePushTokens(for: activity, thread: thread)
                return activity.id
            } catch {
                // Request can fail if the user disabled activities between the
                // guard and here, or the per-app budget is exceeded. Swallow —
                // the manager treats a nil id as "no activity".
                return nil
            }
        }

        public func updateActivity(thread: ThreadID, state: RunState) async {
            guard let activity = activities[thread] else { return }
            // `Activity` is a non-Sendable class whose `update`/`end` are
            // `nonisolated async`; passing this main-actor-held handle into them
            // trips Swift 6 region isolation. ActivityKit documents these as safe
            // to call from any context and we do not mutate the handle across the
            // boundary, so bridge it out of the actor region explicitly.
            nonisolated(unsafe) let handle = activity
            await handle.update(
                ActivityContent(state: Self.contentState(from: state), staleDate: nil))
        }

        public func endActivity(thread: ThreadID, state: RunState) async {
            tokenTasks.removeValue(forKey: thread)?.cancel()
            guard let activity = activities.removeValue(forKey: thread) else { return }
            // Dismiss shortly after the run ends so the final state is briefly
            // visible on the lock screen, then clears itself. See `updateActivity`
            // for why the handle is bridged out of the actor region.
            nonisolated(unsafe) let handle = activity
            await handle.end(
                ActivityContent(state: Self.contentState(from: state), staleDate: nil),
                dismissalPolicy: .after(.now + 5))
        }

        public func endAll() async {
            for task in tokenTasks.values { task.cancel() }
            tokenTasks.removeAll()
            let live = activities
            activities.removeAll()
            for activity in live.values {
                // See `updateActivity` for why the handle is bridged out of the
                // actor region before calling the `nonisolated` `end`.
                nonisolated(unsafe) let handle = activity
                await handle.end(nil, dismissalPolicy: .immediate)
            }
        }

        // MARK: - Push-to-start

        /// The device's Live Activity push-to-start token updates as hex strings,
        /// bridged from `Activity<AgentRunAttributes>.pushToStartTokenUpdates`.
        /// The manager forwards these to
        /// `PUT /api/devices/me/live-activity-start-token` so the gateway can
        /// spawn the activity on a killed app.
        public nonisolated func pushToStartTokenUpdates() -> AsyncStream<String> {
            AsyncStream { continuation in
                let task = Task {
                    for await tokenData in Activity<AgentRunAttributes>.pushToStartTokenUpdates {
                        let hex = tokenData.map { String(format: "%02x", $0) }.joined()
                        continuation.yield(hex)
                    }
                    continuation.finish()
                }
                continuation.onTermination = { _ in task.cancel() }
            }
        }

        // MARK: - Push tokens

        /// Observe an activity's `pushTokenUpdates` and forward each hex token to
        /// the manager for gateway registration.
        private func observePushTokens(
            for activity: Activity<AgentRunAttributes>,
            thread: ThreadID
        ) {
            let activityID = activity.id
            tokenTasks[thread]?.cancel()
            tokenTasks[thread] = Task { [weak self] in
                for await tokenData in activity.pushTokenUpdates {
                    if Task.isCancelled { break }
                    let hex = tokenData.map { String(format: "%02x", $0) }.joined()
                    await self?.manager?.registerActivityPushToken(
                        activityID: activityID, pushToken: hex, thread: thread)
                }
            }
        }

        // MARK: - Mapping

        /// Map the content-free ``RunState`` to the shared attributes'
        /// `ContentState`. The raw phase values match, so this is total.
        private static func contentState(
            from state: RunState
        ) -> AgentRunAttributes.ContentState {
            AgentRunAttributes.ContentState(
                phase: AgentRunAttributes.ContentState.RunPhase(rawValue: state.phase.rawValue)
                    ?? .thinking,
                toolName: state.toolName,
                progress: state.progress,
                pendingApprovalID: state.pendingApprovalID,
                revision: state.revision)
        }
    }
#endif
