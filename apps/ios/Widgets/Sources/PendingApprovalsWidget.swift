import SwiftUI
import ThinClawSnapshotKit
import ThinClawWidgetKitShared
import WidgetKit

/// Interactive approvals widget: approve/deny low-risk tool requests without
/// opening the app (AppIntent buttons). High-risk requests render a deep-link
/// row only — biometric approval happens in the app (docs/MOBILE_SECURITY.md
/// D-K3). Risk enforcement is done here (render side) AND re-checked inside
/// `ApproveToolIntent` (perform side), so a lock screen can never approve a
/// high-risk action.
struct PendingApprovalsWidget: Widget {
    /// Max rows we render; keeps the medium/large family legible.
    static let maxRows = 4

    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: WidgetReload.Kind.approvals,
            provider: PendingApprovalsProvider()
        ) { entry in
            PendingApprovalsView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Pending approvals")
        .description("Approve or deny tool requests from your home screen.")
        .supportedFamilies([.systemMedium, .systemLarge])
    }
}

struct PendingApprovalsEntry: TimelineEntry {
    let date: Date
    let approvals: PendingApprovalsSnapshot?
    let isStale: Bool
}

struct PendingApprovalsProvider: TimelineProvider {
    private static let refreshInterval: TimeInterval = 15 * 60

    func placeholder(in context: Context) -> PendingApprovalsEntry {
        PendingApprovalsEntry(date: .now, approvals: nil, isStale: false)
    }

    func getSnapshot(in context: Context, completion: @escaping (PendingApprovalsEntry) -> Void) {
        completion(Self.currentEntry())
    }

    func getTimeline(
        in context: Context, completion: @escaping (Timeline<PendingApprovalsEntry>) -> Void
    ) {
        let entry = Self.currentEntry()
        let next = Date.now.addingTimeInterval(Self.refreshInterval)
        completion(Timeline(entries: [entry], policy: .after(next)))
    }

    private static func currentEntry() -> PendingApprovalsEntry {
        let snapshot = WidgetSnapshotAccess.load(PendingApprovalsSnapshot.self)
        return PendingApprovalsEntry(
            date: .now,
            approvals: snapshot,
            isStale: snapshot?.isStale() ?? false
        )
    }
}

struct PendingApprovalsView: View {
    let entry: PendingApprovalsEntry

    var body: some View {
        if let snapshot = entry.approvals, !snapshot.approvals.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(snapshot.approvals.prefix(PendingApprovalsWidget.maxRows)) { item in
                    ApprovalRow(item: item)
                }

                let overflow = snapshot.approvals.count - PendingApprovalsWidget.maxRows
                if overflow > 0 {
                    Text("+\(overflow) more in app")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }

                if entry.isStale {
                    Text("Stale as of \(snapshot.generatedAt.formatted(.relative(presentation: .named)))")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        } else if entry.approvals == nil {
            // No snapshot at all → not connected / not paired.
            VStack(spacing: 4) {
                Label("Not connected", systemImage: "shield.slash")
                    .font(.caption)
                Text("Open the app to pair").font(.caption2).foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .widgetURL(AppRoute.approvals(requestID: nil, threadID: nil).url)
        } else {
            Label("No pending approvals", systemImage: "checkmark.shield")
                .font(.caption)
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

/// One approval row. Low-risk rows carry inline Approve/Deny buttons;
/// high-risk (or unknown-risk) rows carry a Deny button and a deep link into
/// the app — the approve action is never offered off-device (D-K3).
private struct ApprovalRow: View {
    let item: PendingApprovalsSnapshot.PendingApproval

    private var isLowRisk: Bool { item.effectiveRisk == .low }

    var body: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 4) {
                    Text(item.toolName).font(.caption).bold().lineLimit(1)
                    if !isLowRisk {
                        Text("HIGH RISK")
                            .font(.system(size: 8, weight: .heavy))
                            .padding(.horizontal, 4)
                            .padding(.vertical, 1)
                            .background(.red.opacity(0.15), in: Capsule())
                            .foregroundStyle(.red)
                    }
                }
                Text(item.description)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 4)

            if isLowRisk {
                // Low-risk: inline approve + deny. The risk tier is stamped
                // onto the intent so the perform side can re-verify (D-K3).
                Button(
                    intent: ApproveToolIntent(
                        requestID: item.id, threadID: item.threadID, risk: "low")
                ) {
                    Image(systemName: "checkmark")
                }
                .tint(.green)

                Button(intent: DenyToolIntent(requestID: item.id, threadID: item.threadID)) {
                    Image(systemName: "xmark")
                }
                .tint(.red)
            } else {
                // High-risk: deny is safe from any surface; approve requires
                // the in-app biometric gate, so we deep-link instead.
                Button(intent: DenyToolIntent(requestID: item.id, threadID: item.threadID)) {
                    Image(systemName: "xmark")
                }
                .tint(.red)

                Link(destination: approvalDeepLink(item)) {
                    Image(systemName: "lock.open")
                }
            }
        }
    }

    private func approvalDeepLink(_ item: PendingApprovalsSnapshot.PendingApproval) -> URL {
        AppRoute.approvals(requestID: item.id, threadID: item.threadID).url
    }
}
