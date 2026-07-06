import SwiftUI
import ThinClawCore
import ThinClawDesign

/// Pending tool approvals, presented as a badge-driven sheet (not a tab).
///
/// The store (`ThinClawCore.ApprovalsStore`) is UI-free and macOS-tested; this
/// view is the thin iOS shell that renders its ``ApprovalRequest`` list as
/// risk-tiered ``ApprovalCard``s and forwards decisions. High-risk approvals
/// are biometric-gated inside the store before the decision fires
/// (docs/MOBILE_SECURITY.md, D-K3).
public struct ApprovalsScreen: View {
    @State private var store: ApprovalsStore

    public init(store: ApprovalsStore) {
        self._store = State(initialValue: store)
    }

    public var body: some View {
        Group {
            if store.pending.isEmpty {
                ContentUnavailableView(
                    "No pending approvals",
                    systemImage: "checkmark.shield",
                    description: Text("Tool requests that need your decision appear here.")
                )
            } else {
                ScrollView {
                    LazyVStack(spacing: ThinClawSpacing.md) {
                        ForEach(store.pending) { request in
                            ApprovalCard(
                                toolName: request.toolName,
                                requestDescription: request.description,
                                risk: request.risk.designTier,
                                onApprove: {
                                    Task { await store.approve(request.requestID) }
                                },
                                onDeny: {
                                    Task { await store.deny(request.requestID) }
                                }
                            )
                        }
                    }
                    .padding(ThinClawSpacing.md)
                }
            }
        }
        .navigationTitle("Approvals")
        .refreshable { await store.refresh() }
        .task { await store.start() }
    }
}

extension RiskTier {
    /// Bridge the canonical domain ``RiskTier`` (owned by ThinClawCore) onto
    /// the design system's ``ApprovalCard/RiskTier``. ThinClawDesign is kept
    /// dependency-free (widgets and the watch import it without Core), so the
    /// two enums are mirrors and the app layer maps between them here.
    var designTier: ApprovalCard.RiskTier {
        switch self {
        case .low: return .low
        case .high: return .high
        }
    }
}
