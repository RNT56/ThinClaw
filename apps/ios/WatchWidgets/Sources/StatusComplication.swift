import SwiftUI
import ThinClawSnapshotKit
import WidgetKit

/// Watch complication bundle: a single glanceable status complication driven
/// by the snapshot the phone mirrors into the watch App Group over
/// WatchConnectivity (docs/MOBILE_SECURITY.md D-K4).
@main
struct ThinClawWatchWidgetsBundle: WidgetBundle {
    var body: some Widget {
        StatusComplication()
    }
}

/// Agent phase icon + pending-approvals count, in the circular / corner /
/// inline complication families. Reads the mirrored App Group snapshots; the
/// timeline reloads when the bridge writes a new mirror (via `WidgetCenter`)
/// and on the scheduled cadence below. Resilient to a missing snapshot: renders
/// an "open watch app" affordance rather than an empty-but-live glance.
struct StatusComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(
            kind: "com.thinclaw.ios.watch.status",
            provider: ComplicationProvider()
        ) { entry in
            ComplicationView(entry: entry)
                .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("ThinClaw")
        .description("Agent status at a glance.")
        .supportedFamilies([.accessoryCircular, .accessoryCorner, .accessoryInline])
    }
}

struct ComplicationEntry: TimelineEntry {
    let date: Date
    /// The mirrored agent status, or `nil` when nothing has been mirrored yet.
    let phase: AgentStatusSnapshot.Phase?
    let pendingCount: Int
    /// True when the mirrored status is old enough to badge rather than imply
    /// live data.
    let isStale: Bool

    /// No snapshot mirrored yet → prompt the operator to open the watch app.
    var hasData: Bool { phase != nil || pendingCount > 0 }
    var hasPendingApprovals: Bool { pendingCount > 0 }
}

struct ComplicationProvider: TimelineProvider {
    /// The watch-local App Group that mirrors phone snapshots (matches the
    /// widget + watch entitlements).
    static let watchAppGroupID = "group.com.thinclaw.shared.watch"

    /// Modest scheduled cadence; the bridge triggers `WidgetCenter` reloads on
    /// a fresh mirror for timeliness, so this is only a backstop.
    private static let refreshInterval: TimeInterval = 30 * 60

    func placeholder(in context: Context) -> ComplicationEntry {
        ComplicationEntry(date: .now, phase: .idle, pendingCount: 0, isStale: false)
    }

    func getSnapshot(in context: Context, completion: @escaping (ComplicationEntry) -> Void) {
        completion(Self.currentEntry())
    }

    func getTimeline(
        in context: Context, completion: @escaping (Timeline<ComplicationEntry>) -> Void
    ) {
        let entry = Self.currentEntry()
        let next = Date.now.addingTimeInterval(Self.refreshInterval)
        completion(Timeline(entries: [entry], policy: .after(next)))
    }

    /// Read the mirrored snapshots defensively; any failure (missing container,
    /// missing file, corrupt, newer schema) degrades to a "no data" entry and
    /// never crashes the extension.
    private static func currentEntry() -> ComplicationEntry {
        let store = SnapshotStore(appGroupID: watchAppGroupID)
        let status = store.flatMap { try? $0.load(AgentStatusSnapshot.self) } ?? nil
        let approvals = store.flatMap { try? $0.load(PendingApprovalsSnapshot.self) } ?? nil
        let isStale =
            status.map {
                $0.isKnownStale
                    || Date.now.timeIntervalSince($0.generatedAt) > staleThreshold
            } ?? false
        return ComplicationEntry(
            date: .now,
            phase: status?.phase,
            pendingCount: approvals?.approvals.count ?? 0,
            isStale: isStale
        )
    }

    /// Watch-local staleness threshold (mirrors ThinClawWidgetKitShared's
    /// 30-minute `isStale`, which is not a watch-widget dependency).
    private static let staleThreshold: TimeInterval = 30 * 60
}

/// Renders per complication family. The corner/circular families are icon-led
/// (phase icon, or a shield with the pending count); the inline family is a
/// short text line.
struct ComplicationView: View {
    @Environment(\.widgetFamily) private var family
    let entry: ComplicationEntry

    var body: some View {
        switch family {
        case .accessoryInline:
            Label(inlineText, systemImage: symbol)
        case .accessoryCorner:
            Image(systemName: symbol)
                .font(.title3)
                .widgetLabel(cornerLabel)
        default:  // .accessoryCircular
            circular
        }
    }

    // MARK: - Circular

    private var circular: some View {
        ZStack {
            AccessoryWidgetBackground()
            if entry.hasPendingApprovals {
                VStack(spacing: 0) {
                    Image(systemName: "checkmark.shield.fill")
                        .font(.caption)
                    Text("\(entry.pendingCount)")
                        .font(.system(.caption2, design: .rounded)).bold()
                }
                .foregroundStyle(.orange)
            } else {
                Image(systemName: symbol)
                    .font(.title3)
            }
        }
    }

    // MARK: - Text

    private var inlineText: String {
        if !entry.hasData { return "Open ThinClaw" }
        if entry.hasPendingApprovals {
            return "\(entry.pendingCount) pending"
        }
        return entry.phase.map(Self.phaseLabel) ?? "Idle"
    }

    private var cornerLabel: String {
        if entry.hasPendingApprovals {
            return "\(entry.pendingCount)"
        }
        return entry.phase.map(Self.phaseLabel) ?? "ThinClaw"
    }

    // MARK: - Symbol

    /// Pending approvals dominate the glance (they are the actionable state);
    /// otherwise the current phase icon, or a neutral "open app" glyph when no
    /// snapshot has been mirrored.
    private var symbol: String {
        if entry.hasPendingApprovals { return "checkmark.shield.fill" }
        guard let phase = entry.phase else { return "brain" }
        return Self.phaseIcon(phase)
    }

    static func phaseLabel(_ phase: AgentStatusSnapshot.Phase) -> String {
        switch phase {
        case .idle: "Idle"
        case .thinking: "Thinking"
        case .streaming: "Responding"
        case .runningTool: "Tool"
        case .waitingForApproval: "Approval"
        case .error: "Error"
        }
    }

    static func phaseIcon(_ phase: AgentStatusSnapshot.Phase) -> String {
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
