import SwiftUI
import ThinClawCore
import ThinClawDesign

/// Pending tool approvals, presented as a badge-driven sheet (not a tab).
/// High-risk approvals require Face ID before the store fires the decision
/// (docs/MOBILE_SECURITY.md, D-K3).
@MainActor
@Observable
public final class ApprovalsStore {
    public private(set) var pending: [ApprovalRequest] = []

    public init() {}

    /// M2: POST /api/chat/approval, biometric-gated for high risk;
    /// pull-refresh from GET /api/chat/approvals.
    public func respond(_ requestID: String, approve: Bool) async {}
}

public struct ApprovalsScreen: View {
    @State private var store: ApprovalsStore

    public init(store: ApprovalsStore = ApprovalsStore()) {
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
                                risk: .low,
                                onApprove: {
                                    Task { await store.respond(request.requestID, approve: true) }
                                },
                                onDeny: {
                                    Task { await store.respond(request.requestID, approve: false) }
                                }
                            )
                        }
                    }
                    .padding(ThinClawSpacing.md)
                }
            }
        }
        .navigationTitle("Approvals")
    }
}
