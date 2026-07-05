import SwiftUI
import ThinClawSnapshotKit
import ThinClawWidgetKitShared
import WidgetKit

/// Interactive approvals widget: approve/deny low-risk tool requests without
/// opening the app (AppIntent buttons). High-risk requests render as a
/// deep-link row only — biometric approval happens in the app.
struct PendingApprovalsWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: "com.thinclaw.ios.widget.approvals",
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
}

struct PendingApprovalsProvider: TimelineProvider {
    func placeholder(in context: Context) -> PendingApprovalsEntry {
        PendingApprovalsEntry(date: .now, approvals: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (PendingApprovalsEntry) -> Void) {
        completion(PendingApprovalsEntry(date: .now, approvals: nil))
    }

    func getTimeline(
        in context: Context, completion: @escaping (Timeline<PendingApprovalsEntry>) -> Void
    ) {
        // M3: read PendingApprovalsSnapshot from the App Group store.
        let entry = PendingApprovalsEntry(date: .now, approvals: nil)
        completion(Timeline(entries: [entry], policy: .after(.now.addingTimeInterval(15 * 60))))
    }
}

struct PendingApprovalsView: View {
    let entry: PendingApprovalsEntry

    var body: some View {
        if let snapshot = entry.approvals, !snapshot.approvals.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                ForEach(snapshot.approvals.prefix(3)) { item in
                    HStack {
                        VStack(alignment: .leading) {
                            Text(item.toolName).font(.caption).bold()
                            Text(item.description).font(.caption2).foregroundStyle(.secondary)
                                .lineLimit(1)
                        }
                        Spacer()
                        Button(intent: ApproveToolIntent(requestID: item.id, threadID: item.threadID)) {
                            Image(systemName: "checkmark")
                        }
                        Button(intent: DenyToolIntent(requestID: item.id, threadID: item.threadID)) {
                            Image(systemName: "xmark")
                        }
                    }
                }
            }
        } else {
            Label("No pending approvals", systemImage: "checkmark.shield")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }
}
