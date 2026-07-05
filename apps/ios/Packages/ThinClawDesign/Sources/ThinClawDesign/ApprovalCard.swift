import SwiftUI

/// Inline tool-approval prompt.
///
/// High-risk approvals (per the gateway-computed risk tier) must be gated by
/// biometrics by the caller before `onApprove` fires — this view only renders
/// the tier. Widgets and the watch never present this card for high risk;
/// they deep-link into the app instead (docs/MOBILE_SECURITY.md, D-K3).
public struct ApprovalCard: View {
    public enum RiskTier: Sendable, Hashable {
        case low
        case high
    }

    private let toolName: String
    private let requestDescription: String
    private let risk: RiskTier
    private let onApprove: () -> Void
    private let onDeny: () -> Void

    public init(
        toolName: String,
        requestDescription: String,
        risk: RiskTier,
        onApprove: @escaping () -> Void,
        onDeny: @escaping () -> Void
    ) {
        self.toolName = toolName
        self.requestDescription = requestDescription
        self.risk = risk
        self.onApprove = onApprove
        self.onDeny = onDeny
    }

    public var body: some View {
        GlassEffectContainer {
            VStack(alignment: .leading, spacing: ThinClawSpacing.md) {
                HStack(spacing: ThinClawSpacing.sm) {
                    Image(systemName: "wrench.and.screwdriver")
                    Text(toolName)
                        .font(ThinClawTypography.cardTitle)
                    Spacer()
                    if risk == .high {
                        Label("High risk", systemImage: "exclamationmark.shield")
                            .font(ThinClawTypography.caption)
                            .foregroundStyle(.orange)
                            .labelStyle(.titleAndIcon)
                    }
                }
                Text(requestDescription)
                    .font(ThinClawTypography.mono)
                    .lineLimit(6)
                    .foregroundStyle(.secondary)
                HStack(spacing: ThinClawSpacing.md) {
                    Button(role: .destructive, action: onDeny) {
                        Text("Deny")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.glass)

                    Button(action: onApprove) {
                        Text(risk == .high ? "Approve with Face ID" : "Approve")
                            .frame(maxWidth: .infinity)
                    }
                    .buttonStyle(.glassProminent)
                }
            }
            .padding(ThinClawSpacing.lg)
            .glassEffect(
                .regular,
                in: .rect(cornerRadius: ThinClawRadius.card)
            )
        }
        .accessibilityElement(children: .contain)
    }
}
