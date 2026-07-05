import SwiftUI
import ThinClawWidgetKitShared
import WidgetKit

/// One-tap prompt launcher. The button fires `QuickAskIntent` (which opens
/// dictation/typing via the App Shortcut) — answers come back as pushes.
struct QuickAskWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: "com.thinclaw.ios.widget.quickask",
            provider: QuickAskProvider()
        ) { entry in
            QuickAskView()
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Quick ask")
        .description("Send a prompt to your agent without opening the app.")
        .supportedFamilies([.systemSmall, .accessoryCircular])
    }
}

struct QuickAskEntry: TimelineEntry {
    let date: Date
}

struct QuickAskProvider: TimelineProvider {
    func placeholder(in context: Context) -> QuickAskEntry { QuickAskEntry(date: .now) }
    func getSnapshot(in context: Context, completion: @escaping (QuickAskEntry) -> Void) {
        completion(QuickAskEntry(date: .now))
    }
    func getTimeline(in context: Context, completion: @escaping (Timeline<QuickAskEntry>) -> Void) {
        completion(Timeline(entries: [QuickAskEntry(date: .now)], policy: .never))
    }
}

struct QuickAskView: View {
    var body: some View {
        VStack(spacing: 6) {
            Image(systemName: "bubble.left.and.text.bubble.right")
                .font(.title2)
            Text("Ask ThinClaw")
                .font(.caption)
        }
        .widgetURL(URL(string: "thinclaw://quick-ask"))
    }
}
