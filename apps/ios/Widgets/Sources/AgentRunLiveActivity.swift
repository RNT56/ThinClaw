import ActivityKit
import SwiftUI
import ThinClawSnapshotKit
import WidgetKit

/// Live Activity for an agent run: Dynamic Island + lock screen progress.
/// ContentState carries a status enum + progress only (no prompt text, no
/// tool arguments — docs/MOBILE_SECURITY.md D-N2). Push-updated via APNs
/// `liveactivity` type; local SSE updates win while foregrounded via the
/// monotonic `revision`.
struct AgentRunLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentRunAttributes.self) { context in
            // Lock screen / banner
            HStack {
                Image(systemName: context.state.phase.systemImage)
                VStack(alignment: .leading) {
                    Text(context.attributes.threadTitle)
                        .font(.headline)
                        .lineLimit(1)
                    Text(context.state.phase.label)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                if let progress = context.state.progress {
                    ProgressView(value: Double(progress), total: 100)
                        .progressViewStyle(.circular)
                }
            }
            .padding()
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Image(systemName: context.state.phase.systemImage)
                }
                DynamicIslandExpandedRegion(.center) {
                    Text(context.state.phase.label)
                        .font(.caption)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    if let progress = context.state.progress {
                        Text("\(progress)%").font(.caption2)
                    }
                }
            } compactLeading: {
                Image(systemName: "brain")
            } compactTrailing: {
                Image(systemName: context.state.phase.systemImage)
            } minimal: {
                Image(systemName: context.state.phase.systemImage)
            }
        }
    }
}

extension AgentRunAttributes.ContentState.RunPhase {
    var label: String {
        switch self {
        case .thinking: "Thinking…"
        case .runningTool: "Running a tool"
        case .awaitingApproval: "Waiting for approval"
        case .done: "Done"
        case .failed: "Failed"
        }
    }

    var systemImage: String {
        switch self {
        case .thinking: "brain"
        case .runningTool: "gearshape.2"
        case .awaitingApproval: "checkmark.shield"
        case .done: "checkmark.circle"
        case .failed: "exclamationmark.triangle"
        }
    }
}
