import Testing

@testable import ThinClawCore

@MainActor
@Suite("Foreground lifecycle reconciliation")
struct ForegroundLifecycleReconcilerTests {
    @Test("Pairing in an already-active scene starts every effect immediately once")
    func alreadyActivePairStartsImmediately() async {
        let recorder = Recorder()
        let reconciler = recorder.makeReconciler()

        reconciler.reconcile(to: .active(pairingGeneration: 1))
        await reconciler.waitUntilSettled()

        #expect(recorder.activations == 1)
        #expect(recorder.postActivations == 1)
        #expect(recorder.deactivations == 0)
    }

    @Test("Pairing while inactive defers all activation until the scene becomes active")
    func inactivePairDefers() async {
        let recorder = Recorder()
        let reconciler = recorder.makeReconciler()

        reconciler.reconcile(to: .inactive)
        await reconciler.waitUntilSettled()
        #expect(recorder.activations == 0)

        reconciler.reconcile(to: .active(pairingGeneration: 1))
        await reconciler.waitUntilSettled()
        #expect(recorder.activations == 1)
        #expect(recorder.postActivations == 1)
    }

    @Test("Duplicate lifecycle transitions are coalesced")
    func duplicateTransitionsAreIdempotent() async {
        let recorder = Recorder()
        let reconciler = recorder.makeReconciler()

        reconciler.reconcile(to: .active(pairingGeneration: 7))
        reconciler.reconcile(to: .active(pairingGeneration: 7))
        reconciler.reconcile(to: .active(pairingGeneration: 7))
        await reconciler.waitUntilSettled()

        #expect(recorder.activations == 1)
        #expect(recorder.postActivations == 1)
    }

    @Test("A stale suspended activation is torn down without post-activation effects")
    func staleActivationIsGenerationChecked() async {
        let gate = SuspensionGate()
        let recorder = Recorder(activationGate: gate)
        let reconciler = recorder.makeReconciler()

        reconciler.reconcile(to: .active(pairingGeneration: 1))
        await gate.waitUntilEntered()
        reconciler.reconcile(to: .inactive)
        gate.release()
        await reconciler.waitUntilSettled()

        #expect(recorder.activations == 1)
        #expect(recorder.postActivations == 0)
        #expect(recorder.deactivations == 1)
    }

    @Test("A rapid foreground during suspended shutdown restarts the session")
    func foregroundDuringShutdownRestarts() async {
        let shutdownGate = SuspensionGate()
        let recorder = Recorder(deactivationGate: shutdownGate)
        let reconciler = recorder.makeReconciler()

        reconciler.reconcile(to: .active(pairingGeneration: 2))
        await reconciler.waitUntilSettled()
        reconciler.reconcile(to: .inactive)
        await shutdownGate.waitUntilEntered()
        reconciler.reconcile(to: .active(pairingGeneration: 2))
        shutdownGate.release()
        await reconciler.waitUntilSettled()

        #expect(recorder.activations == 2)
        #expect(recorder.postActivations == 2)
        #expect(recorder.deactivations == 1)
    }
}

@MainActor
private final class Recorder {
    private let activationGate: SuspensionGate?
    private let deactivationGate: SuspensionGate?
    private(set) var activations = 0
    private(set) var postActivations = 0
    private(set) var deactivations = 0

    init(
        activationGate: SuspensionGate? = nil,
        deactivationGate: SuspensionGate? = nil
    ) {
        self.activationGate = activationGate
        self.deactivationGate = deactivationGate
    }

    func makeReconciler() -> ForegroundLifecycleReconciler {
        ForegroundLifecycleReconciler(
            actions: .init(
                activate: { [weak self] in
                    guard let self else { return }
                    self.activations += 1
                    await self.activationGate?.suspend()
                },
                didActivate: { [weak self] in
                    self?.postActivations += 1
                },
                deactivate: { [weak self] in
                    guard let self else { return }
                    self.deactivations += 1
                    await self.deactivationGate?.suspend()
                }))
    }
}

@MainActor
private final class SuspensionGate {
    private var entryContinuation: CheckedContinuation<Void, Never>?
    private var releaseContinuation: CheckedContinuation<Void, Never>?
    private var entered = false
    private var released = false

    func suspend() async {
        entered = true
        entryContinuation?.resume()
        entryContinuation = nil
        guard !released else { return }
        await withCheckedContinuation { continuation in
            releaseContinuation = continuation
        }
    }

    func waitUntilEntered() async {
        guard !entered else { return }
        await withCheckedContinuation { continuation in
            entryContinuation = continuation
        }
    }

    func release() {
        released = true
        releaseContinuation?.resume()
        releaseContinuation = nil
    }
}
