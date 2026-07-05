import SwiftUI

/// Agent prose that is still arriving. Renders the partial body with a
/// trailing shimmer caret; content transitions are driven by the coalesced
/// ~10 Hz updates from `StreamChunkCoalescer`, not per-chunk.
public struct StreamingText: View {
    private let text: String
    private let isStreaming: Bool

    public init(_ text: String, isStreaming: Bool) {
        self.text = text
        self.isStreaming = isStreaming
    }

    public var body: some View {
        Text(text + (isStreaming ? " ●" : ""))
            .font(ThinClawTypography.body)
            .contentTransition(.interpolate)
            .animation(.easeOut(duration: 0.12), value: text)
            .accessibilityLabel(
                isStreaming ? Text("Agent is responding: \(text)") : Text(text)
            )
    }
}
