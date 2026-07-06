import SwiftUI

/// Agent prose that is still arriving. Renders the partial body with a
/// trailing shimmer caret; content transitions are driven by the coalesced
/// ~10 Hz updates from `StreamChunkCoalescer`, not per-chunk.
public struct StreamingText: View {
    private let text: String
    private let isStreaming: Bool

    /// Honor the system Reduce Motion setting: the animated content transition
    /// and the pulsing caret are motion, so both are suppressed when the user
    /// asks for reduced motion. The prose still updates — just without the
    /// interpolation/caret.
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    public init(_ text: String, isStreaming: Bool) {
        self.text = text
        self.isStreaming = isStreaming
    }

    private var showsCaret: Bool { isStreaming && !reduceMotion }

    public var body: some View {
        Text(text + (showsCaret ? " ●" : ""))
            .font(ThinClawTypography.body)
            .contentTransition(reduceMotion ? .identity : .interpolate)
            .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: text)
            // Keep a stable label and put the changing prose in the value so
            // VoiceOver announces streaming growth politely (as a value change)
            // rather than re-reading the whole reply on every coalesced update.
            .accessibilityLabel(isStreaming ? Text("Agent is responding") : Text("Agent said"))
            .accessibilityValue(Text(text))
    }
}
