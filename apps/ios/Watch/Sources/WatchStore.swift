import Foundation
import Observation
import ThinClawSnapshotKit
import ThinClawWatchBridge

/// Observable state backing the whole watch surface.
///
/// Holds the mirrored snapshot bundle (status + pending approvals), the live
/// transport route badge, and the per-approval / per-ask in-flight state the
/// views render as spinners and haptics. All gateway I/O is delegated to a
/// ``WatchGatewayProxy`` (relay-first; the watch attaches its own reduced-scope
/// token — docs/MOBILE_SECURITY.md D-K4). The store never talks to the network
/// directly.
@MainActor
@Observable
public final class WatchStore {
    /// The freshest mirrored snapshot bundle; `nil` until the first mirror.
    public private(set) var bundle: WatchSnapshotBundle?

    /// The transport route the next request would take, for the badge.
    public private(set) var route: WatchRoute

    /// Approval ids currently awaiting a round-trip (spinner shown).
    public private(set) var inFlightApprovalIDs: Set<String> = []

    /// The most recent quick-ask receipt, driving the "sent / queued / failed"
    /// confirmation on the Ask screen.
    public private(set) var lastAskReceipt: QuickAskReceipt?

    /// A transient, human-readable error from the last failed action, if any.
    public private(set) var lastError: String?

    private let proxy: any WatchGatewayProxy

    public init(proxy: any WatchGatewayProxy) {
        self.proxy = proxy
        self.route = proxy.currentRoute()
    }

    // MARK: - Derived, glanceable state

    /// The agent status projection, if mirrored.
    public var status: AgentStatusSnapshot? { bundle?.status }

    /// Pending approvals, newest-relevant first (as mirrored).
    public var approvals: [PendingApprovalsSnapshot.PendingApproval] {
        bundle?.approvals?.approvals ?? []
    }

    /// Count of pending approvals — drives the root badge and complication.
    public var pendingCount: Int { approvals.count }

    /// True when we have never received a snapshot (render "open watch app" /
    /// "pair on iPhone" affordances rather than implying an empty-but-live
    /// state).
    public var hasSnapshot: Bool { bundle != nil }

    // MARK: - Snapshot refresh

    /// Pull the latest mirrored snapshot and refresh the route badge.
    public func refresh() async {
        route = proxy.currentRoute()
        if let bundle = await proxy.refreshSnapshot() {
            self.bundle = bundle
        }
    }

    // MARK: - Approvals

    /// Whether a given entry may be *approved* from the wrist. Deny is always
    /// allowed; approve is offered only for low-risk entries (D-K3/D-K4). A
    /// missing tier reads back as high (fail-closed) via ``effectiveRisk``.
    public func canApproveOnWatch(
        _ approval: PendingApprovalsSnapshot.PendingApproval
    ) -> Bool {
        approval.effectiveRisk == .low
    }

    /// Result of a wrist decision, so the view can fire the right haptic.
    public enum DecisionOutcome: Sendable, Equatable {
        /// The gateway accepted the decision.
        case accepted
        /// The request was queued and will send when reachable.
        case queued
        /// The decision failed (surfaced as an error haptic + message).
        case failed(String)
        /// A high-risk approve was blocked on the watch before any send
        /// (defense in depth alongside the server-side refusal).
        case blockedHighRisk
    }

    /// Approve a low-risk entry. Refuses (without sending) if the entry is not
    /// low-risk — the server would refuse it too, but we never even offer the
    /// approve path off-device.
    public func approve(
        _ approval: PendingApprovalsSnapshot.PendingApproval
    ) async -> DecisionOutcome {
        guard canApproveOnWatch(approval) else { return .blockedHighRisk }
        return await decide(id: approval.id, action: "approve")
    }

    /// Deny an entry. Always permitted, at any risk tier.
    public func deny(
        _ approval: PendingApprovalsSnapshot.PendingApproval
    ) async -> DecisionOutcome {
        await decide(id: approval.id, action: "deny")
    }

    private func decide(id: String, action: String) async -> DecisionOutcome {
        inFlightApprovalIDs.insert(id)
        defer { inFlightApprovalIDs.remove(id) }

        // Snapshot the route *before* sending so a queued send is reported
        // honestly even if reachability flips mid-flight.
        let queued = proxy.currentRoute() == .queued
        let response = await proxy.approve(id: id, action: action)

        switch response {
        case .accepted:
            // Optimistically drop the row from the mirrored bundle so the list
            // updates immediately; the next real mirror reconciles.
            removeApprovalLocally(id: id)
            lastError = nil
            return queued ? .queued : .accepted
        case let .failed(reason):
            lastError = reason
            return .failed(reason)
        case .reprovisionRequired:
            // The watch's companion credential is missing/revoked; the phone
            // re-mints on next reachability (D-K4). Surface it as a failure so
            // the row stays and the operator retries once re-provisioned.
            let message = "Re-provisioning — open ThinClaw on iPhone"
            lastError = message
            return .failed(message)
        }
    }

    private func removeApprovalLocally(id: String) {
        guard var approvals = bundle?.approvals else { return }
        approvals.approvals.removeAll { $0.id == id }
        bundle?.approvals = approvals
    }

    // MARK: - Quick Ask

    /// Send a dictated prompt. Returns the receipt the Ask view renders as a
    /// "sent" / "will send when reachable" confirmation.
    public func quickAsk(_ prompt: String) async -> QuickAskReceipt {
        let trimmed = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        let queued = proxy.currentRoute() == .queued
        let response = await proxy.quickAsk(prompt: trimmed)

        let state: QuickAskReceipt.DeliveryState
        switch response {
        case .accepted:
            state = queued ? .queued : .sent
            lastError = nil
        case let .failed(reason):
            state = .failed
            lastError = reason
        case .reprovisionRequired:
            state = .failed
            lastError = "Re-provisioning — open ThinClaw on iPhone"
        }

        let receipt = QuickAskReceipt(
            generatedAt: .now,
            text: trimmed,
            deliveryState: state
        )
        lastAskReceipt = receipt
        return receipt
    }
}
