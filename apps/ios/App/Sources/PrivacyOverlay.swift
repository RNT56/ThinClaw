import SwiftUI
import ThinClawCore
import ThinClawDesign
import ThinClawSnapshotKit

/// Stable `UserDefaults` keys for privacy-related operator preferences shared
/// across the app. The FeatureSettings "Enhanced protection" toggle persists
/// to ``enhancedProtection``, which gates the heavier data-at-rest file
/// protection level (the app-switcher redaction overlay is always on,
/// independent of this toggle).
enum PrivacySettingsKey {
    static let enhancedProtection = DataProtectionPolicy.enhancedPreferenceKey
}

/// App-switcher / snapshot redaction (M5, docs/MOBILE_SECURITY.md
/// "Data at rest & logging hygiene": *optional app-switcher redaction overlay
/// on the client*).
///
/// iOS composites the multitasking-switcher snapshot while the scene is
/// `.inactive`, capturing whatever the live window shows — which would leak
/// transcript content. This modifier covers the window with a plain, opaque
/// glass panel whenever ``PrivacyRedactionPolicy`` says the current scene phase
/// should be redacted, and removes it on `.active`.
///
/// The *whether* decision lives in the pure ``PrivacyRedactionPolicy`` in
/// ThinClawCore (unit-tested on macOS); this view only maps SwiftUI's
/// `ScenePhase` onto that policy and renders the cover.
struct PrivacyOverlayModifier: ViewModifier {
    @Environment(\.scenePhase) private var scenePhase

    private var redacts: Bool {
        PrivacyRedactionPolicy.shouldRedact(for: PrivacyRedactionPolicy.Phase(scenePhase))
    }

    func body(content: Content) -> some View {
        content.overlay {
            if redacts {
                PrivacyCover()
                    // The snapshot is captured synchronously as the phase
                    // flips, so the cover must be present *without* a transition
                    // that would let a frame of content leak. No animation.
                    .transition(.identity)
            }
        }
    }
}

/// The opaque cover drawn over the window while redacting. Deliberately plain:
/// a full-bleed material with the app glyph, carrying no transcript content.
private struct PrivacyCover: View {
    var body: some View {
        ZStack {
            Rectangle()
                .fill(.background)
                .ignoresSafeArea()
            Image(systemName: "lock.shield")
                .font(.system(size: 44, weight: .regular))
                .foregroundStyle(.secondary)
        }
        // Hidden from VoiceOver: it is a privacy screen, not content.
        .accessibilityHidden(true)
    }
}

extension PrivacyRedactionPolicy.Phase {
    /// Map SwiftUI's `ScenePhase` onto the Foundation-only policy phase.
    /// Unknown future cases fall back to `.inactive` (redact) — fail safe.
    init(_ phase: ScenePhase) {
        switch phase {
        case .active: self = .active
        case .inactive: self = .inactive
        case .background: self = .background
        @unknown default: self = .inactive
        }
    }
}

extension View {
    /// Apply the always-on app-switcher redaction overlay (covers the window
    /// whenever the scene is not active, so the switcher snapshot never leaks
    /// transcript content).
    func privacyOverlay() -> some View {
        modifier(PrivacyOverlayModifier())
    }
}
