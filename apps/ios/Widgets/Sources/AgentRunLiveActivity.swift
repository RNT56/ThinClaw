import ActivityKit
import SwiftUI
import ThinClawSnapshotKit
import WidgetKit

/// Live Activity for an agent run: Dynamic Island + lock screen progress.
///
/// ContentState carries a status enum, an optional tool name, and progress only
/// — never prompt text or tool arguments (docs/MOBILE_SECURITY.md D-N2). The
/// tool name is populated only from *local* SSE updates while the app is
/// foregrounded; the gateway's push `content-state` never includes it, so
/// nothing sensitive transits APNs. Local SSE updates win over a late push via
/// the monotonic `revision` (a push carrying an older revision is superseded by
/// the higher-revision local update the app already applied).
struct AgentRunLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentRunAttributes.self) { context in
            LockScreenView(
                title: context.attributes.threadTitle,
                state: context.state)
        } dynamicIsland: { context in
            let state = context.state
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Label {
                        Text(state.phase.label)
                            .font(.caption)
                            .lineLimit(1)
                    } icon: {
                        Image(systemName: state.phase.systemImage)
                            .foregroundStyle(state.phase.tint)
                    }
                }
                DynamicIslandExpandedRegion(.trailing) {
                    if let progress = state.progress {
                        Text("\(progress)%")
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.secondary)
                    }
                }
                DynamicIslandExpandedRegion(.bottom) {
                    if state.phase == .runningTool, let toolName = state.toolName {
                        Text(toolName)
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    } else if let progress = state.progress {
                        ProgressView(value: Double(progress), total: 100)
                            .tint(state.phase.tint)
                    }
                }
            } compactLeading: {
                Image(systemName: state.phase.systemImage)
                    .foregroundStyle(state.phase.tint)
            } compactTrailing: {
                if let progress = state.progress {
                    Text("\(progress)%")
                        .font(.caption2.monospacedDigit())
                }
            } minimal: {
                Image(systemName: state.phase.systemImage)
                    .foregroundStyle(state.phase.tint)
            }
        }
    }
}

/// Lock-screen / banner presentation of a run: icon, thread title, phase label
/// (specialized with the tool name while a tool runs), and a circular progress
/// indicator when the run reports progress.
private struct LockScreenView: View {
    let title: String
    let state: AgentRunAttributes.ContentState

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: state.phase.systemImage)
                .font(.title3)
                .foregroundStyle(state.phase.tint)
                .frame(width: 28)
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.headline)
                    .lineLimit(1)
                Text(subtitle)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            if let progress = state.progress {
                ProgressView(value: Double(progress), total: 100)
                    .progressViewStyle(.circular)
                    .tint(state.phase.tint)
            }
        }
        .padding()
    }

    /// The phase label, specialized with the tool name while a tool runs.
    private var subtitle: String {
        if state.phase == .runningTool, let toolName = state.toolName {
            return toolName
        }
        return state.phase.label
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

    /// Accent tint for the phase icon: neutral while working, prominent when the
    /// run needs the operator, semantic on terminal states.
    var tint: Color {
        switch self {
        case .thinking, .runningTool: .accentColor
        case .awaitingApproval: .orange
        case .done: .green
        case .failed: .red
        }
    }
}
