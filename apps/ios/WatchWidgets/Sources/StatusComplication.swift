import SwiftUI
import ThinClawSnapshotKit
import WidgetKit

/// Watch complication / corner widget: agent phase + pending approvals
/// count, from the snapshot mirrored over WatchConnectivity (M4).
@main
struct ThinClawWatchWidgetsBundle: WidgetBundle {
    var body: some Widget {
        StatusComplication()
    }
}

struct StatusComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: "com.thinclaw.ios.watch.status",
            provider: ComplicationProvider()
        ) { entry in
            Image(systemName: entry.hasPendingApprovals ? "checkmark.shield.fill" : "brain")
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("ThinClaw")
        .description("Agent status at a glance.")
        .supportedFamilies([.accessoryCircular, .accessoryCorner, .accessoryInline])
    }
}

struct ComplicationEntry: TimelineEntry {
    let date: Date
    let hasPendingApprovals: Bool
}

struct ComplicationProvider: TimelineProvider {
    func placeholder(in context: Context) -> ComplicationEntry {
        ComplicationEntry(date: .now, hasPendingApprovals: false)
    }

    func getSnapshot(in context: Context, completion: @escaping (ComplicationEntry) -> Void) {
        completion(ComplicationEntry(date: .now, hasPendingApprovals: false))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<ComplicationEntry>) -> Void) {
        // M4: read the mirrored snapshot from the watch App Group store.
        let entry = ComplicationEntry(date: .now, hasPendingApprovals: false)
        completion(Timeline(entries: [entry], policy: .after(.now.addingTimeInterval(30 * 60))))
    }
}
