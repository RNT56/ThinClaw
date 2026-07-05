import SwiftUI

/// Spacing, radius, and typography tokens for the ThinClaw surfaces.
///
/// Components take their materials from the system Liquid Glass APIs
/// (`glassEffect`, `GlassEffectContainer`) rather than custom fills, so the
/// tokens here are limited to layout and type — color and depth come from
/// the OS.
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
