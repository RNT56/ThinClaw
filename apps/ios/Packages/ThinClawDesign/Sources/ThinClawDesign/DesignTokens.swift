import SwiftUI

/// Spacing, radius, and typography tokens for the ThinClaw surfaces.
///
/// Components use native Liquid Glass on iOS/watchOS 26 and a semantic
/// material fallback on earlier supported releases.
public enum ThinClawSpacing {
    /// Tight intra-row spacing (icon ↔ label).
    public static let xs: CGFloat = 4
    /// Default intra-component spacing.
    public static let sm: CGFloat = 8
    /// Default inter-component spacing.
    public static let md: CGFloat = 12
    /// Section spacing.
    public static let lg: CGFloat = 20
    /// Screen-edge insets.
    public static let xl: CGFloat = 28
}

public enum ThinClawRadius {
    /// Chips, pills, small controls.
    public static let control: CGFloat = 12
    /// Cards (approvals, tool activity).
    public static let card: CGFloat = 20
}

public enum ThinClawTypography {
    /// Streaming/final agent prose.
    public static let body: Font = .body
    /// Status and tool-activity captions.
    public static let caption: Font = .caption
    /// Card titles (tool name, approval title).
    public static let cardTitle: Font = .headline
    /// Monospaced snippets (tool parameters, command previews).
    public static let mono: Font = .system(.callout, design: .monospaced)
}

public enum ThinClawColor {
    /// Supporting copy with stronger contrast than the platform's decorative
    /// `.secondary` style. This remains semantic in light/dark appearances
    /// while meeting body/caption readability needs on plain and glass surfaces.
    public static let secondaryText = Color.primary.opacity(0.72)
}

extension View {
    /// Availability-safe card surface shared by the app and watch.
    @ViewBuilder
    public func thinClawSurface(cornerRadius: CGFloat = ThinClawRadius.card) -> some View {
        if #available(iOS 26.0, watchOS 26.0, *) {
            glassEffect(.regular, in: .rect(cornerRadius: cornerRadius))
        } else {
            background(.regularMaterial, in: RoundedRectangle(cornerRadius: cornerRadius))
                .overlay {
                    RoundedRectangle(cornerRadius: cornerRadius)
                        .stroke(.primary.opacity(0.12), lineWidth: 0.5)
                }
        }
    }

    /// Availability-safe capsule surface used for compact state labels.
    @ViewBuilder
    public func thinClawCapsuleSurface() -> some View {
        if #available(iOS 26.0, watchOS 26.0, *) {
            glassEffect(.regular, in: .capsule)
        } else {
            background(.regularMaterial, in: Capsule())
                .overlay {
                    Capsule().stroke(.primary.opacity(0.12), lineWidth: 0.5)
                }
        }
    }

    /// Availability-safe native button styling.
    @ViewBuilder
    public func thinClawButtonStyle(prominent: Bool = false) -> some View {
        if #available(iOS 26.0, watchOS 26.0, *) {
            if prominent {
                buttonStyle(.glassProminent)
            } else {
                buttonStyle(.glass)
            }
        } else if prominent {
            buttonStyle(.borderedProminent)
        } else {
            buttonStyle(.bordered)
        }
    }
}
