import Foundation

/// VoiceOver descriptors for a chat ``TimelineItem`` (M5 accessibility).
///
/// The chat timeline is a flat list of heterogeneous rows; a sighted reader
/// distinguishes them by shape and color, but VoiceOver needs each row to
/// announce *what kind of thing it is* and *who it came from*. This type is the
/// single, pure mapping from a `TimelineItem.Kind` to the strings the view
/// applies via `.accessibilityLabel` / `.accessibilityValue` / `.accessibilityHint`.
///
/// Keeping it in ThinClawCore (Foundation only) makes the wording unit-testable
/// on a Mac host — the FeatureChat row view is a thin adapter that reads these
/// fields and never composes VoiceOver prose itself.
public struct TimelineAccessibility: Sendable, Hashable {
    /// The primary spoken label (identity + kind), e.g. `"Agent said Hello"`.
    public var label: String
    /// A separately-announced value, used for content that *updates in place*.
    ///
    /// A streaming agent reply keeps a stable `label` ("Agent is responding")
    /// and moves the changing prose into `value`, so VoiceOver announces the
    /// growth politely as an `accessibilityValue` change instead of re-reading
    /// the whole row on every ~10 Hz coalesced update.
    public var value: String?
    /// An optional hint describing the row's available action, e.g. the retry
    /// affordance on a failure row. `nil` when the row is not actionable.
    public var hint: String?
    /// Whether the row should carry the header trait (none of the chat kinds
    /// do today, but the field keeps the descriptor self-contained for future
    /// section rows).
    public var isHeader: Bool

    public init(label: String, value: String? = nil, hint: String? = nil, isHeader: Bool = false) {
        self.label = label
        self.value = value
        self.hint = hint
        self.isHeader = isHeader
    }
}

extension TimelineItem {
    /// The VoiceOver descriptor for this row.
    public var accessibility: TimelineAccessibility { kind.accessibility }
}

extension TimelineItem.Kind {
    /// Pure mapping from a timeline-row kind to its VoiceOver descriptor.
    public var accessibility: TimelineAccessibility {
        switch self {
        case .userMessage(let text):
            return TimelineAccessibility(label: "You said \(text)")

        case .agentMessage(let text):
            return TimelineAccessibility(label: "Agent said \(text)")

        case .streamingAgentMessage(let text):
            // Stable label + changing value: VoiceOver announces the growing
            // reply as a value update (polite) rather than relabeling the row.
            return TimelineAccessibility(label: "Agent is responding", value: text)

        case .statusNote(let text):
            return TimelineAccessibility(label: "Status. \(text)")

        case .toolCall(let name, let status):
            let phrase: String
            switch status {
            case .running: phrase = "running"
            case .succeeded: phrase = "succeeded"
            case .failed: phrase = "failed"
            }
            return TimelineAccessibility(label: "Tool \(name) \(phrase)")

        case .approval(let request):
            let risk = request.risk == .high ? "high" : "low"
            return TimelineAccessibility(
                label: "\(request.toolName) approval, \(risk) risk",
                hint: "Opens the approval to review and decide")

        case .authPrompt(let prompt):
            return TimelineAccessibility(
                label: "\(prompt.extensionName) needs authorization",
                hint: prompt.authURL != nil ? "Opens the authorization page" : nil)

        case .credentialPrompt(let prompt):
            // Per D-T4 the phone cannot answer this; the label says so.
            return TimelineAccessibility(
                label: "\(prompt.provider) needs a credential. Handle on desktop")

        case .failure(let message):
            return TimelineAccessibility(
                label: "Failed. \(message)",
                hint: "Double tap to retry")
        }
    }
}
