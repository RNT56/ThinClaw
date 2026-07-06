import Foundation

/// App-switcher / snapshot redaction policy (M5, docs/MOBILE_SECURITY.md
/// "Data at rest & logging hygiene": *optional app-switcher redaction overlay
/// on the client*).
///
/// When iOS snapshots the app for the multitasking switcher it captures the
/// live window, which can leak transcript content. The client answers this
/// with a plain overlay that covers the window whenever the scene is not
/// active. This type is the single, pure decision point for *whether* that
/// overlay should be shown, so the rule is unit-testable on a Mac host without
/// SwiftUI: the app layer maps SwiftUI's `ScenePhase` to ``Phase`` and reads
/// the operator's "Enhanced protection" setting, then asks ``shouldRedact``.
public enum PrivacyRedactionPolicy {
    /// A Foundation-only mirror of SwiftUI's `ScenePhase`, so the policy stays
    /// in ThinClawCore (no UI import) and remains macOS-testable. The app maps
    /// `ScenePhase.active`/`.inactive`/`.background` onto these cases.
    public enum Phase: Sendable, Hashable, CaseIterable {
        /// Foreground and interactive — never redacted.
        case active
        /// Transitioning (app switcher is being composited, incoming call
        /// banner, control center pull-down). This is the phase the switcher
        /// snapshot is taken in, so it MUST be covered.
        case inactive
        /// Fully backgrounded.
        case background
    }

    /// Whether the redaction overlay should cover the window for `phase`.
    ///
    /// Applied for `.inactive` and `.background` and removed for `.active`, so
    /// the app-switcher snapshot (taken while `.inactive`) never shows
    /// transcript content. This is **unconditional** — a cheap, always-on
    /// privacy measure for a security-first app, independent of the heavier
    /// "Enhanced protection" data-at-rest toggle (which gates file protection,
    /// not the switcher overlay).
    public static func shouldRedact(for phase: Phase) -> Bool {
        switch phase {
        case .active:
            return false
        case .inactive, .background:
            return true
        }
    }
}
