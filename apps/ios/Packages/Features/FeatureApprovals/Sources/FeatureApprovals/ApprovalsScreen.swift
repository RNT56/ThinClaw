import SwiftUI
import ThinClawCore
import ThinClawDesign

/// Pending tool approvals, presented as a badge-driven main tab.
///
/// The store (`ThinClawCore.ApprovalsStore`) is UI-free and macOS-tested; this
/// view is the thin iOS shell that renders its ``ApprovalRequest`` list as
/// risk-tiered ``ApprovalCard``s and forwards decisions. High-risk approvals
/// are biometric-gated inside the store before the decision fires
/// (docs/MOBILE_SECURITY.md, D-K3).
public struct ApprovalsScreen: View {
    @State private var store: ApprovalsStore
    @Binding private var focusedRequestID: String?

    public init(
        store: ApprovalsStore,
        focusedRequestID: Binding<String?> = .constant(nil)
    ) {
        self._store = State(initialValue: store)
        self._focusedRequestID = focusedRequestID
    }

    public var body: some View {
        Group {
            if store.pending.isEmpty {
                VStack(spacing: ThinClawSpacing.md) {
                    statusMessages
                    ContentUnavailableView(
                        "No pending approvals",
                        systemImage: "checkmark.shield",
                        description: Text("Tool requests that need your decision appear here.")
                    )
                }
            } else {
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: ThinClawSpacing.md) {
                            statusMessages
                            ForEach(store.pending) { request in
                                ApprovalCard(
                                    toolName: request.toolName,
                                    requestDescription: request.description,
                                    parameters: request.parameters,
                                    risk: request.risk.designTier,
                                    onApprove: {
                                        Task { await store.approve(request.requestID) }
                                    },
                                    onDeny: {
                                        Task { await store.deny(request.requestID) }
                                    }
                                )
                                .id(request.requestID)
                                .disabled(store.submittingRequestIDs.contains(request.requestID))
                                .overlay(alignment: .topTrailing) {
                                    if store.submittingRequestIDs.contains(request.requestID) {
                                        ProgressView()
                                            .padding(ThinClawSpacing.md)
                                    }
                                }
                            }
                        }
                        .padding(ThinClawSpacing.md)
                    }
                    .onChange(of: store.pending) { _, _ in
                        guard let focusedRequestID else { return }
                        withAnimation { proxy.scrollTo(focusedRequestID, anchor: .center) }
                        self.focusedRequestID = nil
                    }
                    .task {
                        guard let focusedRequestID else { return }
                        proxy.scrollTo(focusedRequestID, anchor: .center)
                        self.focusedRequestID = nil
                    }
                }
            }
        }
        .navigationTitle("Approvals")
        .refreshable { await store.refresh() }
        .task { await store.start() }
    }

    @ViewBuilder
    private var statusMessages: some View {
        if let error = store.errorMessage {
            Label(error, systemImage: "exclamationmark.triangle.fill")
                .font(ThinClawTypography.caption)
                .foregroundStyle(.red)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("approvals.error")
        } else if let notice = store.notice {
            Label(notice, systemImage: "info.circle")
                .font(ThinClawTypography.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier("approvals.notice")
        }
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
