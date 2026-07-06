import SwiftUI
import ThinClawSnapshotKit

#if canImport(WatchKit)
    import WatchKit
#endif

/// Pending approvals on the wrist.
///
/// Low-risk entries get Approve/Deny with a success/failure haptic. High-risk
/// (or unknown-risk, which reads back as high) entries show **"Approve on
/// iPhone"** and are tappable to hand off — there is NO wrist approve for them
/// (docs/MOBILE_SECURITY.md D-K3/D-K4). Deny is always available. Decisions go
/// through ``WatchStore`` → ``WatchGatewayProxy`` (which forwards the watch's
/// own token); a spinner shows during the round-trip, then a haptic fires, and
/// a queued send is labelled "will send when reachable".
struct ApprovalsListView: View {
    @State var store: WatchStore

    var body: some View {
        Group {
            if store.approvals.isEmpty {
                ContentUnavailableView(
                    store.hasSnapshot ? "No pending approvals" : "Open watch app",
                    systemImage: "checkmark.shield",
                    description: Text(
                        store.hasSnapshot
                            ? "You're all caught up."
                            : "Waiting for the paired iPhone to mirror status."
                    )
                )
            } else {
                List {
                    ForEach(store.approvals) { approval in
                        ApprovalRow(store: store, approval: approval)
                    }
                }
            }
        }
        .navigationTitle("Approvals")
        .task { await store.refresh() }
        .refreshable { await store.refresh() }
    }
}

/// One approval row. Renders the tool name, a risk badge, the description, and
/// the correct action set for the tier.
private struct ApprovalRow: View {
    let store: WatchStore
    let approval: PendingApprovalsSnapshot.PendingApproval

    @State private var showHandoff = false

    private var isLowRisk: Bool { store.canApproveOnWatch(approval) }
    private var isInFlight: Bool { store.inFlightApprovalIDs.contains(approval.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            header
            Text(approval.description)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(3)

            if isInFlight {
                HStack(spacing: 6) {
                    ProgressView()
                    Text("Sending…").font(.caption2).foregroundStyle(.secondary)
                }
            } else {
                actions
            }
        }
        .padding(.vertical, 2)
    }

    private var header: some View {
        HStack(spacing: 4) {
            Text(approval.toolName)
                .font(.caption).bold()
                .lineLimit(1)
            Spacer()
            RiskBadge(isLowRisk: isLowRisk)
        }
    }

    @ViewBuilder
    private var actions: some View {
        if isLowRisk {
            HStack(spacing: 8) {
                Button(role: .destructive) {
                    Task { await runDeny() }
                } label: {
                    Label("Deny", systemImage: "xmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .tint(.red)

                Button {
                    Task { await runApprove() }
                } label: {
                    Label("Approve", systemImage: "checkmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent)
                .tint(.green)
            }
        } else {
            // High-risk: deny is safe from any surface; approve is refused on
            // the wrist and hands off to the iPhone (D-K3/D-K4).
            VStack(spacing: 6) {
                Button {
                    showHandoff = true
                } label: {
                    Label("Approve on iPhone", systemImage: "iphone")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)

                Button(role: .destructive) {
                    Task { await runDeny() }
                } label: {
                    Label("Deny", systemImage: "xmark")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.bordered)
                .tint(.red)
            }
            .alert("Approve on iPhone", isPresented: $showHandoff) {
                Button("OK", role: .cancel) {}
            } message: {
                Text(
                    "High-risk actions need Face ID on your iPhone. Open ThinClaw there to approve."
                )
            }
        }
    }

    // MARK: - Decisions + haptics

    private func runApprove() async {
        let outcome = await store.approve(approval)
        play(for: outcome)
    }

    private func runDeny() async {
        let outcome = await store.deny(approval)
        play(for: outcome)
    }

    private func play(for outcome: WatchStore.DecisionOutcome) {
        #if canImport(WatchKit)
            switch outcome {
            case .accepted:
                WKInterfaceDevice.current().play(.success)
            case .queued:
                WKInterfaceDevice.current().play(.notification)
            case .failed, .blockedHighRisk:
                WKInterfaceDevice.current().play(.failure)
            }
        #endif
    }
}

/// Compact risk badge: a quiet "low" or a loud "HIGH RISK" that reads on the
/// wrist. A missing tier already reads back as high (fail-closed).
private struct RiskBadge: View {
    let isLowRisk: Bool

    var body: some View {
        if isLowRisk {
            Text("low")
                .font(.system(size: 9, weight: .semibold))
                .foregroundStyle(.secondary)
        } else {
            Text("HIGH RISK")
                .font(.system(size: 9, weight: .heavy))
                .padding(.horizontal, 4)
                .padding(.vertical, 1)
                .background(.red.opacity(0.18), in: Capsule())
                .foregroundStyle(.red)
        }
    }
}
