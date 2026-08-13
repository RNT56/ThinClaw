import Foundation

/// Serialises foreground lifecycle changes around an asynchronous activation.
///
/// Pairing and `scenePhase` can change while the previous activation is
/// suspended in transport startup. This reconciler keeps only the newest
/// desired state, never runs two lifecycle operations concurrently, and gates
/// post-activation effects (push registration, platform relays) on the captured
/// pairing generation still being current.
@MainActor
public final class ForegroundLifecycleReconciler {
    public enum Target: Sendable, Equatable {
        case inactive
        case active(pairingGeneration: UInt64)
    }

    public struct Actions: Sendable {
        /// Start the authenticated session and complete its initial foreground
        /// reconciliation (outbox + snapshots).
        public var activate: @MainActor @Sendable () async -> Void
        /// Effects that are safe only after activation is still current.
        public var didActivate: @MainActor @Sendable () -> Void
        /// Stop the authenticated foreground session.
        public var deactivate: @MainActor @Sendable () async -> Void

        public init(
            activate: @escaping @MainActor @Sendable () async -> Void,
            didActivate: @escaping @MainActor @Sendable () -> Void,
            deactivate: @escaping @MainActor @Sendable () async -> Void
        ) {
            self.activate = activate
            self.didActivate = didActivate
            self.deactivate = deactivate
        }
    }

    private let actions: Actions
    private var desired: Target = .inactive
    private var applied: Target = .inactive
    private var revision: UInt64 = 0
    private var reconciliationTask: Task<Void, Never>?

    public init(actions: Actions) {
        self.actions = actions
    }

    /// Reconcile to the newest desired state. Repeated identical transitions
    /// are coalesced, including while an earlier activation is suspended.
    public func reconcile(to target: Target) {
        guard target != desired else { return }
        desired = target
        revision &+= 1
        guard reconciliationTask == nil else { return }
        reconciliationTask = Task { @MainActor [weak self] in
            await self?.runLoop()
        }
    }

    /// Test/support hook that waits until all currently requested work settles.
    public func waitUntilSettled() async {
        await reconciliationTask?.value
    }

    private func runLoop() async {
        while desired != applied {
            let target = desired
            let requestedRevision = revision

            switch target {
            case .inactive:
                await actions.deactivate()
                // Deactivation took effect even if the desired state changed
                // while it was suspended. Recording that fact makes a rapid
                // background/foreground transition restart correctly.
                applied = .inactive

            case .active:
                await actions.activate()
                // Likewise, transport startup took effect. A stale activation
                // is immediately followed by reconciliation to the new target,
                // but it must not trigger push/platform effects.
                applied = target
                guard requestedRevision == revision, desired == target else {
                    continue
                }
                actions.didActivate()
            }
        }

        reconciliationTask = nil
    }
}
