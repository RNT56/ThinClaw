import Foundation
import SwiftUI

/// Inline tool-approval prompt.
///
/// High-risk approvals (per the gateway-computed risk tier) must be gated by
/// biometrics by the caller before `onApprove` fires — this view only renders
/// the tier. Widgets and the watch never present this card for high risk;
/// they deep-link into the app instead (docs/MOBILE_SECURITY.md, D-K3).
public struct ApprovalCard: View {
    /// Presentation mirror of `ThinClawCore.RiskTier`. ThinClawDesign is kept
    /// dependency-free (widgets and the watch import it without dragging in
    /// Core/Transport), so the canonical tier lives in Core and the app layer
    /// maps `RiskTier` -> `ApprovalCard.RiskTier` at the call site. Keep the
    /// two cases in lockstep.
    public enum RiskTier: Sendable, Hashable {
        case low
        case high
    }

    private let toolName: String
    private let requestDescription: String
    private let parameters: String
    private let risk: RiskTier
    private let onApprove: () -> Void
    private let onDeny: () -> Void

    /// Under Increase Contrast, the high-risk badge switches from a tinted
    /// orange to a filled capsule so it stays legible over the glass material.
    @Environment(\.colorSchemeContrast) private var contrast
    @State private var showsParameters = false

    public init(
        toolName: String,
        requestDescription: String,
        parameters: String = "",
        risk: RiskTier,
        onApprove: @escaping () -> Void,
        onDeny: @escaping () -> Void
    ) {
        self.toolName = toolName
        self.requestDescription = requestDescription
        self.parameters = parameters
        self.risk = risk
        self.onApprove = onApprove
        self.onDeny = onDeny
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: ThinClawSpacing.md) {
            HStack(spacing: ThinClawSpacing.sm) {
                Image(systemName: "wrench.and.screwdriver")
                    .accessibilityHidden(true)
                Text(toolName)
                    .font(ThinClawTypography.cardTitle)
                Spacer()
                if risk == .high {
                    highRiskBadge
                }
            }
            // The header reads as one element: "<tool> approval, <risk> risk".
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(
                Text("\(toolName) approval, \(risk == .high ? "high" : "low") risk")
            )
            .accessibilityAddTraits(.isHeader)
            Text(requestDescription)
                .font(ThinClawTypography.mono)
                .foregroundStyle(.secondary)
            if !parameters.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                #if os(watchOS)
                    Text(formattedParameters)
                        .font(ThinClawTypography.caption)
                        .lineLimit(4)
                        .foregroundStyle(.secondary)
                #else
                    DisclosureGroup("Parameters", isExpanded: $showsParameters) {
                        ScrollView(.horizontal) {
                            Text(formattedParameters)
                                .font(ThinClawTypography.mono)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.top, ThinClawSpacing.xs)
                        }
                    }
                    .font(ThinClawTypography.caption)
                    .accessibilityHint("Shows the arguments this tool will receive")
                #endif
            }
            HStack(spacing: ThinClawSpacing.md) {
                Button(role: .destructive, action: onDeny) {
                    Text("Deny")
                        .frame(maxWidth: .infinity)
                }
                .thinClawButtonStyle()

                Button(action: onApprove) {
                    Text(risk == .high ? "Approve with Face ID" : "Approve")
                        .frame(maxWidth: .infinity)
                }
                .thinClawButtonStyle(prominent: true)
            }
        }
        .padding(ThinClawSpacing.lg)
        .thinClawSurface()
        .accessibilityElement(children: .contain)
    }

    private var formattedParameters: String {
        guard let data = parameters.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data),
            JSONSerialization.isValidJSONObject(object),
            let pretty = try? JSONSerialization.data(
                withJSONObject: object, options: [.prettyPrinted, .sortedKeys]),
            let result = String(data: pretty, encoding: .utf8)
        else { return parameters }
        return result
    }

    /// High-risk badge. In the default contrast it is a tinted orange label; in
    /// Increase Contrast it becomes a filled orange capsule with black glyph so
    /// the warning does not wash out against the translucent glass.
    @ViewBuilder
    private var highRiskBadge: some View {
        if contrast == .increased {
            Label("High risk", systemImage: "exclamationmark.shield")
                .font(ThinClawTypography.caption)
                .labelStyle(.titleAndIcon)
                .foregroundStyle(.black)
                .padding(.horizontal, ThinClawSpacing.sm)
                .padding(.vertical, ThinClawSpacing.xs)
                .background(.orange, in: .capsule)
        } else {
            Label("High risk", systemImage: "exclamationmark.shield")
                .font(ThinClawTypography.caption)
                .foregroundStyle(.orange)
                .labelStyle(.titleAndIcon)
        }
    }
}
