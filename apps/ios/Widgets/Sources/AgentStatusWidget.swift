import SwiftUI
import ThinClawSnapshotKit
import WidgetKit

/// Home/lock-screen glance: instance reachability, active session, running
/// jobs, pending approvals count. Reads the App Group snapshot written by
/// the app; timelines reload on push-driven snapshot writes and BGAppRefresh.
struct AgentStatusWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: "com.thinclaw.ios.widget.status",
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
}

struct AgentStatusProvider: TimelineProvider {
    func placeholder(in context: Context) -> AgentStatusEntry {
        AgentStatusEntry(date: .now, snapshot: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (AgentStatusEntry) -> Void) {
        // M3: read AgentStatusSnapshot from the App Group SnapshotStore.
        completion(AgentStatusEntry(date: .now, snapshot: nil))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<AgentStatusEntry>) -> Void) {
        let entry = AgentStatusEntry(date: .now, snapshot: nil)
        completion(Timeline(entries: [entry], policy: .after(.now.addingTimeInterval(30 * 60))))
    }
}

struct AgentStatusView: View {
    let entry: AgentStatusEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Label("ThinClaw", systemImage: "brain")
                .font(.headline)
            if let snapshot = entry.snapshot {
                Text(snapshot.activeThreadTitle ?? statusLabel(for: snapshot.phase))
                    .font(.caption)
                    .lineLimit(1)
                if snapshot.unreadCount > 0 {
                    Text("\(snapshot.unreadCount) unread")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            } else {
                Text("Open the app to pair")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .widgetURL(URL(string: "thinclaw://thread"))
    }

    private func statusLabel(for phase: AgentStatusSnapshot.Phase) -> String {
        switch phase {
        case .idle: "Idle"
        case .thinking: "Thinking…"
        case .streaming: "Responding…"
        case .runningTool: "Running a tool"
        case .waitingForApproval: "Needs approval"
        case .error: "Error"
        }
    }
}
