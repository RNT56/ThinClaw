import SwiftUI
import ThinClawSnapshotKit
import ThinClawWidgetKitShared
import WidgetKit

/// Home/lock-screen glance: instance reachability, active session phase, active
/// thread title, and a pending-approvals count. Reads the App Group snapshots
/// written by the app; timelines reload on push-driven snapshot writes (the app
/// calls `WidgetCenter`) and on the scheduled refresh below.
struct AgentStatusWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: WidgetReload.Kind.status,
            provider: AgentStatusProvider()
        ) { entry in
            AgentStatusView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Agent status")
        .description("Reachability, running work, and pending approvals.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular])
    }
}

struct AgentStatusEntry: TimelineEntry {
    let date: Date
    let snapshot: AgentStatusSnapshot?
    let pendingCount: Int
    /// True when the newest snapshot is old enough that we should badge it
    /// rather than imply live data.
    let isStale: Bool
}

struct AgentStatusProvider: TimelineProvider {
    /// Scheduled refresh cadence. WidgetKit budgets refreshes, so we ask for a
    /// modest ~20 min cadence and rely on push-driven `WidgetCenter` reloads
    /// (from the app) for timeliness.
    private static let refreshInterval: TimeInterval = 20 * 60

    func placeholder(in context: Context) -> AgentStatusEntry {
        AgentStatusEntry(date: .now, snapshot: nil, pendingCount: 0, isStale: false)
    }

    func getSnapshot(in context: Context, completion: @escaping (AgentStatusEntry) -> Void) {
        completion(Self.currentEntry())
    }

    func getTimeline(
        in context: Context, completion: @escaping (Timeline<AgentStatusEntry>) -> Void
    ) {
        let entry = Self.currentEntry()
        let next = Date.now.addingTimeInterval(Self.refreshInterval)
        completion(Timeline(entries: [entry], policy: .after(next)))
    }

    /// Read both snapshots defensively; any failure degrades to placeholder
    /// content and never crashes the extension.
    private static func currentEntry() -> AgentStatusEntry {
        let status = WidgetSnapshotAccess.load(AgentStatusSnapshot.self)
        let approvals = WidgetSnapshotAccess.load(PendingApprovalsSnapshot.self)
        return AgentStatusEntry(
            date: .now,
            snapshot: status,
            pendingCount: approvals?.approvals.count ?? 0,
            isStale: status?.isStale() ?? false
        )
    }
}

struct AgentStatusView: View {
    @Environment(\.widgetFamily) private var family
    let entry: AgentStatusEntry

    var body: some View {
        switch family {
        case .accessoryRectangular:
            rectangularBody
        default:
            standardBody
        }
    }

    // MARK: Home-screen (small / medium)

    private var standardBody: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label("ThinClaw", systemImage: "brain")
                .font(.headline)
                .lineLimit(1)

            if let snapshot = entry.snapshot {
                Text(snapshot.activeThreadTitle ?? phaseLabel(snapshot.phase))
                    .font(.caption)
                    .lineLimit(1)

                if snapshot.activeThreadTitle != nil {
                    Text(phaseLabel(snapshot.phase))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }

                if entry.pendingCount > 0 {
                    Label(
                        "\(entry.pendingCount) pending",
                        systemImage: "checkmark.shield"
                    )
                    .font(.caption2)
                    .foregroundStyle(.orange)
                }

                if entry.isStale {
                    staleBadge(snapshot.generatedAt)
                }
            } else {
                Text("Not connected")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Text("Open the app to pair")
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .widgetURL(URL(string: "thinclaw://thread"))
    }

    // MARK: Lock-screen rectangular

    private var rectangularBody: some View {
        VStack(alignment: .leading, spacing: 1) {
            if let snapshot = entry.snapshot {
                Label(phaseLabel(snapshot.phase), systemImage: phaseIcon(snapshot.phase))
                    .font(.caption).bold()
                    .lineLimit(1)
                if let title = snapshot.activeThreadTitle {
                    Text(title).font(.caption2).lineLimit(1)
                }
                if entry.pendingCount > 0 {
                    Text("\(entry.pendingCount) pending approval\(entry.pendingCount == 1 ? "" : "s")")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else if entry.isStale {
                    Text("Stale as of \(snapshot.generatedAt.formatted(.relative(presentation: .named)))")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            } else {
                Label("Not connected", systemImage: "brain")
                    .font(.caption).bold()
                Text("Open app to pair").font(.caption2).foregroundStyle(.secondary)
            }
        }
        .widgetURL(URL(string: "thinclaw://thread"))
    }

    // MARK: Helpers

    @ViewBuilder
    private func staleBadge(_ generatedAt: Date) -> some View {
        Text("Stale as of \(generatedAt.formatted(.relative(presentation: .named)))")
            .font(.caption2)
            .foregroundStyle(.secondary)
            .lineLimit(1)
    }

    private func phaseLabel(_ phase: AgentStatusSnapshot.Phase) -> String {
        switch phase {
        case .idle: "Idle"
        case .thinking: "Thinking…"
        case .streaming: "Responding…"
        case .runningTool: "Running a tool"
        case .waitingForApproval: "Needs approval"
        case .error: "Error"
        }
    }

    private func phaseIcon(_ phase: AgentStatusSnapshot.Phase) -> String {
        switch phase {
        case .idle: "moon.zzz"
        case .thinking: "brain"
        case .streaming: "text.bubble"
        case .runningTool: "gearshape.2"
        case .waitingForApproval: "checkmark.shield"
        case .error: "exclamationmark.triangle"
        }
    }
}
