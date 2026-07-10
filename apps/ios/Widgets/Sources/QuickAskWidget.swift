import SwiftUI
import ThinClawSnapshotKit
import ThinClawWidgetKitShared
import WidgetKit

/// One-tap prompt launcher. The button fires `QuickAskIntent`, which sends the
/// prompt over the pinned session and writes a `QuickAskReceipt`; the answer
/// arrives as a push. The widget renders the most recent receipt so the user
/// gets a "sent ✓ / couldn't send" glance without opening the app.
struct QuickAskWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: WidgetReload.Kind.quickAsk,
            provider: QuickAskProvider()
        ) { entry in
            QuickAskView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Quick ask")
        .description("Send a prompt to your agent without opening the app.")
        .supportedFamilies([.systemSmall, .accessoryCircular])
    }
}

struct QuickAskEntry: TimelineEntry {
    let date: Date
    let receipt: QuickAskReceipt?
}

struct QuickAskProvider: TimelineProvider {
    func placeholder(in context: Context) -> QuickAskEntry {
        QuickAskEntry(date: .now, receipt: nil)
    }

    func getSnapshot(in context: Context, completion: @escaping (QuickAskEntry) -> Void) {
        completion(currentEntry())
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<QuickAskEntry>) -> Void) {
        // Static launcher; no scheduled churn. The intent reloads this kind
        // after writing a receipt.
        completion(Timeline(entries: [currentEntry()], policy: .never))
    }

    private func currentEntry() -> QuickAskEntry {
        QuickAskEntry(date: .now, receipt: WidgetSnapshotAccess.load(QuickAskReceipt.self))
    }
}

struct QuickAskView: View {
    @Environment(\.widgetFamily) private var family
    let entry: QuickAskEntry

    var body: some View {
        switch family {
        case .accessoryCircular:
            circularBody
        default:
            smallBody
        }
    }

    private var smallBody: some View {
        Button(intent: QuickAskIntent(prompt: "Give me a concise status update.")) {
            VStack(spacing: 6) {
                Image(systemName: "bubble.left.and.text.bubble.right")
                    .font(.title2)
                Text("Ask ThinClaw")
                    .font(.caption)
                if let receipt = entry.receipt, isRecent(receipt.generatedAt) {
                    receiptLabel(receipt)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .buttonStyle(.plain)
    }

    private var circularBody: some View {
        Button(intent: QuickAskIntent(prompt: "Give me a concise status update.")) {
            ZStack {
                AccessoryWidgetBackground()
                Image(systemName: "bubble.left.and.text.bubble.right")
                    .font(.title3)
            }
        }
        .buttonStyle(.plain)
    }

    @ViewBuilder
    private func receiptLabel(_ receipt: QuickAskReceipt) -> some View {
        switch receipt.deliveryState {
        case .sent:
            Label("Sent", systemImage: "checkmark.circle")
                .font(.caption2).foregroundStyle(.green).labelStyle(.titleAndIcon)
        case .queued:
            Label("Queued", systemImage: "clock")
                .font(.caption2).foregroundStyle(.secondary).labelStyle(.titleAndIcon)
        case .failed:
            Label("Not sent", systemImage: "exclamationmark.triangle")
                .font(.caption2).foregroundStyle(.orange).labelStyle(.titleAndIcon)
        }
    }

    /// Only surface a receipt for a short window so an old "sent" state does
    /// not linger as if it were the current status.
    private func isRecent(_ date: Date, within: TimeInterval = 5 * 60) -> Bool {
        entry.date.timeIntervalSince(date) <= within
    }
}
