import SwiftUI

/// Connection / agent-state pill rendered in glass, used in the chat nav bar,
/// the status widget, and the watch root view.
public struct StatusPill: View {
    public enum Status: Sendable, Hashable {
        case connected
        case connecting
        case degraded
        case offline

        var label: LocalizedStringKey {
            switch self {
            case .connected: "Connected"
            case .connecting: "Connecting…"
            case .degraded: "Degraded"
            case .offline: "Offline"
            }
        }

        var tint: Color {
            switch self {
            case .connected: .green
            case .connecting: .yellow
            case .degraded: .orange
            case .offline: .red
            }
        }
    }

    private let status: Status
    private let detail: String?

    public init(_ status: Status, detail: String? = nil) {
        self.status = status
        self.detail = detail
    }

    public var body: some View {
        HStack(spacing: ThinClawSpacing.xs) {
            Circle()
                .fill(status.tint)
                .frame(width: 8, height: 8)
            Text(status.label)
                .font(ThinClawTypography.caption)
            if let detail {
                Text(detail)
                    .font(ThinClawTypography.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, ThinClawSpacing.md)
        .padding(.vertical, ThinClawSpacing.xs)
        .glassEffect(.regular, in: .capsule)
        .accessibilityElement(children: .combine)
    }
}
